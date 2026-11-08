//! Process gates for external workers. Commands originate in explicit run configuration.
use baton_core::{domain::work::CheckResult, ports::work::WorkCheckRunner};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 16384;

fn capture(mut input: impl Read) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            return Ok(output);
        }
        let remaining = (OUTPUT_LIMIT / 2).saturating_sub(output.len());
        output.extend_from_slice(&buffer[..count.min(remaining)]);
    }
}

pub struct ShellWorkChecks;
impl WorkCheckRunner for ShellWorkChecks {
    fn run(&self, workspace: &Path, command: &str) -> Result<CheckResult, String> {
        run_shell(workspace, command, 300, &|| false)
    }
}

pub fn run_shell(
    workspace: &Path,
    command: &str,
    timeout_seconds: u32,
    cancelled: &dyn Fn() -> bool,
) -> Result<CheckResult, String> {
    if cancelled() {
        return Err("Run cancelled".into());
    }
    let mut process = Command::new("sh");
    process
        .args(["-c", command])
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        process.process_group(0);
    }
    let mut child = process.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("Missing check stdout")?;
    let stderr = child.stderr.take().ok_or("Missing check stderr")?;
    let stdout = std::thread::spawn(move || capture(stdout));
    let stderr = std::thread::spawn(move || capture(stderr));
    let deadline = Instant::now() + Duration::from_secs(u64::from(timeout_seconds));
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status, false),
            Ok(None) if Instant::now() < deadline && !cancelled() => {
                std::thread::sleep(Duration::from_millis(20))
            }
            result => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let status = child.wait().map_err(|e| e.to_string())?;
                if let Err(error) = result {
                    return Err(error.to_string());
                }
                break (status, true);
            }
        }
    };
    // Descendants must not keep the output pipes open after the check exits.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let mut bytes = stdout
        .join()
        .map_err(|_| "Check stdout reader panicked")?
        .map_err(|e| e.to_string())?;
    let errors = stderr
        .join()
        .map_err(|_| "Check stderr reader panicked")?
        .map_err(|e| e.to_string())?;
    bytes.extend_from_slice(&errors[..errors.len().min(OUTPUT_LIMIT.saturating_sub(bytes.len()))]);
    let mut output = String::from_utf8_lossy(&bytes).into_owned();
    if timed_out {
        output.push_str(&format!(
            "\nCheck timed out after {timeout_seconds} seconds"
        ));
    }
    Ok(CheckResult {
        command: command.into(),
        exit_code: if timed_out { None } else { status.code() },
        output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discards_output_beyond_the_limit_without_blocking_the_check() {
        let result = ShellWorkChecks
            .run(
                Path::new("."),
                "yes x | head -c 1000000; yes y | head -c 1000000 >&2",
            )
            .unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.output.len(), OUTPUT_LIMIT);
        assert!(result.output.contains('y'));
    }

    #[test]
    fn reports_exit_status_and_workspace() {
        let result = run_shell(
            Path::new("."),
            "pwd; echo failure >&2; exit 7",
            300,
            &|| false,
        )
        .unwrap();
        assert_eq!(result.exit_code, Some(7));
        assert!(result.output.contains("failure"));
    }

    #[test]
    fn respects_configured_timeout() {
        let result = run_shell(Path::new("."), "sleep 30", 1, &|| false).unwrap();
        assert_eq!(result.exit_code, None);
        assert!(result.output.contains("timed out after 1 seconds"));
    }

    #[test]
    fn cancels_running_process() {
        let start = Instant::now();
        let result = run_shell(Path::new("."), "sleep 30", 300, &|| {
            start.elapsed() > Duration::from_millis(50)
        })
        .unwrap();
        assert_eq!(result.exit_code, None);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
