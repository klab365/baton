use baton_cli::work::application::{WorkApplication, WorkCommand};
use serde_json::{Value, json};
struct Fixture {
    _temp: tempfile::TempDir,
    path: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        std::fs::create_dir(path.join(".baton")).unwrap();
        std::fs::write(path.join(".baton/config.json"), json!({
            "schemaVersion": 7,
            "agents": [{"id":"worker", "systemPrompt":"Follow the original issue and its acceptance criteria", "skillIds":[]}],
            "pipelines": [{"id":"factory", "steps":[
                {"id":"build", "name":"Build", "agentId":"worker"},
                {"id":"review", "name":"Review", "agentId":"worker", "transitions":[
                    {"outcome":"approved", "to":"human"},
                    {"outcome":"revise_build", "to":"build"}
                ]},
                {"id":"human", "name":"Human decision", "human":true}
            ]}]
        }).to_string()).unwrap();
        std::fs::write(path.join("uncommitted.txt"), "keep me").unwrap();
        Self { _temp: temp, path }
    }
    fn app(&self) -> WorkApplication {
        WorkApplication::new(self.path.clone()).unwrap()
    }
    async fn call(&self, app: &WorkApplication, command: Value) -> Result<Value, String> {
        let command: WorkCommand = serde_json::from_value(command).unwrap();
        let mutation = !matches!(
            &command,
            WorkCommand::Next { .. } | WorkCommand::Status { .. }
        );
        let result = app.execute(command).await?;
        if mutation {
            app.execute(WorkCommand::Status {
                id: Some(result["id"].as_str().unwrap().into()),
            })
            .await
        } else {
            Ok(result)
        }
    }
    async fn start(&self, app: &WorkApplication, limit: u32, checks: Value) -> Value {
        self.call(app, json!({"command":"start", "task":"Issue body\nAcceptance criteria: preserve the original wording", "source":"https://github.com/a/b/issues/1", "pipeline":null, "maxAttempts":limit, "checks":checks})).await.unwrap()
    }
    async fn submit(
        &self,
        app: &WorkApplication,
        run: &Value,
        outcome: &str,
    ) -> Result<Value, String> {
        self.call(app, submission(run, outcome)).await
    }
}
fn submission(run: &Value, outcome: &str) -> Value {
    let step = run["steps"].as_array().unwrap().last().unwrap();
    json!({"command":"submit", "id":run["id"], "result":{
        "revision":run["session"]["revision"], "stepId":step["stepId"], "attempt":step["attempt"],
        "outcome":outcome, "summary":"Result and acceptance criterion verification", "evidence":["src/change.rs and verified behavior"]
    }})
}

