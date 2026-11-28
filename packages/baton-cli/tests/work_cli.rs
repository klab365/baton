mod common;

use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    path: PathBuf,
    vault: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        let output = Command::new("git").arg("init").arg(&path).output().unwrap();
        assert!(output.status.success());
        fs::create_dir(path.join(".baton")).unwrap();
        let example =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/session-factory/.baton");
        fs::copy(example.join("project.yml"), path.join(".baton/project.yml")).unwrap();
        for name in ["agents", "pipelines"] {
            fs::create_dir(path.join(".baton").join(name)).unwrap();
            for entry in fs::read_dir(example.join(name)).unwrap() {
                let entry = entry.unwrap();
                fs::copy(
                    entry.path(),
                    path.join(".baton").join(name).join(entry.file_name()),
                )
                .unwrap();
            }
        }
        fs::write(path.join("uncommitted"), "preserve").unwrap();
        let vault = path.join("vault");
        Self {
            _temp: temp,
            path,
            vault,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_baton"));
        command
            .current_dir(&self.path)
            .env("BATON_CREDENTIAL_DIR", &self.vault);
        command
    }
    fn work(&self, args: &[&str]) -> Output {
        self.command().arg("work").args(args).output().unwrap()
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().arg("run").args(args).output().unwrap()
    }
    fn value(&self, args: &[&str]) -> Value {
        let output = self.work(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn automatic_run_executes_managed_pipeline_with_fake_runtime() {
    let fixture = Fixture::new();
    for entry in fs::read_dir(fixture.path.join(".baton/agents")).unwrap() {
        let path = entry.unwrap().path();
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text.replacen("---\n", "---\nprovider: fake\n", 1)).unwrap();
    }
    fs::write(
        fixture.path.join(".baton/pipelines/factory.yml"),
        "---\nid: factory\nsteps:\n- id: execute\n  name: Execute\n  agentId: implementer\n  transitions:\n  - outcome: completed\n    to: human\n  type: agent\n- id: human\n  name: Human\n  type: human\n",
    )
    .unwrap();
    let git = Command::new("git")
        .current_dir(&fixture.path)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "fixture",
        ])
        .output()
        .unwrap();
    assert!(git.status.success());
    let output = fixture
        .command()
        .args([
            "run",
            fixture.path.to_str().unwrap(),
            "factory",
            "task prompt",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let run: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(run.get("id").is_some());
    assert_eq!(run["status"], "waitingHuman");
    assert_eq!(run["worktree"]["owner"], run["id"]);
}

#[test]
fn automatic_run_pause_can_be_resolved_through_server_api() {
    let fixture = Fixture::new();
    for entry in fs::read_dir(fixture.path.join(".baton/agents")).unwrap() {
        let path = entry.unwrap().path();
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text.replacen("---\n", "---\nprovider: fake\n", 1)).unwrap();
    }
    fs::write(
        fixture.path.join(".baton/pipelines/factory.yml"),
        "---\nid: factory\nsteps:\n- id: execute\n  name: Execute\n  agentId: implementer\n  transitions:\n  - outcome: completed\n    to: human\n  type: agent\n- id: human\n  name: Human\n  type: human\n",
    )
    .unwrap();
    let git = Command::new("git")
        .current_dir(&fixture.path)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "fixture",
        ])
        .output()
        .unwrap();
    assert!(git.status.success());
    let output = fixture
        .command()
        .args([
            "run",
            fixture.path.to_str().unwrap(),
            "factory",
            "task prompt",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run: Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = run["id"].as_str().unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut server = Server(
        fixture
            .command()
            .env("BATON_PORT", port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(server.0.try_wait().unwrap().is_none(), "server exited");
        assert!(Instant::now() < deadline, "server startup timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    let response = Command::new("curl")
        .args([
            "-sS",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "-d",
            r#"{"outcome":"approved","response":"approved"}"#,
            &format!("http://127.0.0.1:{port}/api/runs/{id}/actions"),
        ])
        .output()
        .unwrap();
    assert!(
        response.status.success(),
        "status={} stderr={} body={}",
        response.status,
        String::from_utf8_lossy(&response.stderr),
        String::from_utf8_lossy(&response.stdout)
    );
    let updated: Value = serde_json::from_slice(&response.stdout).unwrap();
    assert_eq!(
        updated["id"],
        id,
        "{}",
        String::from_utf8_lossy(&response.stdout)
    );
    assert_eq!(updated["status"], "succeeded");
}

#[test]
fn automatic_run_reports_invalid_pipeline_from_managed_start_command() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args([
            "run",
            fixture.path.to_str().unwrap(),
            "missing-pipeline",
            "task prompt",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Selected pipeline does not exist"));
}

#[test]
fn run_auto_escape_consumes_flag_and_uses_positional_form() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["run", "--auto", "missing-project", "factory", "task"])
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(!error.contains("Usage: baton run"));
}

#[test]
fn run_stepwise_alias_and_work_alias_produce_same_pipeline_listing() {
    let fixture = Fixture::new();
    let old = fixture.work(&["pipelines"]);
    let new = fixture.run(&["pipelines"]);
    assert_eq!(old.status.success(), new.status.success());
    assert_eq!(old.stdout, new.stdout);
    assert_eq!(old.stderr, new.stderr);
}

#[cfg(unix)]
#[test]
fn cli_persists_reported_handoff_instead_of_pi_final_response() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let bin = f.path.join("fake-bin");
    fs::create_dir(&bin).unwrap();
    let pi = bin.join("pi");
    fs::write(&pi, r##"#!/bin/sh
while [ "$#" -gt 0 ]; do
  if [ "$1" = '--session-dir' ]; then shift; session="$1"; fi
  if [ "$1" = '--session-id' ]; then shift; id="$1"; fi
  shift
done
mkdir -p "$session"
prompt=$(cat)
printf '%s\\n' "$prompt" >> "$session/inputs"
printf '{"type":"session","id":"%s","cwd":"%s"}\n{"type":"message","message":{"role":"user","content":"prior-conversation"}}\n{"type":"message","message":{"role":"assistant"}}\n' "$id" "$PWD" > "$session/test_$id.jsonl"
printf '{"loop_complete":true,"outcome":"completed","summary":"TOOL HANDOFF","structured_handoff":{"version":1,"summary":"TOOL HANDOFF","nextAction":"done"}}' > "$BATON_PI_OUTCOME_FILE"
printf 'FINAL ANSWER'
"##).unwrap();
    fs::set_permissions(&pi, fs::Permissions::from_mode(0o755)).unwrap();
    common::write_config(
        &f.path,
        &json!({
            "schemaVersion": 7,
            "agents": [{"id":"worker", "systemPrompt":"Implement", "provider":"pi", "skillIds":[], "allowedTools":["report_outcome"]}],
            "pipelines": [{"id":"factory", "steps":[{"id":"build", "name":"Build", "type":"agent", "agentId":"worker", "transitions":[{"outcome":"completed", "to":"review"}]}, {"id":"review", "name":"Review", "type":"agent", "agentId":"worker", "transitions":[{"outcome":"completed", "to":null}]}]}]
        }),
    );
    let start = f.value(&["start", "--issue", "Implement feature"]);
    let output = f
        .command()
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .args(["work", "execute", "--run", start["id"].as_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status = f.value(&["status", "--run", start["id"].as_str().unwrap()]);
    assert_eq!(status["steps"][0]["output"], "TOOL HANDOFF");
    assert_eq!(status["currentStep"], "review");
    let next = f.value(&["next", "--run", start["id"].as_str().unwrap()]);
    assert!(next.to_string().contains("TOOL HANDOFF"), "{next}");
    assert!(
        !status["steps"][0]["output"]
            .as_str()
            .unwrap()
            .contains("FINAL ANSWER")
    );
}

#[cfg(unix)]
#[test]
fn refine_reuses_pi_conversation_in_the_same_worktree_and_rejects_missing_history() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let bin = f.path.join("fake-bin");
    fs::create_dir(&bin).unwrap();
    let pi = bin.join("pi");
    fs::write(
        &pi,
        r##"#!/bin/sh
session_dir=""
existing=""
session_id=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --session-dir) shift; session_dir="$1" ;;
    --session) shift; existing="$1" ;;
    --session-id) shift; session_id="$1" ;;
  esac
  shift
done
prompt=$(cat)
printf '%s\t%s\n' "$session_dir" "$existing" >> "$PI_CALL_LOG"
printf '%s\n---END---\n' "$prompt" >> "$PI_PROMPT_LOG"
mkdir -p "$session_dir"
if [ -z "$existing" ]; then
  printf '{"type":"session","id":"%s","cwd":"%s"}\n{"type":"message","message":{"role":"user"}}\n{"type":"message","message":{"role":"assistant"}}\n' "$session_id" "$PWD" > "$session_dir/session_$session_id.jsonl"
fi
printf '{"loop_complete":true,"outcome":"completed","summary":"agent summary","structured_handoff":{"version":1,"summary":"agent summary","nextAction":"done"}}' > "$BATON_PI_OUTCOME_FILE"
printf 'completed'
"##,
    )
    .unwrap();
    fs::set_permissions(&pi, fs::Permissions::from_mode(0o755)).unwrap();
    common::write_config(
        &f.path,
        &json!({
            "schemaVersion": 7,
            "agents": [{"id":"worker", "systemPrompt":"Implement", "provider":"pi", "skillIds":[], "allowedTools":["report_outcome"]}],
            "pipelines": [{"id":"factory", "steps":[{"id":"build", "name":"Build", "type":"agent", "agentId":"worker", "transitions":[{"outcome":"completed", "to":null}]}]}]
        }),
    );
    let call_log = f.path.join("pi-calls.log");
    let prompt_log = f.path.join("pi-prompts.log");
    let env_path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());

    let original = f.value(&["start", "--issue", "Implement feature"]);
    let original_id = original["id"].as_str().unwrap();
    let execute = |id: &str| {
        f.command()
            .env("PATH", &env_path)
            .env("PI_CALL_LOG", &call_log)
            .env("PI_PROMPT_LOG", &prompt_log)
            .args(["work", "execute", "--run", id])
            .output()
            .unwrap()
    };
    let completed = execute(original_id);
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    assert_eq!(
        f.value(&["status", "--run", original_id])["status"],
        "succeeded"
    );
    common::write_config(
        &f.path,
        &json!({
            "schemaVersion": 7,
            "agents": [{"id":"worker", "systemPrompt":"Updated instructions", "provider":"pi", "skillIds":[], "allowedTools":["report_outcome"]}],
            "pipelines": [{"id":"factory", "steps":[{"id":"build", "name":"Build", "type":"agent", "agentId":"worker", "transitions":[{"outcome":"completed", "to":null}]}]}]
        }),
    );

    let refined = f.value(&[
        "refine",
        "--run",
        original_id,
        "--instruction",
        "Fix the remaining edge case",
    ]);
    let refined_id = refined["id"].as_str().unwrap();
    assert_ne!(refined_id, original_id);
    let old_status = f.value(&["status", "--run", original_id]);
    let new_status = f.value(&["status", "--run", refined_id]);
    assert_eq!(old_status["status"], "succeeded");
    assert_eq!(new_status["status"], "running");
    assert_eq!(new_status["session"]["workspace"], f.path.to_str().unwrap());
    assert_eq!(new_status["session"]["conversationId"], original_id);
    assert_eq!(
        new_status["session"]["conversationAgents"],
        json!(["worker"])
    );
    assert_eq!(
        new_status["configSnapshot"]["agents"][0]["instructions"],
        "Updated instructions"
    );
    assert!(
        new_status["handoff"]
            .as_str()
            .unwrap()
            .contains("agent summary"),
        "{new_status}"
    );
    assert!(
        new_status["handoff"]
            .as_str()
            .unwrap()
            .contains("Fix the remaining edge case")
    );

    let completed_refinement = execute(refined_id);
    assert!(
        completed_refinement.status.success(),
        "{}",
        String::from_utf8_lossy(&completed_refinement.stderr)
    );
    assert_eq!(
        f.value(&["status", "--run", refined_id])["status"],
        "succeeded"
    );
    let calls = fs::read_to_string(&call_log).unwrap();
    let calls: Vec<_> = calls.lines().collect();
    assert_eq!(calls.len(), 2);
    let (first_dir, first_session) = calls[0].split_once('\t').unwrap();
    let (second_dir, second_session) = calls[1].split_once('\t').unwrap();
    assert_eq!(first_dir, second_dir);
    assert!(first_session.is_empty());
    assert!(second_session.starts_with(first_dir));
    let prompts = fs::read_to_string(&prompt_log).unwrap();
    let refined_prompt = prompts.split("---END---").nth(1).unwrap();
    assert!(refined_prompt.contains("agent summary"));
    assert!(refined_prompt.contains("Fix the remaining edge case"));

    let final_refinement = f.value(&[
        "refine",
        "--run",
        refined_id,
        "--instruction",
        "Check missing conversation recovery",
    ]);
    let final_id = final_refinement["id"].as_str().unwrap();
    let conversation = PathBuf::from(first_dir);
    fs::remove_dir_all(&conversation).unwrap();
    let missing = execute(final_id);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("session is missing"));
    assert_eq!(f.value(&["status", "--run", final_id])["status"], "failed");
}

