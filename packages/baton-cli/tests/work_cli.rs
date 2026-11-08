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
    fs::write(&result, json!({"revision":0,"stepId":"build","attempt":1,"outcome":"completed","summary":"Changed and tested login","evidence":["actual verification"]}).to_string()).unwrap();
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
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("Unknown option --server")
    );
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
