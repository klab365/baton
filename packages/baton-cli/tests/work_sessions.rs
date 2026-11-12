mod common;

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
        common::write_config(
            &path,
            &json!({
                "schemaVersion": 7,
                "agents": [{"id":"worker", "systemPrompt":"Follow the original issue and its acceptance criteria", "skillIds":[]}],
                "pipelines": [{"id":"factory", "steps":[
                    {"id":"build", "name":"Build", "type":"agent", "agentId":"worker", "transitions":[
                        {"outcome":"revise_build", "to":"build"},
                        {"outcome":"completed", "to":"review"}
                    ]},
                    {"id":"review", "name":"Review", "type":"agent", "agentId":"worker", "transitions":[
                        {"outcome":"approved", "to":"human"},
                        {"outcome":"revise_build", "to":"build"}
                    ]},
                    {"id":"human", "name":"Human decision", "type":"human"}
                ]}]
            }),
        );
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
        "outcome":outcome, "summary":"Result and acceptance criterion verification"
    }})
}

#[tokio::test]
async fn execute_runs_configured_agent_in_existing_workspace() {
    let f = Fixture::new();
    let mut config = common::read_config(&f.path);
    config["agents"][0]["provider"] = json!("fake");
    config["pipelines"][0]["steps"][0]["transitions"] = json!([]);
    config["pipelines"][0]["steps"][0]["prompt"] = json!("CLI-specific step prompt");
    common::write_config(&f.path, &config);
    let app = f.app();
    let run = f.start(&app, 5, json!([])).await;
    let result = f
        .call(&app, json!({"command":"execute", "id":run["id"]}))
        .await
        .unwrap();
    assert_eq!(result["currentStep"], "review");
    assert_eq!(result["steps"][0]["outcome"], "completed");
    assert!(
        result["steps"][0]["input"]
            .as_str()
            .unwrap()
            .contains("CLI-specific step prompt")
    );
    assert!(
        result["steps"][0]["output"]
            .as_str()
            .unwrap()
            .contains("CLI-specific step prompt")
    );
    assert!(result["steps"][0]["startedAt"].is_string());
    assert!(result["steps"][0]["endedAt"].is_string());
    assert!(result["steps"][0]["durationMs"].is_number());
    assert!(result["steps"][0]["usage"].is_null());
    assert!(
        result["steps"][0]["output"]
            .as_str()
            .unwrap()
            .contains("Acceptance criteria")
    );
    assert_eq!(
        std::fs::read_to_string(f.path.join("uncommitted.txt")).unwrap(),
        "keep me"
    );
}

#[tokio::test]
async fn missing_agent_outcome_stops_the_run() {
    let f = Fixture::new();
    let mut config = common::read_config(&f.path);
    config["agents"][0]["provider"] = json!("fake");
    common::write_config(&f.path, &config);
    let app = f.app();
    let run = f.start(&app, 50, json!([])).await;
    let id = run["id"].as_str().unwrap().to_owned();
    let error = app
        .execute(WorkCommand::Execute { id: id.clone() })
        .await
        .unwrap_err();
    assert!(error.contains("without reporting"));
    let status = app
        .execute(WorkCommand::Status { id: Some(id) })
        .await
        .unwrap();
    assert_eq!(status["status"], "failed");
    assert!(status["steps"][0]["startedAt"].is_string());
    assert!(status["steps"][0]["endedAt"].is_string());
    assert!(status["steps"][0]["durationMs"].is_number());
    assert_eq!(status["steps"].as_array().unwrap().len(), 1);
    assert!(
        status["steps"][0]["error"]
            .as_str()
            .unwrap()
            .contains("valid configured outcome")
    );
}

#[tokio::test]
async fn execute_command_steps_record_success_and_failure() {
    for (command, expected) in [
        ("printf done", "succeeded"),
        ("exit 1", "failed"),
        ("yes x | head -c 20000", "succeeded"),
    ] {
        let f = Fixture::new();
        common::write_config(
            &f.path,
            &json!({
                "schemaVersion": 7, "agents": [],
                "pipelines": [{"id":"factory", "steps":[{"id":"check", "name":"Check", "type":"shell", "command":command}]}]
            }),
        );
        let app = f.app();
        let run = f.start(&app, 5, json!([])).await;
        let result = f
            .call(&app, json!({"command":"execute", "id":run["id"]}))
            .await
            .unwrap();
        assert_eq!(result["status"], expected);
        assert!(
            result["steps"][0]["output"]
                .as_str()
                .unwrap()
                .contains("Exit status:")
        );
    }
}

