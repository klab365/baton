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
        let remaining = OUTPUT_LIMIT.saturating_sub(output.len());
        output.extend_from_slice(&buffer[..count.min(remaining)]);
    }
}

pub struct ShellWorkChecks;
impl WorkCheckRunner for ShellWorkChecks {
    fn run(&self, workspace: &Path, command: &str) -> Result<CheckResult, String> {
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
        let deadline = Instant::now() + Duration::from_secs(300);
        let (status, timed_out) = loop {
            match child.try_wait() {
                Ok(Some(status)) => break (status, false),
                Ok(None) if Instant::now() < deadline => {
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
        bytes.extend_from_slice(
            &errors[..errors.len().min(OUTPUT_LIMIT.saturating_sub(bytes.len()))],
        );
        let mut output = String::from_utf8_lossy(&bytes).into_owned();
        if timed_out {
            output.push_str("\nCheck timed out after 300 seconds");
        }
        Ok(CheckResult {
            command: command.into(),
            exit_code: if timed_out { None } else { status.code() },
            output,
        })
    }
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
    }
}
