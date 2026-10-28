use baton_adapters::repositories::filesystem_run::FilesystemRunRepository;
use baton_core::{
    domain::{
        pipeline::*,
        project::{Agent, Outcome, ProjectConfig, parse_config},
    },
    features::{human_actions::resume_human, pipeline_engine::*},
    ports::{
        runs::RunRepository,
        runtime::{AgentDecision, AgentRuntime},
    },
};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "baton-graph-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        fs::create_dir_all(&path).unwrap();
        for args in [
            vec!["init"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-m",
                "test",
            ],
        ] {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&path)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        Self(path)
    }
    fn repo(&self) -> FilesystemRunRepository {
        FilesystemRunRepository(self.0.clone())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct Script {
    outputs: VecDeque<String>,
    directories: Vec<PathBuf>,
    decision: Option<AgentDecision>,
}
impl Script {
    fn new(outputs: &[&str]) -> Self {
        Self {
            outputs: outputs.iter().map(|s| (*s).into()).collect(),
            ..Default::default()
        }
    }
}
impl AgentRuntime for Script {
    fn execute(
        &mut self,
        directory: &Path,
        _agent: &Agent,
        _input: &str,
    ) -> Result<String, String> {
        assert!(directory.join(".git").exists());
        self.directories.push(directory.into());
        let output = self
            .outputs
            .pop_front()
            .ok_or_else(|| "Unexpected agent execution".to_owned())?;
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct FixtureDecision {
            outcome: Outcome,
            output: String,
        }
        if let Ok(decision) = serde_json::from_str::<FixtureDecision>(&output) {
            self.decision = Some(AgentDecision {
                outcome: decision.outcome,
                summary: decision.output.clone(),
            });
            Ok(decision.output)
        } else {
            Ok(output)
        }
    }
    fn take_decision(&mut self) -> Option<AgentDecision> {
        self.decision.take()
    }
}
fn config() -> ProjectConfig {
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("../../baton-core/tests/fixture_config.json")).unwrap();
    json["pipelines"][0]["steps"] = serde_json::json!([
        {"id":"plan", "name":"Plan", "agentId":"writer"},
        {"id":"build", "name":"Build", "agentId":"writer"},
        {"id":"review", "name":"Review", "agentId":"reviewer", "transitions":[
            {"outcome":"approved", "to":"human", "maxTraversals":1},
            {"outcome":"revise_build", "to":"build", "maxTraversals":1},
            {"outcome":"revise_plan", "to":"plan", "maxTraversals":1}
        ]},
        {"id":"human", "name":"Human approval", "human":true, "transitions":[
            {"outcome":"approved", "to":"finish", "maxTraversals":1},
            {"outcome":"rejected", "to":null, "maxTraversals":1},
            {"outcome":"responded", "to":"build", "maxTraversals":1}
        ]},
        {"id":"finish", "name":"Finish", "agentId":"writer"}
    ]);
    parse_config(&json.to_string()).unwrap()
}
const APPROVED: &str = r#"{"outcome":"approved","output":"Ready for a human"}"#;
const REWORK: &str = r#"{"outcome":"revise_build","output":"Fix the build"}"#;

