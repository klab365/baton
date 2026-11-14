use super::ConfiguredRuntime;
use baton_core::{
    domain::{
        pipeline::RunTraceEntry,
        project::{Agent, Outcome},
    },
    ports::runtime::AgentDecision,
};
use std::{
    io::{Read, Write},
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};

fn read_capped(mut reader: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut truncated = false;
    let mut buffer = [0; 8192];
    while let Ok(read) = reader.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let keep = (limit - output.len()).min(read);
        output.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }
    (output, truncated)
}

// shortcut: kills the initial process group only; use an OS job object/subreaper if Pi tools may detach descendants.
fn kill_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = child.kill();
}

fn collect_pi_io(
    child: &mut std::process::Child,
    stdout: mpsc::Receiver<(Vec<u8>, bool)>,
    stderr: mpsc::Receiver<(Vec<u8>, bool)>,
    prompt: mpsc::Receiver<std::io::Result<()>>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Vec<u8>, bool, Option<String>), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let (mut stdout_result, mut stderr_result, mut prompt_result) = (None, None, None);
    while stdout_result.is_none() || stderr_result.is_none() || prompt_result.is_none() {
        if cancelled() {
            kill_process_tree(child);
            return Err("Agent execution cancelled".into());
        }
        if Instant::now() >= deadline {
            kill_process_tree(child);
            return Err("Pi CLI descendants kept its output pipes open".into());
        }
        if stdout_result.is_none() {
            stdout_result = stdout.try_recv().ok();
        }
        if stderr_result.is_none() {
            stderr_result = stderr.try_recv().ok();
        }
        if prompt_result.is_none() {
            prompt_result = prompt.try_recv().ok();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (output, truncated) = stdout_result.unwrap();
    let prompt_error = prompt_result
        .and_then(Result::err)
        .map(|error| format!("Could not send Pi prompt: {error}"));
    Ok((output, truncated, prompt_error))
}

pub(super) fn execute(
    runtime: &mut ConfiguredRuntime,
    directory: &Path,
    agent: &Agent,
    input: &str,
    allowed_outcomes: &[Outcome],
    cancelled: &dyn Fn() -> bool,
    observer: &mut dyn FnMut(RunTraceEntry) -> Result<(), String>,
) -> Result<String, String> {
    if !allowed_outcomes.is_empty() && !agent.allows_tool("report_outcome") {
        return Err("Pi branching steps require the report_outcome tool".into());
    }
    let supported = [
        "read",
        "list",
        "search",
        "shell",
        "write",
        "edit",
        "report_outcome",
    ];
    if let Some(tool) = agent.allowed_tools.as_ref().and_then(|tools| {
        tools
            .iter()
            .find(|tool| !supported.contains(&tool.as_str()))
    }) {
        return Err(format!(
            "Pi runtime does not support the configured {tool:?} tool"
        ));
    }
    use std::{process::Stdio, thread, time::Duration};
    if cancelled() {
        return Err("Agent execution cancelled".into());
    }
    let mut command = std::process::Command::new(&runtime.pi_command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut system_prompt = agent.instructions.clone();
    let extension_dir = if allowed_outcomes.is_empty() {
        None
    } else {
        let dir = tempfile::tempdir()
            .map_err(|error| format!("Could not prepare Pi extension: {error}"))?;
        let extension = dir.path().join("report-outcome.mjs");
        std::fs::write(&extension, include_str!("pi_report_outcome.js"))
            .map_err(|error| format!("Could not prepare Pi extension: {error}"))?;
        command.arg("--extension").arg(extension);
        command.env("BATON_PI_OUTCOME_FILE", dir.path().join("outcome.json"));
        command.env(
            "BATON_PI_ALLOWED_OUTCOMES",
            serde_json::to_string(allowed_outcomes).map_err(|error| error.to_string())?,
        );
        system_prompt.push_str("\n\nCall report_outcome exactly once with your pipeline decision and a concise summary. Then provide your complete handoff in your final answer.");
        Some(dir)
    };
    command.current_dir(directory).args([
        "--print",
        "--no-session",
        "--no-extensions",
        "--no-mcp",
        "--system-prompt",
        &system_prompt,
    ]);
    if let Some(model) = &agent.model {
        command.args(["--model", &format!("{}/{}", model.provider, model.model)]);
    }
    let default_tools = ["read", "list", "search", "shell", "write", "edit"].map(str::to_owned);
    let tools = agent.allowed_tools.as_deref().unwrap_or(&default_tools);
    if tools.is_empty() {
        command.arg("--no-tools");
    } else {
        let mut mapped = tools
            .iter()
            .flat_map(|tool| match tool.as_str() {
                "report_outcome" if allowed_outcomes.is_empty() => vec![],
                "shell" => vec!["bash"],
                "list" => vec!["ls"],
                "search" => vec!["grep", "find"],
                name => vec![name],
            })
            .collect::<Vec<_>>();
        if !allowed_outcomes.is_empty() && !mapped.contains(&"report_outcome") {
            mapped.push("report_outcome");
        }
        command.args(["--tools", &mapped.join(",")]);
    }
    let setup_trace = RunTraceEntry {
                    attempt: None,
                    occurred_at: baton_core::features::pipeline_engine::utc_now(),
                    kind: "runtimeSetup".into(),
                    content: serde_json::json!({"agentId": agent.id, "runtime": "pi", "model": agent.model.as_ref().map(|model| format!("{}/{}", model.provider, model.model)), "allowedOutcomes": allowed_outcomes}).to_string(),
                };
    observer(setup_trace.clone())?;
    runtime.trace.push(setup_trace);
    let mut child = command
        .arg("--")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Failed to run Pi CLI (is Pi installed?): {error}"))?;
    let (prompt_tx, prompt_rx) = mpsc::channel();
    let stdin = child.stdin.take().ok_or("Could not send Pi prompt")?;
    let input = input.to_owned();
    thread::spawn(move || {
        let mut stdin = stdin;
        let _ = prompt_tx.send(stdin.write_all(input.as_bytes()));
    });
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    let stdout = child.stdout.take().ok_or("Could not read Pi output")?;
    let stderr = child.stderr.take().ok_or("Could not read Pi diagnostics")?;
    thread::spawn(move || {
        let _ = stdout_tx.send(read_capped(stdout, 16 * 1024 * 1024));
    });
    thread::spawn(move || {
        let _ = stderr_tx.send(read_capped(stderr, 64 * 1024));
    });
    let status = loop {
        if cancelled() {
            kill_process_tree(&mut child);
            let _ = child.wait();
            return Err("Agent execution cancelled".into());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Could not wait for Pi CLI: {error}"))?
        {
            kill_process_tree(&mut child);
            break status;
        }
        thread::sleep(Duration::from_millis(25));
    };
    let (stdout, stdout_truncated, prompt_error) =
        collect_pi_io(&mut child, stdout_rx, stderr_rx, prompt_rx, cancelled)?;
    if stdout_truncated {
        return Err("Pi output exceeded the 16 MiB limit".into());
    }
    if !status.success() {
        return Err(format!(
            "Pi CLI failed with status {status}; ensure Pi provider credentials are configured (verify with `pi auth check --model provider/model`)"
        ));
    }
    if let Some(error) = prompt_error {
        return Err(error);
    }
    let output =
        String::from_utf8(stdout).map_err(|_| "Pi returned non-UTF-8 output".to_owned())?;
    if !allowed_outcomes.is_empty() {
        let path = extension_dir
            .as_ref()
            .ok_or("Pi outcome extension was not prepared")?
            .path()
            .join("outcome.json");
        let file =
            std::fs::File::open(path).map_err(|_| "Pi did not call report_outcome".to_owned())?;
        let (data, truncated) = read_capped(file, 16 * 1024 * 1024);
        if truncated {
            return Err("Pi outcome exceeded the 16 MiB limit".into());
        }
        let result: serde_json::Value = serde_json::from_slice(&data)
            .map_err(|_| "Pi returned an invalid report_outcome result".to_owned())?;
        let outcome: Outcome = serde_json::from_value(
            result
                .get("outcome")
                .cloned()
                .ok_or("Pi outcome JSON is missing outcome")?,
        )
        .map_err(|_| "Pi reported an invalid outcome".to_owned())?;
        let summary = result
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .ok_or("Pi outcome JSON is missing summary")?;
        if !allowed_outcomes.contains(&outcome) {
            return Err("Pi reported an outcome not allowed by this pipeline step".into());
        }
        runtime.decision = Some(AgentDecision {
            outcome,
            summary: summary.to_owned(),
        });
    }
    Ok(output)
}
