//! Process gates for external workers. Commands originate in explicit run configuration.
use baton_core::{domain::work::CheckResult, ports::work::WorkCheckRunner};
use std::{
    fs::{OpenOptions, create_dir_all},
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const OUTPUT_LIMIT: usize = 16384;

fn capture(
    mut input: impl Read,
    stream: &'static str,
    log: Arc<Mutex<std::fs::File>>,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::new();
    let mut truncated = false;
    let mut buffer = [0; 4096];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            return Ok((output, truncated));
        }
        let mut log = log
            .lock()
            .map_err(|_| std::io::Error::other("Shell log lock poisoned"))?;
        log.write_all(format!("[{stream}] ").as_bytes())?;
        log.write_all(&buffer[..count])?;
        let remaining = (OUTPUT_LIMIT / 2).saturating_sub(output.len());
        truncated |= count > remaining;
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
    run_shell_inner(workspace, command, timeout_seconds, cancelled, false)
}

pub fn run_pipeline_shell(
    workspace: &Path,
    command: &str,
    timeout_seconds: u32,
    cancelled: &dyn Fn() -> bool,
) -> Result<CheckResult, String> {
    run_shell_inner(workspace, command, timeout_seconds, cancelled, true)
}

fn run_shell_inner(
    workspace: &Path,
    command: &str,
    timeout_seconds: u32,
    cancelled: &dyn Fn() -> bool,
    file_handoff: bool,
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
    let log_dir = workspace.join(".baton/work");
    create_dir_all(&log_dir).map_err(|error| format!("Create shell log directory: {error}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let log_path = log_dir.join(format!("shell-{}-{timestamp}.log", child.id()));
    let log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&log_path)
        .map_err(|error| format!("Create shell log: {error}"))?;
    let log = Arc::new(Mutex::new(log));
    let stdout = child.stdout.take().ok_or("Missing check stdout")?;
    let stderr = child.stderr.take().ok_or("Missing check stderr")?;
    let stdout_log = Arc::clone(&log);
    let stderr_log = Arc::clone(&log);
    let stdout = std::thread::spawn(move || capture(stdout, "stdout", stdout_log));
    let stderr = std::thread::spawn(move || capture(stderr, "stderr", stderr_log));
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
    let (mut bytes, stdout_truncated) = stdout
        .join()
        .map_err(|_| "Check stdout reader panicked")?
        .map_err(|e| e.to_string())?;
    let (errors, stderr_truncated) = stderr
        .join()
        .map_err(|_| "Check stderr reader panicked")?
        .map_err(|e| e.to_string())?;
    bytes.extend_from_slice(&errors[..errors.len().min(OUTPUT_LIMIT.saturating_sub(bytes.len()))]);
    let mut output = if file_handoff {
        format!("Full shell output saved to: {}", log_path.display())
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    if timed_out {
        output.push_str(&format!(
            "\nCheck timed out after {timeout_seconds} seconds"
        ));
    }
    if !file_handoff && (stdout_truncated || stderr_truncated) {
        output.push_str(&format!(
            "\nOutput truncated; full log: {}",
            log_path.display()
        ));
    }
    drop(log);
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
    fn pipeline_shell_always_hands_off_the_full_output_log_path() {
        let workspace = tempfile::tempdir().unwrap();
        let result = run_pipeline_shell(
            workspace.path(),
            "printf 'verify output\\n'; exit 7",
            10,
            &|| false,
        )
        .unwrap();
        assert_eq!(result.exit_code, Some(7));
        assert!(result.output.len() < OUTPUT_LIMIT);
        let path = result
            .output
            .strip_prefix("Full shell output saved to: ")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "[stdout] verify output\n"
        );
    }

    #[test]
    fn logs_output_beyond_the_limit_without_blocking_the_check() {
        let workspace = tempfile::tempdir().unwrap();
        let result = ShellWorkChecks
            .run(
                workspace.path(),
                "yes x | head -c 1000000; yes y | head -c 1000000 >&2",
            )
            .unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert!(result.output.len() > OUTPUT_LIMIT);
        assert!(result.output.contains('y'));
        assert!(result.output.contains("Output truncated; full log:"));
    }

    #[test]
    fn reports_exit_status_and_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let result = run_shell(
            workspace.path(),
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
        let workspace = tempfile::tempdir().unwrap();
        let result = run_shell(workspace.path(), "sleep 30", 1, &|| false).unwrap();
        assert_eq!(result.exit_code, None);
        assert!(result.output.contains("timed out after 1 seconds"));
    }

    #[test]
    fn cancels_running_process() {
        let workspace = tempfile::tempdir().unwrap();
        let start = Instant::now();
        let result = run_shell(workspace.path(), "sleep 30", 300, &|| {
            start.elapsed() > Duration::from_millis(50)
        })
        .unwrap();
        assert_eq!(result.exit_code, None);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