#[cfg(unix)]
#[test]
fn pi_policy_failure_is_persisted_without_provider_diagnostics() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let bin = f.path.join("fake-bin");
    fs::create_dir(&bin).unwrap();
    let pi = bin.join("pi");
    fs::write(&pi, r##"#!/bin/sh
while [ "$#" -gt 0 ]; do
  if [ "$1" = '--session-dir' ]; then shift; session="$1"; fi
  if [ "$1" = '--session-id' ]; then shift; id="$1"; fi
  shift
done
cat >/dev/null
printf '{"type":"session","id":"%s","cwd":"%s"}\n{"type":"message","message":{"role":"user"}}\n{"type":"message","message":{"role":"assistant","stopReason":"error","errorMessage":"content_policy secret-provider-diagnostic"}}\n' "$id" "$PWD" > "$session/test_$id.jsonl"
exit 1
"##).unwrap();
    fs::set_permissions(&pi, fs::Permissions::from_mode(0o755)).unwrap();
    common::write_config(
        &f.path,
        &json!({
            "schemaVersion":7,
            "agents":[{"id":"worker","systemPrompt":"Implement","provider":"pi","skillIds":[],"allowedTools":["report_outcome"]}],
            "pipelines":[{"id":"factory","steps":[{"id":"build","name":"Build","type":"agent","agentId":"worker"}]}]
        }),
    );
    let run = f.value(&["start", "--issue", "Implement feature"]);
    let id = run["id"].as_str().unwrap();
    let output = f
        .command()
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .args(["work", "execute", "--run", id])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("content policy"));
    assert!(!error.contains("secret-provider-diagnostic"));
    let status = f.value(&["status", "--run", id]);
    assert_eq!(status["status"], "failed");
    assert!(
        status["steps"][0]["error"]
            .as_str()
            .unwrap()
            .contains("content policy")
    );
    assert!(!status.to_string().contains("secret-provider-diagnostic"));
    assert_eq!(
        fs::read_to_string(f.path.join("uncommitted")).unwrap(),
        "preserve"
    );
}

