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

fn session_file(directory: &Path, worktree: &Path) -> Result<Option<std::path::PathBuf>, String> {
    let mut files = Vec::new();
    for entry in
        std::fs::read_dir(directory).map_err(|e| format!("Read Pi session directory: {e}"))?
    {
        let entry = entry.map_err(|e| format!("Read Pi session directory: {e}"))?;
        if entry.path().extension().is_some_and(|ext| ext == "jsonl") {
            files.push(entry.path());
        }
    }
    if files.len() > 1 {
        return Err("Multiple Pi sessions found; manual recovery required".into());
    }
    let Some(path) = files.pop() else {
        return Ok(None);
    };
    let file = std::fs::File::open(&path).map_err(|e| format!("Open Pi session: {e}"))?;
    let (data, truncated) = read_capped(file, 16 * 1024 * 1024);
    if truncated {
        return Err("Pi session exceeds 16 MiB; manual recovery required".into());
    }
    let text = std::str::from_utf8(&data)
        .map_err(|_| "Pi session is invalid; manual recovery required")?;
    let mut lines = text.lines();
    let header: serde_json::Value = serde_json::from_str(
        lines
            .next()
            .ok_or("Pi session is empty; manual recovery required")?,
    )
    .map_err(|_| "Pi session is invalid; manual recovery required")?;
    let id = header
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("Pi session identity is missing; manual recovery required")?;
    if header.get("type").and_then(|v| v.as_str()) != Some("session")
        || id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        || !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(&format!("_{id}.jsonl")))
    {
        return Err("Pi session identity is invalid; manual recovery required".into());
    }
    let saved_cwd = header
        .get("cwd")
        .and_then(|v| v.as_str())
        .ok_or("Pi session working directory is missing; manual recovery required")?;
    let expected_cwd =
        std::fs::canonicalize(worktree).map_err(|e| format!("Inspect Pi worktree: {e}"))?;
    let actual_cwd = std::fs::canonicalize(saved_cwd)
        .map_err(|_| "Pi session working directory is unavailable; manual recovery required")?;
    if expected_cwd != actual_cwd {
        return Err("Pi session belongs to a different worktree; manual recovery required".into());
    }
    let (mut user, mut assistant) = (false, false);
    for line in lines {
        let entry: serde_json::Value = serde_json::from_str(line)
            .map_err(|_| "Pi session is invalid; manual recovery required")?;
        if entry.get("type").and_then(|v| v.as_str()) == Some("message") {
            match entry
                .get("message")
                .and_then(|m| m.get("role"))
                .and_then(|r| r.as_str())
            {
                Some("user") => user = true,
                Some("assistant") => assistant = true,
                _ => {}
            }
        }
    }
    if !user || !assistant {
        return Err("Pi session history is missing; manual recovery required".into());
    }
    Ok(Some(path))
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
    if agent.id.is_empty()
        || !agent
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err("Agent ID contains invalid path characters".into());
    }
    let session = runtime
        .auth_root
        .join("runtime-sessions")
        .join(&runtime.run_id)
        .join(format!("agent-{}", agent.id))
        .join("pi");
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&session)
        .map_err(|e| format!("Create Pi session directory: {e}"))?;
    let mut prompt = input.to_owned();
    for attempt in 0..=2 {
        let existing = session_file(&session, directory)?;
        if (runtime.history_required || attempt > 0) && existing.is_none() {
            return Err("Pi agent session is missing; manual recovery required".into());
        }
        let marker = session.join("baton-inflight");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&marker).map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { "Previous Pi execution was interrupted or failed; reconcile side effects and remove its baton-inflight marker before resuming".to_owned() } else { format!("Create Pi execution marker: {e}") })?;
        file.write_all(b"Agent execution requires reconciliation if interrupted.\n")
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("Write Pi execution marker: {e}"))?;
        drop(file);
        let output = execute_once(
            runtime,
            directory,
            agent,
            &prompt,
            allowed_outcomes,
            PiExecution {
                cancelled,
                observer,
                session: &session,
                existing: existing.as_deref(),
            },
        )?;
        if session_file(&session, directory)?.is_none() {
            return Err("Pi did not persist its session; manual recovery required".into());
        }
        std::fs::remove_file(marker).map_err(|e| format!("Clear Pi execution marker: {e}"))?;
        if allowed_outcomes.is_empty() || runtime.decision.is_some() || attempt == 2 {
            return Ok(output);
        }
        let allowed = allowed_outcomes
            .iter()
            .map(|o| {
                serde_json::to_value(o)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(", ");
        prompt = format!(
            "You did not call report_outcome with an allowed outcome. Call report_outcome now with one of: {allowed}, then provide your complete handoff. Do not repeat the task."
        );
    }
    unreachable!()
}

struct PiExecution<'a> {
    cancelled: &'a dyn Fn() -> bool,
    observer: &'a mut dyn FnMut(RunTraceEntry) -> Result<(), String>,
    session: &'a Path,
    existing: Option<&'a Path>,
}

fn execute_once(
    runtime: &mut ConfiguredRuntime,
    directory: &Path,
    agent: &Agent,
    input: &str,
    allowed_outcomes: &[Outcome],
    context: PiExecution<'_>,
) -> Result<String, String> {
    let PiExecution {
        cancelled,
        observer,
        session,
        existing,
    } = context;
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
        "--no-extensions",
        "--no-mcp",
        "--system-prompt",
        &system_prompt,
    ]);
    command.arg("--session-dir").arg(session);
    if let Some(path) = existing {
        command.arg("--session").arg(path);
    } else {
        command.args(["--session-id", &uuid::Uuid::now_v7().to_string()]);
    }
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
        match std::fs::File::open(path) {
            Ok(file) => {
                let (data, truncated) = read_capped(file, 16 * 1024 * 1024);
                if truncated {
                    return Err("Pi outcome exceeded the 16 MiB limit".into());
                }
                if let Ok(result) = serde_json::from_slice::<serde_json::Value>(&data)
                    && let (Some(outcome), Some(summary)) = (
                        result
                            .get("outcome")
                            .cloned()
                            .and_then(|value| serde_json::from_value::<Outcome>(value).ok()),
                        result.get("summary").and_then(serde_json::Value::as_str),
                    )
                    && allowed_outcomes.contains(&outcome)
                {
                    runtime.decision = Some(AgentDecision {
                        outcome,
                        summary: summary.to_owned(),
                    });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Read Pi outcome: {error}")),
        }
    }
    Ok(output)
}
