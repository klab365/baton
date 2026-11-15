use super::pi::parse_model_list;
use async_trait::async_trait;
use baton_core::{domain::runtime::RuntimeModel, ports::pi_models::PiModelCatalog};
use std::{
    ffi::{OsStr, OsString},
    io::Read,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct PiCliModelCatalog {
    command: OsString,
}

impl Default for PiCliModelCatalog {
    fn default() -> Self {
        Self {
            command: super::PI_EXECUTABLE.into(),
        }
    }
}

impl PiCliModelCatalog {
    pub fn with_command(command: impl Into<OsString>) -> Self {
        Self {
            command: command.into(),
        }
    }
}

#[async_trait]
impl PiModelCatalog for PiCliModelCatalog {
    async fn list_models(&self, provider: &str) -> Result<Vec<RuntimeModel>, String> {
        let command = self.command.clone();
        let provider = provider.to_owned();
        tokio::task::spawn_blocking(move || list_models(&command, &provider))
            .await
            .map_err(|_| "Pi model listing task failed".to_owned())?
    }
}

fn list_models(command: &OsStr, provider: &str) -> Result<Vec<RuntimeModel>, String> {
    if provider.is_empty()
        || !provider
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err("Pi provider ID is invalid".into());
    }
    let mut process = Command::new(command);
    process
        .args([
            "--offline",
            "--no-extensions",
            "--no-mcp",
            "--list-models",
            provider,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        process.process_group(0);
    }
    let mut child = process
        .spawn()
        .map_err(|_| "Pi CLI is unavailable; install Pi or check its runtime PATH".to_owned())?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("Pi model output is unavailable")?;
    let (sender, receiver) = mpsc::channel();
    // shortcut: descendants escaping the process group can retain this reader; use an OS subreaper/job object if Pi starts untrusted detached children.
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0; 8192];
        let mut overflow = false;
        let mut read_failed = false;
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    let remaining = OUTPUT_LIMIT.saturating_sub(output.len());
                    let keep = count.min(remaining);
                    output.extend_from_slice(&buffer[..keep]);
                    overflow |= keep < count;
                }
                Err(_) => {
                    read_failed = true;
                    break;
                }
            }
        }
        let _ = sender.send((output, overflow, read_failed));
    });

    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                kill_process_tree(&mut child);
                let _ = child.wait();
                return Err("Pi model listing timed out or could not complete".into());
            }
        }
    };
    kill_process_tree(&mut child);
    if !status.success() {
        return Err(
            "Pi could not list models; verify the installed CLI supports --list-models".into(),
        );
    }
    let (output, overflow, read_failed) = receiver
        .recv_timeout(Duration::from_millis(500))
        .map_err(|_| "Pi model listing output was unavailable".to_owned())?;
    if read_failed {
        return Err("Pi model listing output could not be read".into());
    }
    if overflow {
        return Err("Pi model list exceeds the 1 MiB limit".into());
    }
    let output =
        std::str::from_utf8(&output).map_err(|_| "Pi model list is not valid UTF-8".to_owned())?;
    parse_model_list(output, provider)
}

fn kill_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn fake_pi(script: &str) -> (tempfile::TempDir, OsString) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fake-pi");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (directory, path.into_os_string())
    }

    #[cfg(unix)]
    #[test]
    fn lists_provider_models_offline_and_does_not_return_stderr() {
        let (_directory, command) = fake_pi(
            "printf 'provider model context\\nanthropic claude-4 100K\\nopenai gpt-4.1 1M\\n'; echo secret-shaped >&2",
        );
        let models = list_models(&command, "anthropic").unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model, "claude-4");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_oversized_output_without_returning_it() {
        let (_directory, command) = fake_pi("yes x | head -c 1048577");
        let error = list_models(&command, "anthropic").unwrap_err();
        assert!(error.contains("1 MiB limit"));
        assert!(!error.contains("x x x"));
    }

    #[cfg(unix)]
    #[test]
    fn sanitizes_nonzero_exit_and_missing_executable_errors() {
        let (_directory, command) = fake_pi("echo private-token >&2; exit 9");
        let error = list_models(&command, "anthropic").unwrap_err();
        assert!(error.contains("could not list models"));
        assert!(!error.contains("private-token"));
        let error =
            list_models(OsStr::new("missing-pi-command-issue201"), "anthropic").unwrap_err();
        assert!(error.contains("Pi CLI is unavailable"));
    }

    #[cfg(unix)]
    #[test]
    fn closes_pipes_from_descendants_when_pi_exits() {
        let (_directory, command) =
            fake_pi("sleep 30 & printf 'provider model\\nanthropic model-a\\n'");
        let started = Instant::now();
        assert_eq!(list_models(&command, "anthropic").unwrap().len(), 1);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn times_out_and_kills_a_nonresponsive_pi_cli() {
        let (_directory, command) = fake_pi("sleep 10");
        let started = Instant::now();
        let error = list_models(&command, "anthropic").unwrap_err();
        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(7));
    }
}