#[test]
fn start_rejects_removed_max_attempts_flag_and_help_omits_it() {
    let f = Fixture::new();
    let output = f.work(&["start", "--issue", "task", "--max-attempts", "3"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("max-attempts"));
    let output = f.work(&["--help"]);
    let help: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!help["help"].as_str().unwrap().contains("max-attempts"));
}

#[test]
fn pipelines_lists_configured_ids_and_reports_invalid_configuration() {
    let f = Fixture::new();
    let pipelines = f.value(&["pipelines"]);
    assert!(pipelines.as_array().unwrap().contains(&json!("factory")));

    common::write_config(
        &f.path,
        &json!({"schemaVersion": 7, "agents": [], "pipelines": []}),
    );
    assert_eq!(f.value(&["pipelines"]), json!([]));

    fs::write(f.path.join(".baton/project.yml"), "not: [valid").unwrap();
    let output = f.work(&["pipelines"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(error["error"].as_str().unwrap().contains("Invalid"));
}

#[test]
fn progress_is_read_only_and_never_returns_runtime_trace_strings() {
    let f = Fixture::new();
    let start = f.value(&["start", "--issue", "task"]);
    let id = start["id"].as_str().unwrap();
    let empty_progress = f.value(&["progress", "--run", id]);
    assert_eq!(empty_progress["activity"], serde_json::Value::Null);
    assert_eq!(empty_progress["available"], false);
    assert_eq!(empty_progress["truncated"], false);
    let path = f.path.join(".baton/work").join(format!("{id}.json"));
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let last_step = record["steps"].as_array().unwrap().len() - 1;
    record["steps"][last_step]["usage"] = json!({
        "inputTokens": 12,
        "outputTokens": 4,
        "estimatedCost": "0.03",
        "currency": "USD",
        "priceUnit": "perMillionTokens",
        "coverage": "complete",
        "priceSource": "models.dev",
        "priceCheckedAt": "2026-01-01T00:00:00Z"
    });
    record["trace"] = json!([
        {
            "attempt": 999,
            "occurredAt": "now",
            "kind": "credential=secret",
            "content": "DO NOT LEAK THIS"
        },
        {
            "attempt": 999,
            "occurredAt": "now",
            "kind": "traceTruncated",
            "content": "private marker content"
        }
    ]);
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    let progress = f.value(&["progress", "--run", id]);
    assert_eq!(progress["activity"], serde_json::Value::Null);
    assert_eq!(progress["available"], false);
    assert_eq!(progress["truncated"], true);
    assert_eq!(progress["usage"]["estimatedCost"], "0.03");
    assert_eq!(progress["usage"]["currency"], "USD");
    assert_eq!(
        progress["usageObservations"][0]["usage"]["estimatedCost"],
        "0.03"
    );
    assert_eq!(progress["usageSummary"]["cost"], "0.03");
    assert_eq!(progress["usageSummary"]["currency"], "USD");
    assert!(!progress.to_string().contains("private marker content"));
    assert!(!progress.to_string().contains("DO NOT LEAK THIS"));
    assert!(!progress.to_string().contains("credential=secret"));
    assert_eq!(fs::read(&path).unwrap(), before);
    record["trace"].as_array_mut().unwrap().pop();
    let current_attempt = record["steps"].as_array().unwrap().last().unwrap()["attempt"].clone();
    record["trace"].as_array_mut().unwrap().push(json!({
        "attempt": current_attempt,
        "occurredAt": "now",
        "kind": "toolCall",
        "content": "sensitive tool details"
    }));
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let current = f.value(&["progress", "--run", id]);
    assert_eq!(current["activity"], "Tool activity");
    assert_eq!(current["available"], true);
    assert!(!current.to_string().contains("sensitive tool details"));
    record["trace"].as_array_mut().unwrap().last_mut().unwrap()["kind"] = json!("modelRequest");
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let model = f.value(&["progress", "--run", id]);
    assert_eq!(model["activity"], "Agent activity");
    record["trace"].as_array_mut().unwrap().last_mut().unwrap()["kind"] = json!("ralphIteration");
    record["trace"].as_array_mut().unwrap().last_mut().unwrap()["content"] =
        json!("Ralph loop 2/100");
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let loop_progress = f.value(&["progress", "--run", id]);
    assert_eq!(loop_progress["activity"], "Ralph loop 2/100");
    record["steps"].as_array_mut().unwrap().last_mut().unwrap()["attempt"] = json!(2);
    record["trace"].as_array_mut().unwrap().push(json!({
        "attempt": 1,
        "occurredAt": "now",
        "kind": "traceTruncated",
        "content": "Run trace limits reached; later events omitted"
    }));
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let exhausted = f.value(&["progress", "--run", id]);
    assert_eq!(exhausted["attempt"], 2);
    assert_eq!(exhausted["activity"], serde_json::Value::Null);
    assert_eq!(exhausted["available"], false);
    assert_eq!(exhausted["truncated"], true);
}

#[test]
fn cli_persists_steps_across_processes_and_emits_parseable_errors() {
    let f = Fixture::new();
    let start = f.value(&[
        "start",
        "--issue",
        "Fix login. Acceptance criteria: expired sessions recover",
        "--check",
        "build=test -f uncommitted",
    ]);
    let id = start["id"].as_str().unwrap();
    let listed = f.value(&["status"]);
    assert_eq!(listed[0]["id"], id);
    assert!(listed[0].get("steps").is_none());
    assert!(listed[0].get("task").is_none());
    let next = f.value(&["next", "--run", id]);
    assert!(next.get("task").is_none());
    assert!(start.get("task").is_none());
    assert!(start.get("steps").is_none());
    assert_eq!(next["step"]["id"], "build");
    let result = f.path.join("result.json");
    fs::write(&result, json!({"revision":0,"stepId":"build","attempt":1,"outcome":"completed","summary":"Changed and tested login"}).to_string()).unwrap();
    let result = result.to_str().unwrap();
    let submitted = f.value(&["submit", "--run", id, "--result", result]);
    assert_eq!(submitted["currentStep"], "review");
    assert!(submitted.get("steps").is_none());
    assert!(submitted.get("session").is_none());
    assert_eq!(submitted["revision"], 1);
    assert_eq!(
        f.value(&["status", "--run", id])["session"]["submissions"][0]["checks"][0]["exitCode"],
        0
    );
    let duplicate = f.work(&["submit", "--run", id, "--result", result]);
    assert!(!duplicate.status.success());
    let error: Value = serde_json::from_slice(&duplicate.stderr).unwrap();
    assert!(error["error"].as_str().unwrap().contains("Stale"));
    assert_eq!(f.value(&["status", "--run", id])["session"]["revision"], 1);
    assert_eq!(f.value(&["cancel", "--run", id])["status"], "cancelled");
    assert_eq!(
        fs::read_to_string(f.path.join("uncommitted")).unwrap(),
        "preserve"
    );
}

#[test]
fn cli_execute_runs_runtime_and_saves_result_across_processes() {
    let f = Fixture::new();
    common::write_config(
        &f.path,
        &json!({
            "schemaVersion": 7,
            "agents": [{"id":"worker", "systemPrompt":"Implement", "provider":"fake", "skillIds":[]}],
            "pipelines": [{"id":"factory", "steps":[{"id":"build", "name":"Build", "type":"agent", "agentId":"worker", "includeTask":true}]}]
        }),
    );
    let start = f.value(&["start", "--issue", "Implement feature"]);
    let id = start["id"].as_str().unwrap();
    assert_eq!(f.value(&["execute", "--run", id])["status"], "succeeded");
    let status = f.value(&["status", "--run", id]);
    assert_eq!(status["steps"][0]["outcome"], "completed");
    assert!(
        status["steps"][0]["input"]
            .as_str()
            .unwrap()
            .contains("Implement feature")
    );
    assert_eq!(
        fs::read_to_string(f.path.join("uncommitted")).unwrap(),
        "preserve"
    );
}

#[tokio::test]
async fn cli_executes_rig_against_local_ollama_provider() {
    use baton_core::{
        domain::credentials::{Credential, CredentialStatus, CredentialType},
        ports::credentials::CredentialRepository,
    };
    use std::io::{Read, Write};
    let f = Fixture::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let (request_seen_tx, request_seen_rx) = std::sync::mpsc::channel();
    let (continue_provider_tx, continue_provider_rx) = std::sync::mpsc::channel();
    let (stop_server_tx, stop_server_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        loop {
            if stop_server_rx.try_recv().is_ok() {
                break;
            }
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => panic!(
                    "provider accept failed after {} requests: {e}",
                    requests.len()
                ),
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            if requests.is_empty() {
                request_seen_tx.send(()).unwrap();
                continue_provider_rx
                    .recv_timeout(Duration::from_secs(30))
                    .expect("test should release provider response");
            }
            let request = String::from_utf8(request).unwrap();
            let request_body = request.split_once("\r\n\r\n").unwrap().1;
            let request_json: Value = serde_json::from_str(request_body).unwrap();
            let last_role = request_json["messages"]
                .as_array()
                .and_then(|messages| messages.last())
                .and_then(|message| message["role"].as_str());
            let body = if request.contains("Missing outcome") || last_role == Some("tool") {
                "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Rig finished\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
            } else {
                "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_report\",\"type\":\"function\",\"function\":{\"name\":\"report_outcome\",\"arguments\":\"{\\\"loop_complete\\\":true,\\\"outcome\\\":\\\"completed\\\",\\\"summary\\\":\\\"done\\\",\\\"structured_handoff\\\":{\\\"version\\\":1,\\\"summary\\\":\\\"done\\\",\\\"nextAction\\\":\\\"done\\\"}}\"}}]},\"finish_reason\":null}]}\n\ndata: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n"
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            requests.push(request);
        }
        requests
    });
    let store = baton_adapters::credentials::FilesystemCredentialRepository::for_project_in(
        &f.path, &f.vault,
    )
    .unwrap();
    let session_root = store.root_path().to_path_buf();
    store
        .save(
            &Credential {
                id: "local".into(),
                name: "local".into(),
                credential_type: CredentialType::new("ollama_api_key").unwrap(),
                configuration: [("baseUrl".into(), format!("http://{address}"))]
                    .into_iter()
                    .collect(),
                created_at: 0,
                updated_at: 0,
                status: CredentialStatus::Available,
            },
            Some("test-key"),
        )
        .await
        .unwrap();
    common::write_config(
        &f.path,
        &json!({
            "schemaVersion": 7,
            "agents": [
                {"id":"worker", "systemPrompt":"Implement", "provider":"rig", "model":"ollama/test", "credentialId":"local", "skillIds":[], "allowedTools":["report_outcome"]},
                {"id":"reviewer", "systemPrompt":"Review", "provider":"rig", "model":"ollama/test", "credentialId":"local", "skillIds":[], "allowedTools":["report_outcome"]},
                {"id":"missing", "systemPrompt":"Work", "provider":"rig", "model":"ollama/test", "credentialId":"local", "skillIds":[], "allowedTools":["report_outcome"]}
            ],
            "pipelines": [
                {"id":"factory", "steps":[{"id":"build", "name":"Build", "type":"agent", "agentId":"worker", "includeTask":true}, {"id":"human", "name":"Feedback", "type":"human"}, {"id":"continue", "name":"Continue", "type":"agent", "agentId":"worker"}, {"id":"review", "name":"Review", "type":"agent", "agentId":"reviewer"}]},
                {"id":"missing", "steps":[{"id":"work", "name":"Work", "type":"agent", "agentId":"missing", "includeTask":true, "transitions":[{"outcome":"completed", "to":null}]}]}
            ]
        }),
    );
    let start = f.value(&[
        "start",
        "--issue",
        "Implement feature",
        "--pipeline",
        "factory",
    ]);
    let id = start["id"].as_str().unwrap();
    let run_id = id.to_owned();
    let workspace = f.path.clone();
    let vault = f.vault.clone();
    let execution = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_baton"))
            .current_dir(workspace)
            .env("BATON_CREDENTIAL_DIR", vault)
            .args(["work", "execute", "--run", &run_id])
            .output()
            .unwrap()
    });
    request_seen_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut active = f.value(&["status", "--run", id]);
    while (!active["trace"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["kind"] == "modelRequest")
        || !active["events"].as_array().unwrap().iter().any(|event| {
            event["message"]
                .as_str()
                .unwrap_or_default()
                .contains("Observed runtime activity")
        }))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
        active = f.value(&["status", "--run", id]);
    }
    let saw_model_request = active["trace"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["kind"] == "modelRequest");
    let saw_runtime_activity = active["events"].as_array().unwrap().iter().any(|event| {
        event["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Observed runtime activity")
    });
    continue_provider_tx.send(()).unwrap();
    let completed = execution.await.unwrap();
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&completed.stdout).unwrap()["status"],
        "waitingHuman"
    );
    assert!(
        saw_model_request,
        "modelRequest trace was not persisted before provider release"
    );
    assert!(
        saw_runtime_activity,
        "runtime activity event was not persisted before provider release"
    );
    let response = f.path.join("human-response.json");
    fs::write(&response, json!({"revision":1,"stepId":"human","attempt":2,"outcome":"responded","response":"Include the human feedback"}).to_string()).unwrap();
    assert_eq!(
        f.value(&[
            "respond",
            "--run",
            id,
            "--result",
            response.to_str().unwrap()
        ])["status"],
        "running"
    );
    let session = session_root
        .join("runtime-sessions")
        .join(id)
        .join("agent-worker");
    fs::write(session.join("baton-inflight"), "interrupted").unwrap();
    let interrupted = f.work(&["execute", "--run", id]);
    assert!(!interrupted.status.success());
    assert!(String::from_utf8_lossy(&interrupted.stderr).contains("reconcile side effects"));
    fs::remove_file(session.join("baton-inflight")).unwrap();
    f.value(&["resume", "--run", id]);
    let backup = f.path.join("session-backup");
    fs::rename(&session, &backup).unwrap();
    let missing = f.work(&["execute", "--run", id]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("session is missing"));
    fs::rename(&backup, &session).unwrap();
    f.value(&["resume", "--run", id]);
    let log = fs::read_dir(session.join("logs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .unwrap();
    let saved_log = fs::read(&log).unwrap();
    fs::write(&log, b"not a valid session\n").unwrap();
    let corrupt = f.work(&["execute", "--run", id]);
    assert!(
        !corrupt.status.success(),
        "corrupted session must not silently start fresh"
    );
    fs::write(&log, saved_log).unwrap();
    f.value(&["resume", "--run", id]);
    let interrupted = f.work(&["execute", "--run", id]);
    assert!(!interrupted.status.success());
    assert!(String::from_utf8_lossy(&interrupted.stderr).contains("reconcile side effects"));
    fs::remove_file(session.join("baton-inflight")).unwrap();
    f.value(&["resume", "--run", id]);
    assert_eq!(f.value(&["execute", "--run", id])["status"], "running");
    assert_eq!(f.value(&["execute", "--run", id])["status"], "succeeded");
    let second = f.value(&["start", "--issue", "Another task", "--pipeline", "factory"]);
    let second_id = second["id"].as_str().unwrap();
    f.value(&["execute", "--run", second_id]);
    let missing = f.value(&[
        "start",
        "--issue",
        "Missing outcome",
        "--pipeline",
        "missing",
    ]);
    let missing_id = missing["id"].as_str().unwrap();
    let failed = f.work(&["execute", "--run", missing_id]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("without reporting"));
    let failed_run = f.value(&["status", "--run", missing_id]);
    assert_eq!(failed_run["status"], "failed");
    assert!(!failed_run["trace"].as_array().unwrap().is_empty());
    assert!(
        failed_run["steps"][0]["input"]
            .as_str()
            .unwrap()
            .contains("Missing outcome")
    );
    stop_server_tx.send(()).unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 11);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("Baton rejected your previous report"))
            .count(),
        2
    );
    assert!(requests[0].contains("Implement feature"));
    let assistant_messages = |request: &str| {
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let json: Value = serde_json::from_str(body).unwrap();
        json["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "assistant")
            .count()
    };
    let continued_agent = requests
        .iter()
        .find(|request| request.contains("Include the human feedback"))
        .unwrap();
    assert!(
        assistant_messages(continued_agent) > 0,
        "agent must receive its own earlier response"
    );
    let reviewer = requests
        .iter()
        .find(|request| request.contains("Review\\n\\nUse get_structured_handoff"))
        .unwrap();
    assert_eq!(
        assistant_messages(reviewer),
        0,
        "other agent must not inherit conversation"
    );
    let new_run = requests
        .iter()
        .find(|request| request.contains("Another task"))
        .unwrap();
    assert_eq!(
        assistant_messages(new_run),
        0,
        "new run must not inherit conversation"
    );
    let status = f.value(&["status", "--run", id]);
    assert!(
        status["steps"][0]["output"]
            .as_str()
            .unwrap()
            .contains("Rig finished")
    );
    assert!(!status["trace"].as_array().unwrap().is_empty());
    assert_eq!(status["steps"][0]["model"], "ollama/test");
    assert!(status["steps"][0]["usage"].is_null());
    assert!(status["trace"].to_string().len() < 256 * 1024);
    assert!(!status["trace"].to_string().contains("Implement feature"));
    assert!(!status["trace"].to_string().contains("test-key"));
    assert!(!status.to_string().contains("test-key"));
}