#[tokio::test]
async fn running_step_allows_status_and_cancellation() {
    let f = Fixture::new();
    common::write_config(
        &f.path,
        &json!({
            "schemaVersion": 7, "agents": [],
            "pipelines": [{"id":"factory", "steps":[{"id":"check", "name":"Check", "type":"shell", "command":"touch started; sleep 30; touch finished"}]}]
        }),
    );
    let app = std::sync::Arc::new(f.app());
    let run = f.start(&app, 5, json!([])).await;
    let id = run["id"].as_str().unwrap().to_owned();
    let executing = {
        let app = app.clone();
        let id = id.clone();
        tokio::spawn(async move { app.execute(WorkCommand::Execute { id }).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !f.path.join("started").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let other = f.app();
    assert_eq!(
        other
            .execute(WorkCommand::Status {
                id: Some(id.clone())
            })
            .await
            .unwrap()["status"],
        "running"
    );
    assert!(
        other
            .execute(WorkCommand::Execute { id: id.clone() })
            .await
            .is_err()
    );
    assert_eq!(
        other.execute(WorkCommand::Cancel { id }).await.unwrap()["status"],
        "cancelled"
    );
    drop(other);
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), executing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result["status"], "cancelled");
    assert!(!f.path.join("finished").exists());
}

#[tokio::test]
async fn execute_refuses_human_steps_without_advancing() {
    let f = Fixture::new();
    let app = f.app();
    let run = f.start(&app, 5, json!([])).await;
    let run = f.submit(&app, &run, "completed").await.unwrap();
    let run = f.submit(&app, &run, "approved").await.unwrap();
    let error = app
        .execute(WorkCommand::Execute {
            id: run["id"].as_str().unwrap().into(),
        })
        .await
        .unwrap_err();
    assert!(error.contains("explicit response"));
    let current = app
        .execute(WorkCommand::Status {
            id: Some(run["id"].as_str().unwrap().into()),
        })
        .await
        .unwrap();
    assert_eq!(current["status"], "waitingHuman");
    assert_eq!(current["session"]["revision"], run["session"]["revision"]);
}

#[tokio::test]
async fn unconfigured_progress_is_rejected_without_advancing() {
    let f = Fixture::new();
    let mut config = common::read_config(&f.path);
    config["pipelines"][0]["steps"][0]["transitions"] = json!([]);
    common::write_config(&f.path, &config);
    let app = f.app();
    let run = f.start(&app, 50, json!([])).await;
    let id = run["id"].as_str().unwrap();
    let next = app
        .execute(WorkCommand::Next { id: id.into() })
        .await
        .unwrap();
    assert!(
        !next["allowedOutcomes"]
            .as_array()
            .unwrap()
            .contains(&json!("progress"))
    );
    assert!(
        f.submit(&app, &run, "progress")
            .await
            .unwrap_err()
            .contains("not allowed")
    );
    let current = app
        .execute(WorkCommand::Status {
            id: Some(id.into()),
        })
        .await
        .unwrap();
    assert_eq!(current["session"]["revision"], 0);
    assert_eq!(current["steps"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn configured_rework_continues_same_step_and_is_bounded() {
    let f = Fixture::new();
    let app = f.app();
    let start = f
        .start(&app, 5, json!([{"stepId":"build", "command":"false"}]))
        .await;
    let next = f
        .call(&app, json!({"command":"next", "id":start["id"]}))
        .await
        .unwrap();
    assert!(
        next["allowedOutcomes"]
            .as_array()
            .unwrap()
            .contains(&json!("revise_build"))
    );
    let first = f.submit(&app, &start, "revise_build").await.unwrap();
    assert_eq!(first["currentStep"], "build");
    assert_eq!(first["session"]["revision"], 1);
    assert_eq!(first["steps"][0]["outcome"], "revise_build");
    assert_eq!(first["steps"][1]["attempt"], 2);
    assert!(f.submit(&app, &start, "revise_build").await.is_err());
    assert!(
        f.submit(&app, &first, "completed")
            .await
            .unwrap_err()
            .contains("Configured check failed")
    );
    let second = f.submit(&app, &first, "revise_build").await.unwrap();
    let third = f.submit(&app, &second, "revise_build").await.unwrap();
    assert_eq!(third["status"], "running");
    let fourth = f.submit(&app, &third, "revise_build").await.unwrap();
    let fifth = f.submit(&app, &fourth, "revise_build").await.unwrap();
    assert_eq!(fifth["status"], "limitExhausted");
    assert_eq!(fifth["steps"].as_array().unwrap().len(), 5);
    assert!(!fifth["endedAt"].is_null());
}

#[tokio::test]
async fn next_returns_latest_handoff_without_replaying_large_traces() {
    use baton_core::{domain::pipeline::RunTraceEntry, ports::runs::RunRepository};
    let f = Fixture::new();
    let app = f.app();
    let run = f.start(&app, 50, json!([])).await;
    let run = f.submit(&app, &run, "revise_build").await.unwrap();
    let id = run["id"].as_str().unwrap();
    let repo =
        baton_adapters::repositories::filesystem_work::FilesystemWorkRepository(f.path.clone());
    let mut stored = repo.load(id).unwrap();
    stored.trace.push(RunTraceEntry {
        attempt: Some(1),
        occurred_at: "test".into(),
        kind: "test".into(),
        content: "x".repeat(17 * 1024 * 1024),
    });
    repo.save(&stored).unwrap();
    let next = app
        .execute(WorkCommand::Next { id: id.into() })
        .await
        .unwrap();
    assert_eq!(
        next["lastResult"]["output"],
        "Result and acceptance criterion verification"
    );
    assert!(serde_json::to_vec(&next).unwrap().len() < 65536);
}

#[tokio::test]
async fn explicit_rework_loop_obeys_the_overall_attempt_limit() {
    let f = Fixture::new();
    let app = f.app();
    let mut run = f.start(&app, 5, json!([])).await;
    for _ in 0..5 {
        run = f.submit(&app, &run, "revise_build").await.unwrap();
    }
    assert_eq!(run["status"], "limitExhausted");
    assert_eq!(run["steps"].as_array().unwrap().len(), 5);
}

#[tokio::test]
async fn work_no_longer_creates_or_changes_markdown_notes() {
    let f = Fixture::new();
    let app = f.app();
    let start = f.start(&app, 50, json!([])).await;
    let path = f
        .path
        .join(".baton/work")
        .join(format!("{}.md", start["id"].as_str().unwrap()));
    assert!(!path.exists());
    std::fs::write(&path, "existing notes").unwrap();
    let continued = f.submit(&app, &start, "revise_build").await.unwrap();
    assert_eq!(continued["currentStep"], "build");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "existing notes");
}

#[tokio::test]
async fn rework_can_finish_and_advance_to_review() {
    let f = Fixture::new();
    let app = f.app();
    let start = f.start(&app, 50, json!([])).await;
    let continued = f.submit(&app, &start, "revise_build").await.unwrap();
    drop(app);
    let app = f.app();
    let restored = f
        .call(&app, json!({"command":"status", "id":start["id"]}))
        .await
        .unwrap();
    assert_eq!(restored, continued);
    let review = f.submit(&app, &restored, "completed").await.unwrap();
    assert_eq!(review["currentStep"], "review");
    assert_eq!(
        review["steps"][0]["output"],
        "Result and acceptance criterion verification"
    );
}

#[tokio::test]
async fn demo_pipeline_runs_in_temporary_workspace() {
    let f = Fixture::new();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    common::write_config(&f.path, &common::read_config(&repo.join("examples/demo")));
    for skill in [
        "concise-planning",
        "ponytail",
        "verification-before-completion",
        "code-review-excellence",
    ] {
        let destination = f.path.join(".agents/skills").join(skill);
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::copy(
            repo.join(".agents/skills").join(skill).join("SKILL.md"),
            destination.join("SKILL.md"),
        )
        .unwrap();
    }
    let app = f.app();
    let start = f.start(&app, 50, json!([])).await;
    assert_eq!(start["currentStep"], "plan");
    let plan = f.submit(&app, &start, "completed").await.unwrap();
    assert_eq!(plan["currentStep"], "build");
    let build = f.submit(&app, &plan, "completed").await.unwrap();
    assert_eq!(build["currentStep"], "review");
    let review = f.submit(&app, &build, "approved").await.unwrap();
    assert_eq!(review["status"], "waitingHuman");
    assert_eq!(review["currentStep"], "approval");
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
        next["agent"]["instructions"],
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
    let mut config = common::read_config(&f.path);
    config["pipelines"][0]["steps"][0]["transitions"] = json!([
        {"outcome":"responded", "to":null},
        {"outcome":"revise_build", "to":null}
    ]);
    common::write_config(&f.path, &config);
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
    empty["result"]["summary"] = json!("");
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
        !std::fs::read_to_string(f.path.join(".baton/project.yml"))
            .unwrap()
            .contains("projectId")
    );
    drop(app);
    assert!(WorkApplication::new(f.path.clone()).is_ok());
}

#[tokio::test]
async fn next_leaves_context_with_pi_while_explicit_status_retains_decisions() {
    let f = Fixture::new();
    let mut config = common::read_config(&f.path);
    config["pipelines"][0]["steps"] = json!([
        {"id":"clarify", "name":"Clarify scope", "type":"human"},
        {"id":"plan", "name":"Plan", "type":"agent", "agentId":"worker"},
        {"id":"build", "name":"Build", "type":"agent", "agentId":"worker"},
        {"id":"review", "name":"Review", "type":"agent", "agentId":"worker"}
    ]);
    common::write_config(&f.path, &config);
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