#[test]
fn review_rework_handoffs_and_explicit_approval_survive_restart() {
    let fixture = Fixture::new();
    let mut runtime = Script::new(&["plan", "build", REWORK, "fixed", APPROVED]);
    let waiting = start_pipeline(
        &config(),
        "draft",
        "task",
        "review",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(waiting.status, Status::WaitingHuman);
    assert_eq!(waiting.current_step.as_deref(), Some("human"));
    assert_eq!(
        waiting
            .steps
            .iter()
            .filter(|s| s.step_id == "build")
            .count(),
        2
    );
    let rework = waiting
        .steps
        .iter()
        .find(|s| s.outcome == Some(Outcome::ReviseBuild))
        .unwrap();
    assert_eq!(
        rework.selected_transition.as_ref().unwrap().to.as_deref(),
        Some("build")
    );
    let second_build = waiting
        .steps
        .iter()
        .find(|s| s.step_id == "build" && s.attempt > 2)
        .unwrap();
    assert!(
        second_build
            .input
            .as_deref()
            .unwrap()
            .contains("Fix the build")
    );
    assert!(
        second_build
            .input
            .as_deref()
            .unwrap()
            .contains("Original task:")
    );
    assert_eq!(
        runtime.directories,
        vec![waiting.worktree.as_ref().unwrap().path.clone(); 5]
    );
    let repo = fixture.repo();
    repo.recover_interrupted().unwrap();
    assert_eq!(repo.load("review").unwrap(), waiting);
    assert!(resume_human("review", Outcome::ReviseBuild, "bad", &mut runtime, &repo).is_err());
    assert_eq!(repo.load("review").unwrap(), waiting);
    // A separate process reopens the durable wait, performs startup recovery,
    // and resumes only in response to an explicit human action.
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "human_restart_reader"])
        .env("BATON_GRAPH_RESTART", &fixture.0)
        .status()
        .unwrap();
    assert!(status.success());
    let complete = repo.load("review").unwrap();
    assert_eq!(complete.status, Status::Succeeded);
    assert_eq!(complete.worktree, waiting.worktree);
    assert_eq!(complete.steps.last().unwrap().step_id, "review"); // retained attempt vector is not graph order
    let final_input = complete
        .steps
        .iter()
        .find(|s| s.step_id == "finish")
        .unwrap()
        .input
        .as_deref()
        .unwrap();
    assert!(final_input.contains("Ship it"));
    assert!(final_input.contains("Original task:"));
    assert!(resume_human("review", Outcome::Approved, "again", &mut runtime, &repo).is_err());
}

#[test]
fn human_restart_reader() {
    let Some(path) = std::env::var_os("BATON_GRAPH_RESTART") else {
        return;
    };
    let repo = FilesystemRunRepository(path.into());
    repo.recover_interrupted().unwrap();
    assert_eq!(repo.load("review").unwrap().status, Status::WaitingHuman);
    let mut runtime = Script::new(&["Finished"]);
    let run = resume_human("review", Outcome::Approved, "Ship it", &mut runtime, &repo).unwrap();
    assert_eq!(run.status, Status::Succeeded);
    assert_eq!(runtime.directories.len(), 1);
}

#[test]
fn review_can_repeat_rework_without_a_traversal_limit() {
    let fixture = Fixture::new();
    let mut runtime = Script::new(&[
        "plan",
        "build",
        r#"{"outcome":"revise_plan","output":"New plan"}"#,
        "new plan",
        "new build",
        REWORK,
        "fixed",
        REWORK,
        "another build",
        REWORK,
        "final build",
        APPROVED,
    ]);
    let run = start_pipeline(
        &config(),
        "draft",
        "task",
        "limit",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::WaitingHuman);
    assert_eq!(runtime.directories.len(), 12);
    assert!(run.ended_at.is_none());
    let attempt = run.steps.last().unwrap();
    assert_eq!(attempt.outcome, Some(Outcome::Approved));
    assert!(attempt.error.is_none());
    fixture.repo().recover_interrupted().unwrap();
    assert_eq!(fixture.repo().load("limit").unwrap(), run);
}