#[test]
fn cli_is_independent_of_a_running_server_and_rejects_removed_server_option() {
    let f = Fixture::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut server = Server(
        f.command()
            .env("BATON_PORT", port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(server.0.try_wait().unwrap().is_none(), "server exited");
        assert!(Instant::now() < deadline, "server startup timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut command = f.command();
    let output = command
        .env("BATON_PORT", port.to_string())
        .args(["work", "start", "--issue", "Fix login"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let start: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        f.value(&["next", "--run", start["id"].as_str().unwrap()])["attempt"],
        1
    );
    let server_runs =
        baton_adapters::repositories::filesystem_run::FilesystemRunRepository(f.path.clone());
    assert!(server_runs.list().unwrap().is_empty());
    assert!(
        f.path
            .join(".baton/work")
            .join(format!("{}.json", start["id"].as_str().unwrap()))
            .exists()
    );
    let output = f.work(&["status", "--server", "http://127.0.0.1:1"]);
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(error["error"].as_str().unwrap().contains("--server"));
}

#[test]
fn start_treats_urls_ids_and_requests_as_literal_task_text() {
    let f = Fixture::new();
    for task in [
        "123",
        "https://github.com/klab365/baton/issues/87",
        "Löse Issue https://example.com/issues/123",
    ] {
        let start = f.value(&["start", "--issue", task]);
        let saved = f.value(&["status", "--run", start["id"].as_str().unwrap()]);
        assert_eq!(saved["task"], task);
        assert!(saved["session"]["source"].is_null());
        assert_eq!(start["currentStep"], "build");
    }
}

#[test]
fn work_help_and_errors_are_json_without_a_format_flag() {
    let f = Fixture::new();
    for args in [vec![], vec!["--help"], vec!["-h"]] {
        let output = f.work(&args);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let help: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(help["help"].as_str().unwrap().contains("baton work"));
    }
    let invalid = f.path.join("invalid.json");
    fs::write(&invalid, "not JSON").unwrap();
    for args in [
        vec!["next"],
        vec!["status", "--json"],
        vec![
            "submit",
            "--run",
            "some-run",
            "--result",
            invalid.to_str().unwrap(),
        ],
        vec!["start", "--issue", "test", "--pipeline", "unknown-pipeline"],
    ] {
        let output = f.work(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert!(!error["error"].as_str().unwrap().is_empty());
    }
}