#[tokio::test]
async fn rework_and_human_wait_survive_reopening_without_runtime_or_worktree_allocation() {
    let f = Fixture::new();
    let app = f.app();
    let start = f.start(&app, 50, json!([])).await;
    assert!(start["worktree"].is_null());
    assert_eq!(start["session"]["workspace"], json!(f.path));
    let id = start["id"].as_str().unwrap();
    let next = f
        .call(&app, json!({"command":"next", "id":id}))
        .await
        .unwrap();
    assert_eq!(
        next["agent"]["systemPrompt"],
        "Follow the original issue and its acceptance criteria"
    );
    assert_eq!(
        next,
        f.call(&app, json!({"command":"next", "id":id}))
            .await
            .unwrap()
    );
    let review = f.submit(&app, &start, "completed").await.unwrap();
    assert_eq!(review["currentStep"], "review");
    assert!(
        f.submit(&app, &start, "completed")
            .await
            .unwrap_err()
            .contains("Stale")
    );
    let rework = f.submit(&app, &review, "revise_build").await.unwrap();
    assert_eq!(rework["currentStep"], "build");
    assert_eq!(rework["steps"][2]["attempt"], 3);
    drop(app);
    let app = f.app();
    let restored = f
        .call(&app, json!({"command":"status", "id":id}))
        .await
        .unwrap();
    assert_eq!(restored, rework);
    let review = f.submit(&app, &restored, "completed").await.unwrap();
    let human = f.submit(&app, &review, "approved").await.unwrap();
    assert_eq!(human["status"], "waitingHuman");
    assert!(f.submit(&app, &human, "approved").await.is_err());
    let result = json!({"command":"respond", "id":id, "action":{
        "revision":human["session"]["revision"], "stepId":"human", "attempt":5,
        "outcome":"approved", "response":"User approved"
    }});
    drop(app);
    let app = f.app();
    let done = f.call(&app, result.clone()).await.unwrap();
    assert_eq!(done["status"], "succeeded");
    assert!(!done["endedAt"].is_null());
    assert!(f.call(&app, result).await.is_err());
    assert_eq!(
        std::fs::read_to_string(f.path.join("uncommitted.txt")).unwrap(),
        "keep me"
    );
    assert!(!f.path.join(".git").exists());
    assert!(!f.path.join(".baton/runs").exists());
    assert!(
        f.path
            .join(".baton/work")
            .join(format!("{id}.json"))
            .is_file()
    );
}
#[tokio::test]
async fn legacy_execution_modes_are_accepted_and_removed_from_snapshots() {
    let f = Fixture::new();
    let config_path = f.path.join(".baton/config.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    config["pipelines"][0]["steps"][1]["execution"] = json!("subagent");
    std::fs::write(&config_path, config.to_string()).unwrap();
    let app = f.app();
    let start = f.start(&app, 50, json!([])).await;
    let id = start["id"].as_str().unwrap();
    let inline = f
        .call(&app, json!({"command":"next", "id":id}))
        .await
        .unwrap();
    assert!(inline["step"].get("execution").is_none());
    config["pipelines"][0]["steps"][1]["execution"] = json!("inline");
    std::fs::write(config_path, config.to_string()).unwrap();
    f.submit(&app, &start, "completed").await.unwrap();
    let delegated = f
        .call(&app, json!({"command":"next", "id":id}))
        .await
        .unwrap();
    assert!(delegated["step"].get("execution").is_none());
}

#[tokio::test]
async fn checks_execute_in_the_workspace_and_failed_checks_do_not_advance() {
    let f = Fixture::new();
    let app = f.app();
    let start = f
        .start(
            &app,
            50,
            json!([{"stepId":"build", "command":"test -f gate-ready && printf verified"}]),
        )
        .await;
    assert!(
        f.submit(&app, &start, "completed")
            .await
            .unwrap_err()
            .contains("Configured check failed")
    );
    assert_eq!(
        f.call(&app, json!({"command":"status", "id":start["id"]}))
            .await
            .unwrap(),
        start
    );
    std::fs::write(f.path.join("gate-ready"), "ready").unwrap();
    let accepted = f.submit(&app, &start, "completed").await.unwrap();
    assert_eq!(
        accepted["session"]["submissions"][0]["checks"][0]["exitCode"],
        0
    );
    assert_eq!(
        accepted["session"]["submissions"][0]["checks"][0]["output"],
        "verified"
    );
}
#[tokio::test]
async fn successful_terminal_outcomes_cannot_bypass_checks() {
    let f = Fixture::new();
    let config_path = f.path.join(".baton/config.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    config["pipelines"][0]["steps"][0]["transitions"] = json!([
        {"outcome":"responded", "to":null},
        {"outcome":"revise_build", "to":null}
    ]);
    std::fs::write(config_path, config.to_string()).unwrap();
    let app = f.app();
    let start = f
        .start(&app, 50, json!([{"stepId":"build", "command":"false"}]))
        .await;
    for outcome in ["responded", "revise_build"] {
        assert!(
            f.submit(&app, &start, outcome)
                .await
                .unwrap_err()
                .contains("Configured check failed")
        );
    }
    assert_eq!(
        f.call(&app, json!({"command":"status", "id":start["id"]}))
            .await
            .unwrap(),
        start
    );
}