#[test]
fn unknown_outcomes_and_unconfigured_routes_fail_without_advancing() {
    for (id, output) in [
        ("unknown", r#"{"outcome":"merge_now","output":"bad"}"#),
        ("missing", r#"{"outcome":"blocked","output":"help"}"#),
        (
            "destination",
            r#"{"outcome":"approved","output":"bad","to":"finish"}"#,
        ),
    ] {
        let fixture = Fixture::new();
        let mut runtime = Script::new(&["plan", "build", output]);
        let run = start_pipeline(
            &config(),
            "draft",
            "task",
            id,
            &mut runtime,
            &fixture.repo(),
        )
        .unwrap();
        assert_eq!(run.status, Status::Failed);
        assert_eq!(runtime.directories.len(), 3);
        assert_eq!(run.steps[2].status, Status::Failed);
        assert!(run.steps[2].error.is_some());
        assert_eq!(fixture.repo().load(id).unwrap(), run);
    }
}

#[test]
fn invalid_graphs_are_rejected_before_allocating_a_worktree() {
    for change in ["target", "duplicate", "human"] {
        let fixture = Fixture::new();
        let mut cfg = config();
        let review = &mut cfg.pipelines[0].steps[2];
        match change {
            "target" => review.transitions[0].to = Some("missing".into()),
            "duplicate" => review.transitions.push(review.transitions[0].clone()),
            "human" => cfg.pipelines[0].steps[3].transitions[0].outcome = Outcome::RevisePlan,
            _ => unreachable!(),
        }
        assert!(
            start_pipeline(
                &cfg,
                "draft",
                "task",
                "invalid",
                &mut Script::default(),
                &fixture.repo()
            )
            .is_err()
        );
        assert!(fixture.repo().list().unwrap().is_empty());
    }
}

#[test]
fn rejection_is_terminal_and_wait_cannot_resume_in_a_different_worktree() {
    let fixture = Fixture::new();
    let mut runtime = Script::new(&["plan", "build", APPROVED]);
    let waiting = start_pipeline(
        &config(),
        "draft",
        "task",
        "reject",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    let repo = fixture.repo();
    let mut tampered = waiting.clone();
    tampered.worktree.as_mut().unwrap().path = fixture.0.clone();
    repo.save(&tampered).unwrap();
    assert!(resume_human("reject", Outcome::Approved, "ok", &mut runtime, &repo).is_err());
    assert_eq!(runtime.directories.len(), 3);
    repo.save(&waiting).unwrap();
    let rejected = resume_human(
        "reject",
        Outcome::Rejected,
        "Needs manual work",
        &mut runtime,
        &repo,
    )
    .unwrap();
    assert_eq!(rejected.status, Status::Failed);
    assert_eq!(rejected.steps[3].outcome, Some(Outcome::Rejected));
    assert_eq!(runtime.directories.len(), 3);
}

#[tokio::test]
async fn human_action_api_exposes_graph_history_and_explicit_abort() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let fixture = Fixture::new();
    let mut cfg = config();
    let mut human = cfg.pipelines[0].steps[3].clone();
    human
        .transitions
        .retain(|t| t.outcome != Outcome::Responded);
    for t in &mut human.transitions {
        t.to = None;
    }
    cfg.pipelines[0].steps = vec![human];
    for id in ["approve", "abort"] {
        let run = start_pipeline(
            &cfg,
            "draft",
            "task",
            id,
            &mut Script::default(),
            &fixture.repo(),
        )
        .unwrap();
        assert_eq!(run.status, Status::WaitingHuman);
    }
    let app = baton_server::router(baton_server::AppState::new(fixture.0.clone()));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/runs/approve")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(json["currentStep"], "human");
    assert_eq!(json["status"], "waitingHuman");
    assert_eq!(json["steps"][0]["input"], "task");
    assert_eq!(
        json["configSnapshot"]["pipelines"][0]["steps"][0]["human"],
        true
    );
    for (outcome, status) in [
        ("responded", StatusCode::BAD_REQUEST),
        ("revise_plan", StatusCode::BAD_REQUEST),
        ("approved", StatusCode::OK),
        ("approved", StatusCode::CONFLICT),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/runs/approve/actions")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"outcome":"{outcome}","response":"User decision"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    assert_eq!(
        fixture.repo().load("approve").unwrap().status,
        Status::Succeeded
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runs/abort/cancel")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let aborted = fixture.repo().load("abort").unwrap();
    assert_eq!(aborted.status, Status::Cancelled);
    assert!(aborted.worktree.unwrap().path.exists());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runs/abort/actions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"outcome":"approved"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}
