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
        fs::write(
            path.join(".baton/config.json"),
            include_str!("../../../examples/session-factory/.baton/config.json"),
        )
        .unwrap();
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
    fs::write(f.path.join(".baton/config.json"), json!({
        "schemaVersion": 7,
        "agents": [{"id":"worker", "systemPrompt":"Implement", "runtimeKind":"fake", "skillIds":[]}],
        "pipelines": [{"id":"factory", "steps":[{"id":"build", "name":"Build", "agentId":"worker"}]}]
    }).to_string()).unwrap();
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
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..5 {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => panic!("provider accept failed: {e}"),
                }
            };
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
            let body = "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Rig finished\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            requests.push(String::from_utf8(request).unwrap());
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
    fs::write(f.path.join(".baton/config.json"), json!({
        "schemaVersion": 7,
        "agents": [
            {"id":"worker", "systemPrompt":"Implement", "runtimeKind":"rig", "model":{"provider":"ollama", "model":"test"}, "credentialId":"local", "skillIds":[], "allowedTools":[]},
            {"id":"reviewer", "systemPrompt":"Review", "runtimeKind":"rig", "model":{"provider":"ollama", "model":"test"}, "credentialId":"local", "skillIds":[], "allowedTools":[]},
            {"id":"missing", "systemPrompt":"Work", "runtimeKind":"rig", "model":{"provider":"ollama", "model":"test"}, "credentialId":"local", "skillIds":[], "allowedTools":["report_outcome"]}
        ],
        "pipelines": [
            {"id":"factory", "steps":[{"id":"build", "name":"Build", "agentId":"worker"}, {"id":"human", "name":"Feedback", "human":true}, {"id":"continue", "name":"Continue", "agentId":"worker"}, {"id":"review", "name":"Review", "agentId":"reviewer"}]},
            {"id":"missing", "steps":[{"id":"work", "name":"Work", "agentId":"missing", "transitions":[{"outcome":"completed", "to":null}]}]}
        ]
    }).to_string()).unwrap();
    let start = f.value(&[
        "start",
        "--issue",
        "Implement feature",
        "--pipeline",
        "factory",
    ]);
    let id = start["id"].as_str().unwrap();
    assert_eq!(f.value(&["execute", "--run", id])["status"], "waitingHuman");
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
    let backup = f.path.join("session-backup");
    fs::rename(&session, &backup).unwrap();
    let missing = f.work(&["execute", "--run", id]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("session is missing"));
    fs::rename(&backup, &session).unwrap();
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
    let interrupted = f.work(&["execute", "--run", id]);
    assert!(!interrupted.status.success());
    assert!(String::from_utf8_lossy(&interrupted.stderr).contains("reconcile side effects"));
    fs::remove_file(session.join("baton-inflight")).unwrap();
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
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 5);
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
    assert!(
        assistant_messages(&requests[1]) > 0,
        "agent must receive its own earlier response"
    );
    assert!(requests[1].contains("Include the human feedback"));
    assert_eq!(
        assistant_messages(&requests[2]),
        0,
        "other agent must not inherit conversation"
    );
    assert_eq!(
        assistant_messages(&requests[3]),
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