#[tokio::test]
async fn concurrent_duplicate_submissions_advance_only_once() {
    let f = Fixture::new();
    let app = f.app();
    let start = f.start(&app, 50, json!([])).await;
    let (a, b) = tokio::join!(
        f.submit(&app, &start, "completed"),
        f.submit(&app, &start, "completed")
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let status = f
        .call(&app, json!({"command":"status", "id":start["id"]}))
        .await
        .unwrap();
    assert_eq!(status["session"]["revision"], 1);
    assert_eq!(status["steps"].as_array().unwrap().len(), 2);
}
#[tokio::test]
async fn attempt_limits_cancellation_and_invalid_submissions_preserve_workspace() {
    let f = Fixture::new();
    let app = f.app();
    let start = f.start(&app, 1, json!([])).await;
    assert!(f.submit(&app, &start, "approved").await.is_err());
    let mut empty = submission(&start, "completed");
    empty["result"]["evidence"] = json!([]);
    assert!(f.call(&app, empty).await.is_err());
    let limited = f.submit(&app, &start, "completed").await.unwrap();
    assert_eq!(limited["status"], "limitExhausted");
    assert!(!limited["endedAt"].is_null());
    let start = f.start(&app, 50, json!([])).await;
    let cancelled = f
        .call(&app, json!({"command":"cancel", "id":start["id"]}))
        .await
        .unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    assert!(f.submit(&app, &start, "completed").await.is_err());
    assert_eq!(
        std::fs::read_to_string(f.path.join("uncommitted.txt")).unwrap(),
        "keep me"
    );
}
#[tokio::test]
async fn standalone_lock_excludes_other_work_processes_without_initializing_server_or_credentials()
{
    let f = Fixture::new();
    let app = f.app();
    assert!(WorkApplication::new(f.path.clone()).is_err());
    assert!(!f.path.join(".baton/runs").exists());
    assert!(
        !std::fs::read_to_string(f.path.join(".baton/config.json"))
            .unwrap()
            .contains("projectId")
    );
    drop(app);
    assert!(WorkApplication::new(f.path.clone()).is_ok());
}

#[tokio::test]
async fn next_leaves_context_with_pi_while_explicit_status_retains_decisions() {
    let f = Fixture::new();
    let config_path = f.path.join(".baton/config.json");
    let mut config: Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    config["pipelines"][0]["steps"] = json!([
        {"id":"clarify", "name":"Clarify scope", "human":true},
        {"id":"plan", "name":"Plan", "agentId":"worker"},
        {"id":"build", "name":"Build", "agentId":"worker"},
        {"id":"review", "name":"Review", "agentId":"worker"}
    ]);
    std::fs::write(config_path, config.to_string()).unwrap();
    let app = f.app();
    let start = f.start(&app, 50, json!([])).await;
    let id = start["id"].as_str().unwrap();
    let mut answer = String::from("Support Tokio, WASM and Embassy. ");
    answer.push_str(&"x".repeat(16_384 - answer.len()));
    let planned = f.call(&app, json!({"command":"respond", "id":id, "action":{
        "revision":0, "stepId":"clarify", "attempt":1, "outcome":"responded", "response":answer
    }})).await.unwrap();
    let built = f.submit(&app, &planned, "completed").await.unwrap();
    let reviewed = f.submit(&app, &built, "completed").await.unwrap();
    assert_eq!(reviewed["currentStep"], "review");
    drop(app);
    let app = f.app();
    let next = f
        .call(&app, json!({"command":"next", "id":id}))
        .await
        .unwrap();
    for field in ["input", "handoff", "task", "source", "steps"] {
        assert!(next.get(field).is_none(), "next must not replay {field}");
    }
    assert!(!next.to_string().contains(&answer));
    assert_eq!(next["step"]["id"], "review");
    let saved = f
        .call(&app, json!({"command":"status", "id":id}))
        .await
        .unwrap();
    assert_eq!(saved["task"], start["task"]);
    assert_eq!(saved["steps"][0]["output"], answer);
    assert!(
        saved["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["input"].is_null())
    );
}
