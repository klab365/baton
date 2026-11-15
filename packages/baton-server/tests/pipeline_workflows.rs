use baton_adapters::repositories::{
    filesystem_run::FilesystemRunRepository, filesystem_task::FilesystemTaskRepository,
};
use baton_core::{
    domain::{
        pipeline::*,
        project::{Agent, Outcome, ProjectConfig, parse_config},
    },
    features::{human_actions::resume_human, pipeline_engine::*},
    ports::{
        runs::RunRepository,
        runtime::{AgentDecision, AgentRuntime, AgentRuntimeFactory},
        worktrees::WorktreeManager,
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
    allowed: Vec<Vec<Outcome>>,
    run_ids: Vec<String>,
    history_required: Vec<bool>,
    decision: Option<AgentDecision>,
    emit_observed_activity: bool,
    no_session: bool,
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
    fn list_models(&mut self) -> Result<Vec<baton_core::domain::runtime::RuntimeModel>, String> {
        Err("This runtime does not expose model discovery".into())
    }
    fn execute_cancellable(
        &mut self,
        directory: &Path,
        agent: &Agent,
        input: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String, String> {
        if cancelled() {
            return Err("Run cancelled".into());
        }
        self.execute(directory, agent, input)
    }
    fn take_events(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn take_trace(&mut self) -> Vec<baton_core::domain::pipeline::RunTraceEntry> {
        Vec::new()
    }
    fn supports_session_resumption(&self, _: &Agent) -> bool {
        !self.no_session
    }
    fn set_run_id(&mut self, run_id: &str) -> Result<(), String> {
        self.run_ids.push(run_id.to_owned());
        Ok(())
    }
    fn set_history_required(&mut self, required: bool) {
        self.history_required.push(required);
    }
    fn execute_shell(
        &mut self,
        directory: &Path,
        command: &str,
        timeout_seconds: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<baton_core::domain::work::CheckResult, String> {
        baton_adapters::work::run_shell(directory, command, timeout_seconds, cancelled)
    }
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
            #[serde(default)]
            final_output: Option<String>,
            #[serde(default)]
            final_output_bytes: Option<usize>,
            #[serde(default)]
            blocker_reason: Option<String>,
            #[serde(default)]
            human_action: Option<String>,
        }
        if let Ok(decision) = serde_json::from_str::<FixtureDecision>(&output) {
            let mut summary = decision.output.clone();
            if decision.outcome == Outcome::Blocked
                && let Some(reason) = decision.blocker_reason.as_deref()
            {
                summary.push_str(&format!("\n\nBlocker: {}", reason.trim()));
                if let Some(action) = decision
                    .human_action
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                {
                    summary.push_str(&format!("\nHuman action needed: {}", action.trim()));
                }
            }
            self.decision = Some(AgentDecision {
                outcome: decision.outcome,
                summary,
            });
            Ok(decision.final_output_bytes.map_or_else(
                || decision.final_output.unwrap_or(decision.output),
                |size| "x".repeat(size),
            ))
        } else {
            Ok(output)
        }
    }
    fn execute_pipeline_step(
        &mut self,
        directory: &Path,
        agent: &Agent,
        input: &str,
        allowed_outcomes: &[Outcome],
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<String, String> {
        self.allowed.push(allowed_outcomes.to_vec());
        self.execute(directory, agent, input)
    }
    fn execute_pipeline_step_observed(
        &mut self,
        directory: &Path,
        agent: &Agent,
        input: &str,
        allowed_outcomes: &[Outcome],
        cancelled: &dyn Fn() -> bool,
        observer: &mut dyn FnMut(baton_core::domain::pipeline::RunTraceEntry) -> Result<(), String>,
    ) -> Result<String, String> {
        if self.emit_observed_activity {
            observer(baton_core::domain::pipeline::RunTraceEntry {
                attempt: None,
                occurred_at: baton_core::features::pipeline_engine::utc_now(),
                kind: "runtimeRequest".into(),
                content: r#"{"messageRoles":["user"],"toolNames":[]}"#.into(),
            })?;
        }
        self.execute_pipeline_step(directory, agent, input, allowed_outcomes, cancelled)
    }
    fn take_decision(&mut self) -> Option<AgentDecision> {
        self.decision.take()
    }
}
fn config() -> ProjectConfig {
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("../../baton-core/tests/fixture_config.json")).unwrap();
    json["pipelines"][0]["steps"] = serde_json::json!([
        {"id":"plan", "name":"Plan", "type":"agent", "agentId":"writer", "includeTask":true},
        {"id":"build", "name":"Build", "type":"agent", "agentId":"writer", "includeTask":true},
        {"id":"review", "name":"Review", "type":"agent", "agentId":"reviewer", "includeTask":true, "transitions":[
            {"outcome":"approved", "to":"human", "maxTraversals":1},
            {"outcome":"revise_build", "to":"build", "maxTraversals":1},
            {"outcome":"revise_plan", "to":"plan", "maxTraversals":1}
        ]},
        {"id":"human", "name":"Human approval", "type":"human", "transitions":[
            {"outcome":"approved", "to":"finish", "maxTraversals":1},
            {"outcome":"rejected", "to":null, "maxTraversals":1},
            {"outcome":"responded", "to":"build", "maxTraversals":1}
        ]},
        {"id":"finish", "name":"Finish", "type":"agent", "agentId":"writer", "includeTask":true}
    ]);
    parse_config(&json.to_string()).unwrap()
}

fn software_factory_config() -> ProjectConfig {
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../baton-core/tests/fixture_config.json")).unwrap();
    let factory: serde_json::Value = serde_yaml_ng::from_str(include_str!(
        "../../../.baton/pipelines/software-factory.yml"
    ))
    .unwrap();
    let steps = factory["steps"].as_array().unwrap().clone();
    let template = value["agents"][0].clone();
    let mut agents = Vec::new();
    for step in &steps {
        if let Some(id) = step["agentId"].as_str()
            && !agents
                .iter()
                .any(|agent: &serde_json::Value| agent["id"] == id)
        {
            let mut agent = template.clone();
            agent["id"] = id.into();
            agents.push(agent);
        }
    }
    value["agents"] = serde_json::Value::Array(agents);
    value["pipelines"][0]["steps"] = serde_json::Value::Array(steps);
    parse_config(&value.to_string()).unwrap()
}

#[test]
fn repeated_review_findings_pause_and_resume_after_repository_reload() {
    let fixture = Fixture::new();
    let mut graph = config();
    graph.pipelines[0].steps = serde_json::from_value(serde_json::json!([
        {"id":"build", "name":"Build", "type":"agent", "agentId":"writer"},
        {"id":"review", "name":"Review", "type":"agent", "agentId":"reviewer", "transitions":[
            {"outcome":"revise_build", "to":"build"}, {"outcome":"approved", "to":null}
        ]}
    ]))
    .unwrap();
    let repo = fixture.repo();
    let mut script = Script::new(&[
        "build 1",
        r#"{"outcome":"revise_build","output":"Missing test"}"#,
        "build 2",
        r#"{"outcome":"revise_build","output":"Missing test"}"#,
        "build 3",
        r#"{"outcome":"revise_build","output":"Missing test"}"#,
    ]);
    let paused =
        start_pipeline(&graph, "draft", "task", "loop-review", &mut script, &repo).unwrap();
    assert_eq!(paused.status, Status::WaitingHuman);
    assert!(paused.ended_at.is_none());
    assert_eq!(paused.current_step.as_deref(), Some("build"));
    assert!(paused.events.iter().any(|event| {
        event
            .message
            .as_deref()
            .is_some_and(|message| message.contains("threshold 3"))
    }));
    let worktree = paused.worktree.clone();
    fixture.repo().recover_interrupted().unwrap();
    let recovered = fixture.repo().load("loop-review").unwrap();
    assert_eq!(recovered, paused);
    assert!(
        resume_human(
            "loop-review",
            Outcome::Approved,
            "",
            &mut Script::default(),
            &fixture.repo()
        )
        .is_err()
    );
    assert_eq!(fixture.repo().load("loop-review").unwrap(), paused);
    let mut continuation = Script::new(&["fixed", r#"{"outcome":"approved","output":"Verified"}"#]);
    let resumed = resume_human(
        "loop-review",
        Outcome::Responded,
        "Add the missing test",
        &mut continuation,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(resumed.status, Status::Succeeded);
    assert_eq!(resumed.worktree, worktree);
    assert!(resumed.steps.iter().any(|step| {
        step.input
            .as_deref()
            .is_some_and(|input| input.contains("Add the missing test"))
    }));
}

#[test]
fn changing_review_findings_do_not_pause_productive_iterations() {
    let fixture = Fixture::new();
    let mut graph = config();
    graph.pipelines[0].steps = serde_json::from_value(serde_json::json!([
        {"id":"build", "name":"Build", "type":"agent", "agentId":"writer"},
        {"id":"review", "name":"Review", "type":"agent", "agentId":"reviewer", "transitions":[
            {"outcome":"revise_build", "to":"build"}, {"outcome":"approved", "to":null}
        ]}
    ]))
    .unwrap();
    let mut script = Script::new(&[
        "build",
        r#"{"outcome":"revise_build","output":"Fix A"}"#,
        "build",
        r#"{"outcome":"revise_build","output":"Fix B"}"#,
        "build",
        r#"{"outcome":"revise_build","output":"Fix C"}"#,
        "build",
        r#"{"outcome":"approved","output":"Verified"}"#,
    ]);
    let run = start_pipeline(
        &graph,
        "draft",
        "task",
        "productive-review",
        &mut script,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Succeeded);
    assert!(!run.events.iter().any(|event| {
        event
            .message
            .as_deref()
            .is_some_and(|message| message.starts_with("Possible unproductive loop:"))
    }));
}

struct ResumeFactory;
impl AgentRuntimeFactory for ResumeFactory {
    fn create(&self, _: &ProjectConfig, _: &str) -> Result<Box<dyn AgentRuntime>, String> {
        let mut runtime = Script::new(&[r#"{"outcome":"approved","output":"Verified"}"#]);
        runtime.emit_observed_activity = true;
        Ok(Box::new(runtime))
    }
}

#[test]
fn interrupted_agent_resumes_in_same_worktree_with_reconciliation_warning() {
    let fixture = Fixture::new();
    let mut config = config();
    let mut step = config.pipelines[0].steps[2].clone();
    step.max_attempts = Some(2);
    step.transitions = vec![baton_core::domain::project::Transition {
        outcome: Outcome::Approved,
        to: None,
    }];
    config.pipelines[0].steps = vec![step];
    let repo = fixture.repo();
    let mut run = start_pipeline(
        &config,
        "draft",
        "task",
        "resume-agent",
        &mut Script::new(&[r#"{"outcome":"approved","output":"original"}"#]),
        &repo,
    )
    .unwrap();
    let worktree = run.worktree.clone();
    run.status = Status::Running;
    run.current_step = Some("review".into());
    run.steps[0].status = Status::Running;
    run.steps[0].output = None;
    run.steps[0].outcome = None;
    run.steps[0].selected_transition = None;
    run.handoff = Some("previous step handoff".into());
    repo.save(&run).unwrap();
    repo.recover_interrupted().unwrap();
    let tasks = FilesystemTaskRepository(fixture.0.clone());
    let resumed = baton_core::features::runs::resume_interrupted_run(
        "resume-agent",
        &repo,
        &tasks,
        &ResumeFactory,
    )
    .unwrap();
    assert_eq!(resumed.status, Status::Succeeded);
    assert_eq!(resumed.worktree, worktree);
    assert_eq!(resumed.steps[0].status, Status::Failed);
    assert_eq!(resumed.steps[1].attempt, 2);
    assert!(
        resumed.steps[1]
            .input
            .as_deref()
            .unwrap()
            .contains("previous step handoff")
    );
    assert!(resumed.trace.iter().any(|entry| entry.attempt == Some(2)));
    assert!(
        resumed.steps[1]
            .input
            .as_ref()
            .unwrap()
            .contains("do not repeat side effects blindly")
    );
    assert!(
        baton_core::features::runs::resume_interrupted_run(
            "resume-agent",
            &repo,
            &tasks,
            &ResumeFactory
        )
        .is_err()
    );
}

#[test]
fn interrupted_shell_is_not_replayed_automatically() {
    let fixture = Fixture::new();
    let mut config = config();
    let mut step = config.pipelines[0].steps[0].clone();
    step.kind = baton_core::domain::project::PipelineStepKind::Shell {
        command: "true".into(),
        timeout_seconds: None,
    };
    config.pipelines[0].steps = vec![step];
    let repo = fixture.repo();
    let mut run = start_pipeline(
        &config,
        "draft",
        "task",
        "resume-shell",
        &mut Script::default(),
        &repo,
    )
    .unwrap();
    run.status = Status::Running;
    run.current_step = Some("plan".into());
    run.steps[0].status = Status::Running;
    repo.save(&run).unwrap();
    repo.recover_interrupted().unwrap();
    let tasks = FilesystemTaskRepository(fixture.0.clone());
    let error = baton_core::features::runs::resume_interrupted_run(
        "resume-shell",
        &repo,
        &tasks,
        &ResumeFactory,
    )
    .unwrap_err();
    assert!(error.contains("manual reconciliation"));
    assert_eq!(repo.load("resume-shell").unwrap().status, Status::Failed);
}

#[test]
fn planning_blocker_human_response_returns_to_plan_with_both_contexts() {
    let fixture = Fixture::new();
    let mut graph = config();
    graph.pipelines[0].steps = vec![
        serde_json::from_value(serde_json::json!({"id":"plan","name":"Plan","type":"agent","agentId":"writer","transitions":[{"outcome":"blocked","to":"plan-help"},{"outcome":"completed","to":null}]})).unwrap(),
        serde_json::from_value(serde_json::json!({"id":"plan-help","name":"Clarify plan","type":"human","transitions":[{"outcome":"responded","to":"plan"}]})).unwrap(),
    ];
    let repository = fixture.repo();
    let waiting = start_pipeline(
        &graph,
        "draft",
        "task",
        "planning-blocked",
        &mut Script::new(&[r#"{"outcome":"blocked","output":"Need product decision","blocker_reason":"unclear acceptance"}"#]),
        &repository,
    ).unwrap();
    assert_eq!(waiting.status, Status::WaitingHuman);
    assert_eq!(waiting.current_step.as_deref(), Some("plan-help"));
    let human_index = waiting
        .steps
        .iter()
        .position(|step| step.step_id == "plan-help")
        .unwrap();
    let original = waiting.clone();
    assert!(
        resume_human(
            "planning-blocked",
            Outcome::Approved,
            "wrong action",
            &mut Script::default(),
            &repository
        )
        .is_err()
    );
    assert_eq!(repository.load("planning-blocked").unwrap(), original);
    let mut runtime = Script::new(&[r#"{"outcome":"completed","output":"Revised plan"}"#]);
    let resumed = resume_human(
        "planning-blocked",
        Outcome::Responded,
        "Use acceptance criteria A",
        &mut runtime,
        &repository,
    )
    .unwrap();
    assert_eq!(resumed.status, Status::Succeeded, "{:#?}", resumed.steps);
    let selected = resumed
        .steps
        .iter()
        .find(|step| step.step_id == "plan-help")
        .unwrap();
    assert_eq!(
        selected
            .selected_transition
            .as_ref()
            .and_then(|edge| edge.to.as_deref()),
        Some("plan")
    );
    let resumed_plan = resumed
        .steps
        .iter()
        .find(|step| {
            step.step_id == "plan"
                && step.attempt
                    > waiting
                        .steps
                        .iter()
                        .find(|step| step.step_id == "plan")
                        .unwrap()
                        .attempt
        })
        .unwrap();
    assert!(
        resumed_plan
            .input
            .as_deref()
            .unwrap()
            .contains("Use acceptance criteria A")
    );
    assert!(
        resumed_plan
            .input
            .as_deref()
            .unwrap()
            .contains("unclear acceptance")
    );
    assert_eq!(
        resumed.steps[human_index].output.as_deref(),
        Some("Use acceptance criteria A")
    );
}

#[test]
fn managed_repeated_blocker_warns_and_preserves_transition_after_reload() {
    let fixture = Fixture::new();
    let mut graph = config();
    graph.pipelines[0].steps = vec![
        serde_json::from_value(serde_json::json!({"id":"plan","name":"Plan","type":"agent","agentId":"writer","transitions":[{"outcome":"blocked","to":"help"},{"outcome":"completed","to":null}]})).unwrap(),
        serde_json::from_value(serde_json::json!({"id":"help","name":"Help","type":"human","transitions":[{"outcome":"responded","to":"plan"}]})).unwrap(),
    ];
    let repository = fixture.repo();
    let waiting = start_pipeline(
        &graph,
        "draft",
        "task",
        "repeated-blocker",
        &mut Script::new(&[
            r#"{"outcome":"blocked","output":"Need decision","blocker_reason":"unclear"}"#,
        ]),
        &repository,
    )
    .unwrap();
    assert_eq!(waiting.status, Status::WaitingHuman);
    let resumed = resume_human(
        "repeated-blocker",
        Outcome::Responded,
        "Use A",
        &mut Script::new(&[
            r#"{"outcome":"blocked","output":"Need decision","blocker_reason":"unclear"}"#,
        ]),
        &repository,
    )
    .unwrap();
    assert_eq!(resumed.status, Status::WaitingHuman);
    assert_eq!(resumed.current_step.as_deref(), Some("help"));
    let reloaded = repository.load("repeated-blocker").unwrap();
    assert!(reloaded.events.iter().any(|event| event.message.as_deref()
        == Some("Step plan repeated an identical blocker after a human response")));
    assert_eq!(
        reloaded
            .steps
            .iter()
            .filter(|step| step.step_id == "plan" && step.outcome == Some(Outcome::Blocked))
            .count(),
        2
    );
}

#[test]
fn implementation_blocker_debug_human_response_returns_to_build() {
    let fixture = Fixture::new();
    let mut graph = config();
    graph.pipelines[0].steps = vec![
        serde_json::from_value(serde_json::json!({"id":"plan","name":"Plan","type":"agent","agentId":"writer","transitions":[{"outcome":"completed","to":"build"}]})).unwrap(),
        serde_json::from_value(serde_json::json!({"id":"build","name":"Build","type":"agent","agentId":"writer","transitions":[{"outcome":"blocked","to":"debug"},{"outcome":"completed","to":null}]})).unwrap(),
        serde_json::from_value(serde_json::json!({"id":"debug","name":"Diagnose","type":"agent","agentId":"reviewer","transitions":[{"outcome":"blocked","to":"build-help"}]})).unwrap(),
        serde_json::from_value(serde_json::json!({"id":"build-help","name":"Resolve blocker","type":"human","transitions":[{"outcome":"responded","to":"build"}]})).unwrap(),
    ];
    let repository = fixture.repo();
    let waiting = start_pipeline(
        &graph, "draft", "task", "build-blocked",
        &mut Script::new(&[r#"{"outcome":"completed","output":"Plan"}"#, r#"{"outcome":"blocked","output":"Compilation fails","blocker_reason":"missing dependency"}"#, r#"{"outcome":"blocked","output":"Diagnosis needs credentials","blocker_reason":"private registry access"}"#]),
        &repository,
    ).unwrap();
    assert_eq!(waiting.status, Status::WaitingHuman);
    assert_eq!(waiting.current_step.as_deref(), Some("build-help"));
    let mut runtime = Script::new(&[r#"{"outcome":"completed","output":"Build fixed"}"#]);
    let resumed = resume_human(
        "build-blocked",
        Outcome::Responded,
        "Use approved mirror",
        &mut runtime,
        &repository,
    )
    .unwrap();
    assert_eq!(resumed.status, Status::Succeeded);
    let help = resumed
        .steps
        .iter()
        .find(|step| step.step_id == "build-help")
        .unwrap();
    assert_eq!(
        help.selected_transition
            .as_ref()
            .and_then(|edge| edge.to.as_deref()),
        Some("build")
    );
    let build = resumed
        .steps
        .iter()
        .filter(|step| step.step_id == "build")
        .max_by_key(|step| step.attempt)
        .unwrap();
    let input = build.input.as_deref().unwrap();
    assert!(input.contains("private registry access"));
    assert!(input.contains("Use approved mirror"));
}

#[test]
fn configured_human_wait_resumes_from_frozen_snapshot_after_restart() {
    let fixture = Fixture::new();
    let mut graph = config();
    graph.pipelines[0].steps = vec![
        serde_json::from_value(serde_json::json!({"id":"build","name":"Build","type":"agent","agentId":"writer","transitions":[{"outcome":"blocked","to":"build-help"},{"outcome":"completed","to":null}]})).unwrap(),
        serde_json::from_value(serde_json::json!({"id":"build-help","name":"Help","type":"human","transitions":[{"outcome":"responded","to":"build"}]})).unwrap(),
    ];
    let repository = fixture.repo();
    let waiting = start_pipeline(&graph, "draft", "task", "frozen-help", &mut Script::new(&[r#"{"outcome":"blocked","output":"Build blocked","blocker_reason":"registry unavailable"}"#]), &repository).unwrap();
    assert_eq!(waiting.status, Status::WaitingHuman);
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "configured_human_restart_reader"])
        .env("BATON_CONFIGURED_RESTART", &fixture.0)
        .status()
        .unwrap();
    assert!(status.success());
    let resumed = repository.load("frozen-help").unwrap();
    assert_eq!(resumed.status, Status::Succeeded);
    let help = resumed
        .steps
        .iter()
        .find(|step| step.step_id == "build-help")
        .unwrap();
    assert_eq!(
        help.selected_transition
            .as_ref()
            .and_then(|edge| edge.to.as_deref()),
        Some("build")
    );
    let build = resumed
        .steps
        .iter()
        .filter(|step| step.step_id == "build")
        .max_by_key(|step| step.attempt)
        .unwrap();
    assert!(
        build
            .input
            .as_deref()
            .unwrap()
            .contains("registry unavailable")
    );
    assert!(
        build
            .input
            .as_deref()
            .unwrap()
            .contains("Use cached artifact")
    );
}

#[test]
fn configured_human_restart_reader() {
    let Some(path) = std::env::var_os("BATON_CONFIGURED_RESTART") else {
        return;
    };
    let repository = FilesystemRunRepository(path.into());
    repository.recover_interrupted().unwrap();
    assert_eq!(
        repository.load("frozen-help").unwrap().status,
        Status::WaitingHuman
    );
    let mut runtime = Script::new(&[r#"{"outcome":"completed","output":"Build complete"}"#]);
    let resumed = resume_human(
        "frozen-help",
        Outcome::Responded,
        "Use cached artifact",
        &mut runtime,
        &repository,
    )
    .unwrap();
    assert_eq!(resumed.status, Status::Succeeded);
}

#[test]
fn human_resume_releases_action_lease_before_live_observation() {
    let fixture = Fixture::new();
    let mut config = config();
    let mut human = config.pipelines[0].steps[3].clone();
    human
        .transitions
        .retain(|transition| transition.outcome == Outcome::Approved);
    let finish = config.pipelines[0].steps[4].clone();
    config.pipelines[0].steps = vec![human, finish];
    let repository = fixture.repo();
    let waiting = start_pipeline(
        &config,
        "draft",
        "task",
        "human-observed-resume",
        &mut Script::default(),
        &repository,
    )
    .unwrap();
    assert_eq!(waiting.status, Status::WaitingHuman);
    let lease = repository.acquire_action_lease(&waiting.id).unwrap();
    let (config, run) = baton_core::features::human_actions::prepare_human_resume(
        &waiting.id,
        Outcome::Approved,
        "Approved",
        &repository,
    )
    .unwrap();
    drop(lease);
    let mut runtime = Script::new(&["Finished after approval"]);
    runtime.emit_observed_activity = true;
    let resumed = baton_core::features::pipeline_engine::execute_graph(
        &config,
        run,
        &mut runtime,
        &repository,
    )
    .unwrap();
    assert_eq!(resumed.status, Status::Succeeded);
    assert!(resumed.trace.iter().any(|entry| entry.attempt == Some(2)));
}

#[test]
fn cancellation_after_step_start_is_serialized_before_agent_side_effects() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    struct PauseStartRepository {
        inner: FilesystemRunRepository,
        started: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl RunRepository for PauseStartRepository {
        fn acquire_action_lease(
            &self,
            id: &str,
        ) -> Result<Box<dyn baton_core::ports::runs::RunActionLease>, String> {
            self.inner.acquire_action_lease(id)
        }
        fn worktree_manager(&self) -> Result<Box<dyn WorktreeManager>, String> {
            self.inner.worktree_manager()
        }
        fn create(&self, run: &PipelineRun) -> Result<(), String> {
            self.inner.create(run)
        }
        fn save(&self, run: &PipelineRun) -> Result<(), String> {
            self.inner.save(run)
        }
        fn load(&self, id: &str) -> Result<PipelineRun, String> {
            self.inner.load(id)
        }
        fn list(&self) -> Result<Vec<PipelineRun>, String> {
            self.inner.list()
        }
        fn publish(&self, event: &baton_core::domain::pipeline::RunEvent) -> Result<(), String> {
            if event.message.as_deref() == Some("Step plan started") {
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
            }
            self.inner.publish(event)
        }
    }
    struct WaitForCancellation(Arc<AtomicBool>);
    impl AgentRuntime for WaitForCancellation {
        fn set_run_id(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn set_history_required(&mut self, _: bool) {}
        fn execute_shell(
            &mut self,
            _: &Path,
            _: &str,
            _: u32,
            _: &dyn Fn() -> bool,
        ) -> Result<baton_core::domain::work::CheckResult, String> {
            Err("Shell execution is not configured".into())
        }
        fn list_models(
            &mut self,
        ) -> Result<Vec<baton_core::domain::runtime::RuntimeModel>, String> {
            Err("This runtime does not expose model discovery".into())
        }
        fn execute_pipeline_step(
            &mut self,
            d: &Path,
            a: &Agent,
            i: &str,
            _: &[Outcome],
            c: &dyn Fn() -> bool,
        ) -> Result<String, String> {
            self.execute_cancellable(d, a, i, c)
        }
        fn execute_pipeline_step_observed(
            &mut self,
            d: &Path,
            a: &Agent,
            i: &str,
            o: &[Outcome],
            c: &dyn Fn() -> bool,
            _: &mut dyn FnMut(RunTraceEntry) -> Result<(), String>,
        ) -> Result<String, String> {
            self.execute_pipeline_step(d, a, i, o, c)
        }
        fn take_decision(&mut self) -> Option<AgentDecision> {
            None
        }
        fn take_events(&mut self) -> Vec<String> {
            Vec::new()
        }
        fn take_trace(&mut self) -> Vec<RunTraceEntry> {
            Vec::new()
        }
        fn supports_session_resumption(&self, _: &Agent) -> bool {
            true
        }
        fn execute(&mut self, _: &Path, _: &Agent, _: &str) -> Result<String, String> {
            self.0.store(true, Ordering::SeqCst);
            Ok("unexpected side effect".into())
        }
        fn execute_cancellable(
            &mut self,
            _: &Path,
            _: &Agent,
            _: &str,
            cancelled: &dyn Fn() -> bool,
        ) -> Result<String, String> {
            while !cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err("Run cancelled".into())
        }
    }

    let fixture = Fixture::new();
    let mut config = config();
    config.pipelines[0].steps = vec![config.pipelines[0].steps[0].clone()];
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let repository = PauseStartRepository {
        inner: FilesystemRunRepository(fixture.0.clone()),
        started: started_tx,
        release: Mutex::new(release_rx),
    };
    let cancelled_side_effect = Arc::new(AtomicBool::new(false));
    let runtime_flag = cancelled_side_effect.clone();
    let run_config = config.clone();
    let execution = std::thread::spawn(move || {
        start_pipeline(
            &run_config,
            "draft",
            "task",
            "step-start-cancel",
            &mut WaitForCancellation(runtime_flag),
            &repository,
        )
    });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let cancel_repo = FilesystemRunRepository(fixture.0.clone());
    let cancellation = std::thread::spawn(move || {
        baton_core::features::runs::cancel_run(&cancel_repo, "step-start-cancel")
    });
    std::thread::sleep(std::time::Duration::from_millis(20));
    release_tx.send(()).unwrap();
    assert_eq!(
        cancellation.join().unwrap().unwrap().status,
        Status::Cancelled
    );
    let final_run = execution.join().unwrap().unwrap();
    assert_eq!(final_run.status, Status::Cancelled);
    assert!(!cancelled_side_effect.load(Ordering::SeqCst));
}

#[test]
fn cancellation_wins_over_a_non_observer_terminal_write() {
    use std::sync::mpsc;

    struct ContendedRepository {
        root: PathBuf,
        lease_contended: mpsc::Sender<()>,
    }
    impl ContendedRepository {
        fn inner(&self) -> FilesystemRunRepository {
            FilesystemRunRepository(self.root.clone())
        }
    }
    impl RunRepository for ContendedRepository {
        fn acquire_action_lease(
            &self,
            id: &str,
        ) -> Result<Box<dyn baton_core::ports::runs::RunActionLease>, String> {
            let lease = self.inner().acquire_action_lease(id);
            if lease.is_err() {
                let _ = self.lease_contended.send(());
            }
            lease
        }
        fn worktree_manager(&self) -> Result<Box<dyn WorktreeManager>, String> {
            self.inner().worktree_manager()
        }
        fn create(&self, run: &PipelineRun) -> Result<(), String> {
            self.inner().create(run)
        }
        fn save(&self, run: &PipelineRun) -> Result<(), String> {
            self.inner().save(run)
        }
        fn load(&self, id: &str) -> Result<PipelineRun, String> {
            self.inner().load(id)
        }
        fn list(&self) -> Result<Vec<PipelineRun>, String> {
            self.inner().list()
        }
        fn publish(&self, event: &baton_core::domain::pipeline::RunEvent) -> Result<(), String> {
            self.inner().publish(event)
        }
    }
    struct PausedRuntime(mpsc::Sender<()>, mpsc::Receiver<()>);
    impl AgentRuntime for PausedRuntime {
        fn set_run_id(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn set_history_required(&mut self, _: bool) {}
        fn execute_shell(
            &mut self,
            _: &Path,
            _: &str,
            _: u32,
            _: &dyn Fn() -> bool,
        ) -> Result<baton_core::domain::work::CheckResult, String> {
            Err("Shell execution is not configured".into())
        }
        fn list_models(
            &mut self,
        ) -> Result<Vec<baton_core::domain::runtime::RuntimeModel>, String> {
            Err("This runtime does not expose model discovery".into())
        }
        fn execute_cancellable(
            &mut self,
            d: &Path,
            a: &Agent,
            i: &str,
            c: &dyn Fn() -> bool,
        ) -> Result<String, String> {
            if c() {
                return Err("Run cancelled".into());
            }
            self.execute(d, a, i)
        }
        fn execute_pipeline_step(
            &mut self,
            d: &Path,
            a: &Agent,
            i: &str,
            _: &[Outcome],
            c: &dyn Fn() -> bool,
        ) -> Result<String, String> {
            self.execute_cancellable(d, a, i, c)
        }
        fn execute_pipeline_step_observed(
            &mut self,
            d: &Path,
            a: &Agent,
            i: &str,
            o: &[Outcome],
            c: &dyn Fn() -> bool,
            _: &mut dyn FnMut(RunTraceEntry) -> Result<(), String>,
        ) -> Result<String, String> {
            self.execute_pipeline_step(d, a, i, o, c)
        }
        fn take_decision(&mut self) -> Option<AgentDecision> {
            None
        }
        fn take_events(&mut self) -> Vec<String> {
            Vec::new()
        }
        fn take_trace(&mut self) -> Vec<RunTraceEntry> {
            Vec::new()
        }
        fn supports_session_resumption(&self, _: &Agent) -> bool {
            true
        }
        fn execute(&mut self, _: &Path, _: &Agent, _: &str) -> Result<String, String> {
            self.0.send(()).unwrap();
            self.1.recv().unwrap();
            Ok("Completed just as cancellation arrived".into())
        }
    }

    let fixture = Fixture::new();
    let mut cfg = config();
    cfg.pipelines[0].steps = vec![cfg.pipelines[0].steps[0].clone()];
    let base = fixture.repo();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (lease_tx, lease_rx) = mpsc::channel();
    let run_repository = ContendedRepository {
        root: fixture.0.clone(),
        lease_contended: lease_tx,
    };
    let run_config = cfg.clone();
    let execution = std::thread::spawn(move || {
        start_pipeline(
            &run_config,
            "draft",
            "task",
            "terminal-cancel-race",
            &mut PausedRuntime(entered_tx, release_rx),
            &run_repository,
        )
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let lease = base.acquire_action_lease("terminal-cancel-race").unwrap();
    release_tx.send(()).unwrap();
    lease_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let mut run = base.load("terminal-cancel-race").unwrap();
    assert_eq!(run.status, Status::Running);
    run.status = Status::Cancelled;
    run.ended_at = Some(baton_core::features::pipeline_engine::utc_now());
    run.steps[0].status = Status::Cancelled;
    base.save(&run).unwrap();
    drop(lease);
    assert_eq!(execution.join().unwrap().unwrap().status, Status::Cancelled);
    assert_eq!(
        base.load("terminal-cancel-race").unwrap().status,
        Status::Cancelled
    );
}

#[test]
fn linear_steps_reject_oversized_final_handoffs() {
    let fixture = Fixture::new();
    let mut config = config();
    config.pipelines[0].steps = vec![config.pipelines[0].steps[2].clone()];
    config.pipelines[0].steps[0].transitions.clear();
    let mut runtime = Script::new(&[&"x".repeat(16 * 1024 + 1)]);
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "linear-handoff-test",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Failed);
    assert!(
        run.steps[0]
            .error
            .as_deref()
            .unwrap()
            .contains("16384 bytes")
    );
}

#[test]
fn branching_steps_reject_empty_and_oversized_reported_handoffs() {
    for reported_handoff in [String::new(), "x".repeat(16 * 1024 + 1)] {
        let fixture = Fixture::new();
        let mut config = config();
        let mut step = config.pipelines[0].steps[2].clone();
        step.transitions = vec![baton_core::domain::project::Transition {
            outcome: Outcome::Approved,
            to: None,
        }];
        config.pipelines[0].steps = vec![step];
        let decision = serde_json::json!({"outcome":"approved", "output":reported_handoff});
        let mut runtime = Script::new(&[&decision.to_string()]);
        let run = start_pipeline(
            &config,
            "draft",
            "task",
            "invalid-handoff-test",
            &mut runtime,
            &fixture.repo(),
        )
        .unwrap();
        assert_eq!(run.status, Status::Failed);
        assert!(run.steps[0].error.as_deref().unwrap().contains("summary"));
    }
}

#[test]
fn routed_agent_rework_continues_with_handoff_before_completion() {
    let fixture = Fixture::new();
    let mut config = config();
    let mut step = config.pipelines[0].steps[2].clone();
    step.transitions = vec![
        baton_core::domain::project::Transition {
            outcome: Outcome::ReviseBuild,
            to: Some(step.id.clone()),
        },
        baton_core::domain::project::Transition {
            outcome: Outcome::Approved,
            to: None,
        },
    ];
    config.pipelines[0].steps = vec![step];
    let mut runtime = Script::new(&[
        r#"{"outcome":"revise_build","output":"Tool handoff","final_output_bytes":16385}"#,
        r#"{"outcome":"revise_build","output":"Second","final_output":""}"#,
        r#"{"outcome":"revise_build","output":"Third"}"#,
        r#"{"outcome":"revise_build","output":"Fourth"}"#,
        r#"{"outcome":"approved","output":"Verified"}"#,
    ]);
    runtime.emit_observed_activity = true;
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "progress-test",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Succeeded);
    assert_eq!(run.steps.len(), 5);
    assert_eq!(run.steps[0].output.as_deref(), Some("Tool handoff"));
    assert_eq!(run.handoff.as_deref(), Some("Verified"));
    assert_eq!(run.trace.len(), 5);
    assert!(
        run.trace
            .iter()
            .enumerate()
            .all(|(index, trace)| trace.attempt == Some(index as u32 + 1))
    );
    assert!(
        run.events
            .iter()
            .any(|event| event.message.as_deref()
                == Some("Observed runtime activity: runtimeRequest"))
    );
    let stored = fixture.repo().load("progress-test").unwrap();
    assert_eq!(stored.trace.len(), run.trace.len());
    assert_eq!(
        stored
            .trace
            .iter()
            .map(|trace| trace.attempt)
            .collect::<Vec<_>>(),
        run.trace
            .iter()
            .map(|trace| trace.attempt)
            .collect::<Vec<_>>()
    );
    assert_eq!(stored.events, run.events);
    assert_eq!(run.steps[0].outcome, Some(Outcome::ReviseBuild));
    assert!(
        run.steps[1]
            .input
            .as_ref()
            .unwrap()
            .contains("Tool handoff")
    );
    assert_eq!(runtime.directories[0], runtime.directories[1]);
    assert_eq!(runtime.run_ids, ["progress-test"]);
    assert_eq!(runtime.history_required, [false, true, true, true, true]);
    assert!(
        runtime
            .allowed
            .iter()
            .all(|allowed| allowed.contains(&Outcome::ReviseBuild))
    );
}

#[test]
fn unconfigured_progress_fails_without_automatic_retry() {
    let fixture = Fixture::new();
    let mut config = config();
    let mut step = config.pipelines[0].steps[2].clone();
    step.transitions = vec![baton_core::domain::project::Transition {
        outcome: Outcome::Approved,
        to: None,
    }];
    config.pipelines[0].steps = vec![step];
    let mut runtime = Script::new(&[r#"{"outcome":"progress","output":"next"}"#]);
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "progress-limit",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Failed);
    assert_eq!(runtime.directories.len(), 1);
    assert!(!runtime.allowed[0].contains(&Outcome::Progress));
    assert!(
        run.steps[0]
            .error
            .as_deref()
            .unwrap()
            .contains("not allowed")
    );
}

#[test]
fn missing_configured_outcome_fails_without_automatic_retry() {
    let fixture = Fixture::new();
    let mut config = config();
    let mut step = config.pipelines[0].steps[2].clone();
    step.transitions = vec![baton_core::domain::project::Transition {
        outcome: Outcome::Approved,
        to: None,
    }];
    config.pipelines[0].steps = vec![step];
    let mut runtime = Script::new(&["No structured outcome"]);
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "missing-outcome",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Failed);
    assert_eq!(runtime.directories.len(), 1);
    assert!(
        run.steps[0]
            .error
            .as_deref()
            .unwrap()
            .contains("without reporting")
    );
}

#[test]
fn shell_success_runs_in_task_worktree() {
    let fixture = Fixture::new();
    let mut config = config();
    let pipeline = &mut config.pipelines[0];
    let mut shell = pipeline.steps[0].clone();
    shell.kind = baton_core::domain::project::PipelineStepKind::Shell {
        command: "test -e .git && echo ok".into(),
        timeout_seconds: None,
    };
    shell.transitions.clear();
    pipeline.steps = vec![shell];
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "shell-success",
        &mut Script::default(),
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Succeeded);
    assert!(run.steps[0].output.as_ref().unwrap().contains("ok"));
}

#[test]
fn shell_timeout_routes_as_failure() {
    let fixture = Fixture::new();
    let mut config = config();
    let pipeline = &mut config.pipelines[0];
    let mut shell = pipeline.steps[0].clone();
    shell.kind = baton_core::domain::project::PipelineStepKind::Shell {
        command: "sleep 30".into(),
        timeout_seconds: Some(1),
    };
    shell.transitions = vec![baton_core::domain::project::Transition {
        outcome: Outcome::Failed,
        to: Some("build".into()),
    }];
    pipeline.steps = vec![shell, pipeline.steps[1].clone()];
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "shell-timeout",
        &mut Script::new(&["fixed"]),
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Succeeded);
    assert!(
        run.steps[1]
            .input
            .as_ref()
            .unwrap()
            .contains("timed out after 1 seconds")
    );
}

#[test]
fn shell_failure_routes_with_output_to_next_step() {
    let fixture = Fixture::new();
    let mut config = config();
    let pipeline = &mut config.pipelines[0];
    let mut shell = pipeline.steps[0].clone();
    shell.id = "check".into();
    shell.kind = baton_core::domain::project::PipelineStepKind::Shell {
        command: "echo diagnostic >&2; exit 7".into(),
        timeout_seconds: None,
    };
    shell.transitions = vec![baton_core::domain::project::Transition {
        outcome: Outcome::Failed,
        to: Some("build".into()),
    }];
    pipeline.steps = vec![shell, pipeline.steps[1].clone()];
    let mut runtime = Script::new(&["fixed"]);
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "shell",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Succeeded);
    assert_eq!(run.steps[0].outcome, Some(Outcome::Failed));
    assert!(
        run.steps[0]
            .output
            .as_ref()
            .unwrap()
            .contains("Exit status: 7")
    );
    assert!(run.steps[1].input.as_ref().unwrap().contains("diagnostic"));
}

#[test]
fn configured_factory_reinspection_runs_full_graph_without_duplicate_commit() {
    let fixture = Fixture::new();
    let config = software_factory_config();
    let task_id = "existing-task";
    let repository = fixture.repo();
    let worktree = repository
        .worktree_manager()
        .unwrap()
        .allocate(task_id)
        .unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&worktree.path)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let head = git(&["rev-parse", "HEAD"]);
    let branch = git(&["branch", "--show-current"]);
    let initial_outputs = [
        r#"{"outcome":"completed","output":"Inspect existing PR https://github.com/example/repo/pull/42; branch BRANCH; repo example/repo; head HEAD"}"#,
        r#"{"outcome":"completed","output":"No new implementation changes; retain branch and PR identity"}"#,
        r#"{"outcome":"approved","output":"Reviewed committed diff against base HEAD; head HEAD"}"#,
        r#"{"outcome":"completed","output":"No changes to publish; no commit or push; example/repo BRANCH PR URL HEAD"}"#,
        r#"{"outcome":"completed","output":"PENDING: required checks queued for HEAD"}"#,
    ];
    let initial_outputs: Vec<String> = initial_outputs
        .iter()
        .map(|s| s.replace("BRANCH", &branch).replace("HEAD", &head))
        .collect();
    let initial_refs: Vec<&str> = initial_outputs.iter().map(String::as_str).collect();
    let mut first_runtime = Script::new(&initial_refs);
    let first = start_pipeline_for_task_with_worktree(
        &config,
        "draft",
        "Reinspect existing PR https://github.com/example/repo/pull/42",
        "factory-original",
        TaskRunContext {
            task_id: Some(task_id.into()),
            worktree: Some(worktree.clone()),
        },
        &mut first_runtime,
        &repository,
    )
    .unwrap();
    assert_eq!(first.status, Status::Succeeded);
    assert_eq!(
        first
            .steps
            .iter()
            .filter(|s| s.attempt > 0)
            .map(|s| s.step_id.as_str())
            .collect::<Vec<_>>(),
        ["plan", "build", "review", "commit", "ci"]
    );
    let first_commit = first.steps.iter().find(|s| s.step_id == "commit").unwrap();
    let first_ci = first.steps.iter().find(|s| s.step_id == "ci").unwrap();
    assert!(
        first_commit
            .output
            .as_deref()
            .unwrap()
            .contains("no commit or push")
    );
    assert!(first_ci.output.as_deref().unwrap().contains("PENDING"));
    assert!(first_ci.input.as_deref().unwrap().contains(&head));
    let head_before = head.clone();

    let rerun_task = format!(
        "Reinspect existing PR https://github.com/example/repo/pull/42\n\nHandoffs from previous successful run {} (chronological):\n\n5. ci:\nPENDING: required checks queued for {head}",
        first.id
    );
    let reinspection_outputs = [
        r#"{"outcome":"completed","output":"Reinspect existing PR https://github.com/example/repo/pull/42; branch BRANCH; repo example/repo; head HEAD"}"#,
        r#"{"outcome":"completed","output":"No new implementation changes"}"#,
        r#"{"outcome":"approved","output":"Reviewed committed diff against base HEAD; head HEAD"}"#,
        r#"{"outcome":"completed","output":"No changes to publish; no commit or push; example/repo BRANCH PR URL HEAD"}"#,
        r#"{"outcome":"completed","output":"PASSED: required checks for HEAD"}"#,
    ];
    let reinspection_outputs: Vec<String> = reinspection_outputs
        .iter()
        .map(|s| s.replace("BRANCH", &branch).replace("HEAD", &head))
        .collect();
    let reinspection_refs: Vec<&str> = reinspection_outputs.iter().map(String::as_str).collect();
    let mut rerun_runtime = Script::new(&reinspection_refs);
    let rerun = start_pipeline_for_task_with_worktree(
        &config,
        "draft",
        &rerun_task,
        "factory-reinspection",
        TaskRunContext {
            task_id: Some(task_id.into()),
            worktree: Some(worktree.clone()),
        },
        &mut rerun_runtime,
        &repository,
    )
    .unwrap();
    assert_eq!(rerun.status, Status::Succeeded);
    assert_eq!(rerun.worktree.as_ref().unwrap().path, worktree.path);
    assert!(rerun.steps[0].input.as_deref().unwrap().contains("PENDING"));
    let rerun_commit = rerun.steps.iter().find(|s| s.step_id == "commit").unwrap();
    let rerun_ci = rerun.steps.iter().find(|s| s.step_id == "ci").unwrap();
    assert!(rerun_commit.input.as_deref().unwrap().contains(&head));
    assert!(
        rerun_commit
            .output
            .as_deref()
            .unwrap()
            .contains("no commit or push")
    );
    assert!(rerun_ci.output.as_deref().unwrap().contains("PASSED"));
    assert!(rerun_ci.input.as_deref().unwrap().contains(&head));
    let head_after = Command::new("git")
        .arg("-C")
        .arg(&worktree.path)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(head_after.status.success());
    assert_eq!(
        String::from_utf8(head_after.stdout).unwrap().trim(),
        head_before
    );
    let status = Command::new("git")
        .arg("-C")
        .arg(&worktree.path)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(status.stdout.is_empty());
}

fn run_configured_factory_partial_publication(remote_branch_exists: bool) {
    let fixture = Fixture::new();
    let config = software_factory_config();
    let repository = fixture.repo();
    let task_id = "partial-publication-task";
    let manager = repository.worktree_manager().unwrap();
    let worktree = manager.allocate(task_id).unwrap();
    let bare_remote = fixture.0.join("origin.git");
    assert!(
        Command::new("git")
            .args(["init", "--bare"])
            .arg(&bare_remote)
            .output()
            .unwrap()
            .status
            .success()
    );
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&worktree.path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let target_base = git(&["rev-parse", "HEAD"]);
    for n in 1..=2 {
        let path = format!("prior-{n}.txt");
        fs::write(worktree.path.join(&path), format!("prior {n}\n")).unwrap();
        git(&["add", &path]);
        let prior = Command::new("git")
            .arg("-C")
            .arg(&worktree.path)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                &format!("feat: prior task change {n}"),
            ])
            .output()
            .unwrap();
        assert!(
            prior.status.success(),
            "{}",
            String::from_utf8_lossy(&prior.stderr)
        );
    }
    let precommit_head = git(&["rev-parse", "HEAD"]);
    assert_ne!(target_base, precommit_head);
    git(&["remote", "add", "origin", bare_remote.to_str().unwrap()]);
    if remote_branch_exists {
        git(&["push", "origin", "HEAD:refs/heads/task-reinspect"]);
    }
    fs::write(worktree.path.join("reviewed.txt"), "reviewed change\n").unwrap();
    git(&["add", "reviewed.txt"]);
    let commit = Command::new("git")
        .arg("-C")
        .arg(&worktree.path)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-m",
            "fix: add reviewed change",
        ])
        .output()
        .unwrap();
    assert!(
        commit.status.success(),
        "{}",
        String::from_utf8_lossy(&commit.stderr)
    );
    let commit_sha = git(&["rev-parse", "HEAD"]);
    assert_eq!(git(&["rev-parse", "HEAD^"]), precommit_head);
    let patch = Command::new("git")
        .arg("-C")
        .arg(&worktree.path)
        .args(["diff", "--binary", &precommit_head, &commit_sha])
        .output()
        .unwrap();
    assert!(patch.status.success());
    let patch_file = fixture.0.join("reviewed.patch");
    fs::write(&patch_file, patch.stdout).unwrap();
    let patch_fingerprint = git(&["hash-object", patch_file.to_str().unwrap()]);
    fs::remove_file(patch_file).unwrap();
    let remote_expected = if remote_branch_exists {
        precommit_head.as_str()
    } else {
        "ABSENT"
    };
    let pr_url = if remote_branch_exists {
        "https://github.com/example/repo/pull/42"
    } else {
        "ABSENT"
    };
    let review_facts = format!(
        "repository example/repo; branch task-reinspect; PR URL {pr_url}; target branch base SHA {target_base}; pre-commit task HEAD SHA {precommit_head}; reviewed paths prior-1.txt, prior-2.txt, reviewed.txt; incremental reviewed patch fingerprint {patch_fingerprint}; expected remote task-branch SHA {remote_expected}"
    );
    let first_outputs = [
        r#"{"outcome":"completed","output":"Reinspect repository example/repo branch task-reinspect"}"#.to_owned(),
        r#"{"outcome":"completed","output":"No new changes; existing reviewed commit present"}"#.to_owned(),
        serde_json::json!({"outcome":"approved","output":review_facts}).to_string(),
        serde_json::json!({
            "outcome":"blocked",
            "output":format!("Publication failed after local commit {commit_sha} with parent {precommit_head}; remote remains {remote_expected}; push failed; {review_facts}"),
            "blocker_reason":"Retry publishing the exact reviewed commit"
        }).to_string(),
    ];
    let first_refs: Vec<&str> = first_outputs.iter().map(String::as_str).collect();
    let mut first_runtime = Script::new(&first_refs);
    let waiting = start_pipeline_for_task_with_worktree(
        &config,
        "draft",
        if remote_branch_exists {
            "Reinspect existing PR https://github.com/example/repo/pull/42"
        } else {
            "Publish reviewed task branch task-reinspect; no PR exists yet"
        },
        "partial-publication",
        TaskRunContext {
            task_id: Some(task_id.into()),
            worktree: Some(worktree.clone()),
        },
        &mut first_runtime,
        &repository,
    )
    .unwrap();
    assert_eq!(waiting.status, Status::WaitingHuman);
    assert_eq!(waiting.current_step.as_deref(), Some("commit-help"));
    let blocked_commit = waiting
        .steps
        .iter()
        .find(|s| s.step_id == "commit" && s.outcome == Some(Outcome::Blocked))
        .unwrap();
    assert!(
        blocked_commit
            .output
            .as_deref()
            .unwrap()
            .contains(&commit_sha)
    );
    assert!(
        blocked_commit
            .output
            .as_deref()
            .unwrap()
            .contains(&target_base)
    );
    assert!(
        blocked_commit
            .output
            .as_deref()
            .unwrap()
            .contains(&precommit_head)
    );
    assert!(
        blocked_commit
            .output
            .as_deref()
            .unwrap()
            .contains(&patch_fingerprint)
    );
    assert!(
        blocked_commit
            .output
            .as_deref()
            .unwrap()
            .contains(remote_expected)
    );
    let remote_before = Command::new("git")
        .arg("--git-dir")
        .arg(&bare_remote)
        .args(["rev-parse", "refs/heads/task-reinspect"])
        .output()
        .unwrap();
    if remote_branch_exists {
        assert!(remote_before.status.success());
        assert_eq!(
            String::from_utf8(remote_before.stdout).unwrap().trim(),
            precommit_head
        );
    } else {
        assert!(!remote_before.status.success());
    }

    // Simulate the missing publication operation; the existing local commit is the
    // reviewed change and must not be recreated during the committer retry.
    git(&["push", "origin", "HEAD:refs/heads/task-reinspect"]);
    let retry_outputs = [
        r#"{"outcome":"completed","output":"Verified existing commit; no new commit; repository example/repo branch task-reinspect PR https://github.com/example/repo/pull/42 head COMMIT_SHA"}"#
            .replace("COMMIT_SHA", &commit_sha),
        r#"{"outcome":"completed","output":"PENDING: required check queued for reviewed commit"}"#.to_owned(),
    ];
    let retry_refs: Vec<&str> = retry_outputs.iter().map(String::as_str).collect();
    let mut retry_runtime = Script::new(&retry_refs);
    let completed = resume_human(
        &waiting.id,
        Outcome::Responded,
        "Retry non-force publication of the existing reviewed commit",
        &mut retry_runtime,
        &repository,
    )
    .unwrap();
    assert_eq!(completed.status, Status::Succeeded);
    let commit_attempts: Vec<_> = completed
        .steps
        .iter()
        .filter(|s| s.step_id == "commit")
        .collect();
    assert_eq!(commit_attempts.len(), 2);
    let retry_input = commit_attempts[1].input.as_deref().unwrap();
    for fact in [
        &target_base,
        &precommit_head,
        &patch_fingerprint,
        remote_expected,
        &commit_sha,
        "reviewed paths prior-1.txt, prior-2.txt, reviewed.txt",
    ] {
        assert!(
            retry_input.contains(fact),
            "missing commit-help context: {fact}"
        );
    }
    assert!(
        commit_attempts[1]
            .input
            .as_deref()
            .unwrap()
            .contains("Retry non-force publication")
    );
    assert!(
        commit_attempts[1]
            .output
            .as_deref()
            .unwrap()
            .contains("no new commit")
    );
    let ci_input = completed
        .steps
        .iter()
        .find(|s| s.step_id == "ci")
        .unwrap()
        .input
        .as_deref()
        .unwrap();
    assert!(ci_input.contains(&commit_sha));
    assert!(ci_input.contains("https://github.com/example/repo/pull/42"));
    assert_eq!(git(&["rev-parse", "HEAD"]), commit_sha);
    assert_eq!(git(&["rev-parse", "HEAD^"]), precommit_head);
    assert_eq!(
        git(&["rev-list", "--count", &format!("{target_base}..HEAD")]),
        "3"
    );
    let remote_sha = Command::new("git")
        .arg("--git-dir")
        .arg(&bare_remote)
        .args(["rev-parse", "refs/heads/task-reinspect"])
        .output()
        .unwrap();
    assert!(remote_sha.status.success());
    assert_eq!(
        String::from_utf8(remote_sha.stdout).unwrap().trim(),
        commit_sha
    );
}

#[test]
fn configured_factory_recovers_partial_publish_for_existing_and_absent_remote_branches() {
    run_configured_factory_partial_publication(true);
    run_configured_factory_partial_publication(false);
}

#[test]
fn configured_factory_ci_outcome_scenarios_route_through_real_graph() {
    // Scripted outcomes verify configured transitions, not GitHub classification.
    for (name, ci_outcome, ci_detail) in [
        ("pending", "completed", "PENDING: check queued; head abc123"),
        (
            "failure",
            "revise_build",
            "required check failed; head abc123",
        ),
        (
            "passed",
            "completed",
            "PASSED: all required checks; head abc123",
        ),
        (
            "api",
            "blocked",
            "GitHub API unauthorized; head/details unavailable",
        ),
    ] {
        let fixture = Fixture::new();
        let config = software_factory_config();
        let mut outputs = vec![
            r#"{"outcome":"completed","output":"plan existing PR repo/branch head abc123"}"#.to_owned(),
            r#"{"outcome":"completed","output":"No unnecessary edits"}"#.to_owned(),
            r#"{"outcome":"approved","output":"Reviewed existing diff against base for head abc123"}"#.to_owned(),
            r#"{"outcome":"completed","output":"Existing PR https://github.com/example/repo/pull/42; no commit or push; head abc123"}"#.to_owned(),
            serde_json::json!({"outcome":ci_outcome,"output":ci_detail,"blocker_reason":"API inspection unavailable"}).to_string(),
        ];
        if name == "failure" {
            outputs.extend([
                r#"{"outcome":"completed","output":"Fixed actionable failure"}"#.into(),
                r#"{"outcome":"approved","output":"Reviewed fix against base; head abc123"}"#
                    .into(),
                r#"{"outcome":"completed","output":"Updated existing PR; head def456"}"#.into(),
                r#"{"outcome":"completed","output":"PENDING: other check queued for head def456"}"#
                    .into(),
            ]);
        }
        let output_refs: Vec<&str> = outputs.iter().map(String::as_str).collect();
        let mut runtime = Script::new(&output_refs);
        let run = start_pipeline(
            &config,
            "draft",
            "Reinspect existing PR https://github.com/example/repo/pull/42",
            name,
            &mut runtime,
            &fixture.repo(),
        )
        .unwrap();
        assert_eq!(
            run.status,
            if name == "api" {
                Status::WaitingHuman
            } else {
                Status::Succeeded
            },
            "{name}"
        );
        assert!(runtime.outputs.is_empty(), "{name}");
        let ci_attempts: Vec<_> = run
            .steps
            .iter()
            .filter(|s| s.step_id == "ci" && s.attempt > 0)
            .collect();
        assert!(
            ci_attempts[0]
                .output
                .as_deref()
                .unwrap()
                .contains(ci_detail),
            "{name}"
        );
        assert!(ci_attempts[0].input.as_deref().unwrap().contains("abc123"));
        if name == "failure" {
            assert_eq!(ci_attempts.len(), 2);
            assert_eq!(
                ci_attempts[0]
                    .selected_transition
                    .as_ref()
                    .unwrap()
                    .to
                    .as_deref(),
                Some("build")
            );
            assert_eq!(ci_attempts[1].outcome, Some(Outcome::Completed));
            assert!(
                run.steps
                    .iter()
                    .any(|s| s.step_id == "build" && s.attempt == 2)
            );
        } else if name == "api" {
            assert_eq!(run.current_step.as_deref(), Some("ci-help"));
            assert!(
                ci_attempts[0]
                    .output
                    .as_deref()
                    .unwrap()
                    .contains("details unavailable")
            );
        } else {
            assert_eq!(ci_attempts.len(), 1);
            assert_eq!(
                ci_attempts[0].selected_transition.as_ref().unwrap().to,
                None
            );
        }
    }
}

#[test]
fn blocked_handoffs_route_to_agent_and_human_and_are_persisted() {
    for (destination, action, expected) in [
        (
            "agent",
            None,
            "Cannot access service\n\nBlocker: Missing credentials",
        ),
        (
            "human",
            Some("Grant access"),
            "Cannot access service\n\nBlocker: Missing credentials\nHuman action needed: Grant access",
        ),
    ] {
        let fixture = Fixture::new();
        let mut json: serde_json::Value =
            serde_json::from_str(include_str!("../../baton-core/tests/fixture_config.json"))
                .unwrap();
        json["pipelines"][0]["steps"] = if destination == "agent" {
            serde_json::json!([
                {"id":"build", "name":"Build", "type":"agent", "agentId":"writer", "transitions":[{"outcome":"blocked", "to":"assist"}]},
                {"id":"assist", "name":"Assist", "type":"agent", "agentId":"reviewer", "transitions":[{"outcome":"completed", "to":null}]}
            ])
        } else {
            serde_json::json!([
                {"id":"build", "name":"Build", "type":"agent", "agentId":"writer", "transitions":[{"outcome":"blocked", "to":"assist"}]},
                {"id":"assist", "name":"Human assistance", "type":"human"}
            ])
        };
        let config = parse_config(&json.to_string()).unwrap();
        let outcome = serde_json::json!({"outcome":"blocked", "output":"Cannot access service", "blocker_reason":"Missing credentials", "human_action":action});
        let mut runtime = if destination == "agent" {
            Script::new(&[
                &outcome.to_string(),
                r#"{"outcome":"completed","output":"Resolved"}"#,
            ])
        } else {
            Script::new(&[&outcome.to_string()])
        };
        let run = start_pipeline(
            &config,
            "draft",
            "task",
            destination,
            &mut runtime,
            &fixture.repo(),
        )
        .unwrap();
        assert_eq!(
            run.status,
            if destination == "agent" {
                Status::Succeeded
            } else {
                Status::WaitingHuman
            }
        );
        let stored = fixture.repo().load(destination).unwrap();
        assert_eq!(stored, run);
        assert_eq!(run.steps[0].output.as_deref(), Some(expected));
        assert!(run.steps[1].input.as_deref().unwrap().contains(expected));
    }
}

#[test]
fn debugger_receives_blocker_and_returns_diagnosis_to_builder() {
    let fixture = Fixture::new();
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("../../baton-core/tests/fixture_config.json")).unwrap();
    json["pipelines"][0]["steps"] = serde_json::json!([
        {"id":"build", "name":"Build", "type":"agent", "agentId":"writer", "includeTask":false, "transitions":[
            {"outcome":"blocked", "to":"debug"}, {"outcome":"completed", "to":null}
        ]},
        {"id":"debug", "name":"Debug", "type":"agent", "agentId":"reviewer", "transitions":[
            {"outcome":"revise_build", "to":"build"}
        ]}
    ]);
    let config = parse_config(&json.to_string()).unwrap();
    let mut runtime = Script::new(&[
        r#"{"outcome":"blocked","output":"specific blocker"}"#,
        r#"{"outcome":"revise_build","output":"specific diagnosis"}"#,
        r#"{"outcome":"completed","output":"fixed"}"#,
    ]);
    let run = start_pipeline(
        &config,
        "draft",
        "original task",
        "debugger",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Succeeded);
    let debug = run
        .steps
        .iter()
        .find(|step| step.step_id == "debug")
        .unwrap();
    assert!(debug.input.as_deref().unwrap().contains("specific blocker"));
    let build = run
        .steps
        .iter()
        .find(|step| step.step_id == "build" && step.attempt == 3)
        .unwrap();
    let input = build.input.as_deref().unwrap();
    assert!(input.contains("specific diagnosis"));
    assert!(!input.contains("specific blocker"));
    assert!(!input.contains("original task"));
    assert_eq!(fixture.repo().load("debugger").unwrap(), run);
}

const APPROVED: &str = r#"{"outcome":"approved","output":"Ready for a human"}"#;
const REWORK: &str = r#"{"outcome":"revise_build","output":"Fix the build"}"#;

#[test]
fn review_rework_handoffs_and_explicit_approval_survive_restart() {
    let fixture = Fixture::new();
    let mut runtime = Script::new(&["plan", "build", REWORK, "fixed", APPROVED]);
    let mut graph = config();
    if let baton_core::domain::project::PipelineStepKind::Agent { include_task, .. } =
        &mut graph.pipelines[0].steps[1].kind
    {
        *include_task = Some(false);
    }
    let waiting = start_pipeline(
        &graph,
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
        !second_build
            .input
            .as_deref()
            .unwrap()
            .contains("Original task:")
    );
    let first_build = waiting
        .steps
        .iter()
        .find(|s| s.step_id == "build" && s.attempt == 2)
        .unwrap();
    assert!(first_build.input.as_deref().unwrap().contains("plan"));
    assert!(
        !first_build
            .input
            .as_deref()
            .unwrap()
            .contains("Original task:")
    );
    let review = waiting
        .steps
        .iter()
        .find(|s| s.step_id == "review" && s.attempt == 3)
        .unwrap();
    assert!(review.input.as_deref().unwrap().contains("build"));
    assert!(review.input.as_deref().unwrap().contains("Original task:"));
    assert!(!review.input.as_deref().unwrap().contains("plan"));
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
fn repeated_agent_fails_when_runtime_cannot_resume() {
    let fixture = Fixture::new();
    let mut config = config();
    let mut step = config.pipelines[0].steps[2].clone();
    step.transitions = vec![baton_core::domain::project::Transition {
        outcome: Outcome::ReviseBuild,
        to: Some(step.id.clone()),
    }];
    config.pipelines[0].steps = vec![step];
    let mut runtime = Script::new(&[r#"{"outcome":"revise_build","output":"first"}"#]);
    runtime.no_session = true;
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "no-session",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Failed);
    assert_eq!(runtime.directories.len(), 1);
    assert!(
        run.steps[1]
            .error
            .as_deref()
            .unwrap()
            .contains("does not support resuming")
    );
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
        r#"{"outcome":"revise_build","output":"Fix first issue"}"#,
        "fixed",
        r#"{"outcome":"revise_build","output":"Fix second issue"}"#,
        "another build",
        r#"{"outcome":"revise_build","output":"Fix third issue"}"#,
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
fn global_attempt_limit_counts_human_visits_across_resume_and_freezes_snapshot() {
    let fixture = Fixture::new();
    let mut cfg = config();
    let mut human = cfg.pipelines[0].steps[3].clone();
    human.transitions = vec![baton_core::domain::project::Transition {
        outcome: Outcome::Responded,
        to: Some("human".into()),
    }];
    cfg.pipelines[0].steps = vec![human];
    cfg.pipelines[0].max_attempts = Some(1);
    let mut runtime = Script::new(&[]);
    let waiting = start_pipeline(
        &cfg,
        "draft",
        "task",
        "global-human",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(waiting.status, Status::WaitingHuman);
    assert_eq!(
        waiting.config_snapshot.as_ref().unwrap().pipelines[0].max_attempts,
        Some(1)
    );
    let resumed = resume_human(
        "global-human",
        Outcome::Responded,
        "again",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(resumed.status, Status::LimitExhausted);
    assert!(resumed.events.iter().any(|event| {
        event
            .message
            .as_deref()
            .is_some_and(|m| m == "Pipeline exhausted maxAttempts limit of 1")
    }));
    assert_eq!(
        resumed
            .steps
            .iter()
            .filter(|step| step.status != Status::Pending)
            .count(),
        1
    );
}

#[test]
fn legacy_managed_snapshot_without_global_limit_remains_unlimited_after_resume() {
    let fixture = Fixture::new();
    let mut config = config();
    let mut step = config.pipelines[0].steps[2].clone();
    step.transitions = vec![];
    config.pipelines[0].steps = vec![step];
    let mut snapshot = serde_json::to_value(&config).unwrap();
    snapshot["pipelines"][0]
        .as_object_mut()
        .unwrap()
        .remove("maxAttempts");
    let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
        "id":"legacy-unlimited", "pipelineId":"draft", "task":"task", "startedAt":"2026-01-01T00:00:00Z",
        "status":"failed", "currentStep":"review", "steps":[], "configSnapshot":snapshot,
        "worktree":{"owner":"legacy-unlimited","path":"/tmp/legacy","branch":"task/legacy","baseRevision":"abc","lifecycle":"active","cleanupOutcome":"retained"},
        "events":[{"runId":"legacy-unlimited","status":"failed","occurredAt":"2026-01-01T00:00:00Z","message":"Run interrupted by server restart"}]
    })).unwrap();
    run.steps = (1..=50)
        .map(|attempt| StepRun {
            step_id: "plan".into(),
            agent_id: "writer".into(),
            status: Status::Failed,
            input: None,
            output: None,
            error: Some("interrupted".into()),
            outcome: None,
            selected_transition: None,
            attempt,
            started_at: None,
            ended_at: None,
            duration_ms: None,
            model: None,
            usage: None,
        })
        .collect();
    let repo = fixture.repo();
    let worktree = repo
        .worktree_manager()
        .unwrap()
        .allocate("legacy-unlimited")
        .unwrap();
    run.worktree = Some(worktree);
    run.status = Status::Running;
    run.current_step = Some("review".into());
    run.steps.last_mut().unwrap().status = Status::Running;
    repo.create(&run).unwrap();
    repo.recover_interrupted().unwrap();
    let tasks = FilesystemTaskRepository(fixture.0.clone());
    let resumed = baton_core::features::runs::resume_interrupted_run(
        "legacy-unlimited",
        &repo,
        &tasks,
        &ResumeFactory,
    )
    .unwrap();
    assert_ne!(
        resumed.status,
        Status::LimitExhausted,
        "{}",
        resumed
            .events
            .last()
            .and_then(|e| e.message.as_deref())
            .unwrap_or("no event")
    );
    assert_eq!(resumed.steps.len(), 51);
    assert_eq!(resumed.steps.last().unwrap().status, Status::Succeeded);
    assert_eq!(
        resumed.steps.last().unwrap().output.as_deref(),
        Some("Verified")
    );
    let persisted = repo.load("legacy-unlimited").unwrap();
    assert_eq!(persisted.steps.len(), 51);
    assert_eq!(persisted.steps.last().unwrap().status, Status::Succeeded);
    assert_eq!(
        persisted.config_snapshot.as_ref().unwrap().pipelines[0].max_attempts,
        None
    );
}

#[test]
fn per_step_attempt_limit_stops_before_the_next_runtime_call_and_persists_reason() {
    let fixture = Fixture::new();
    let mut config = config();
    let mut step = config.pipelines[0].steps[2].clone();
    step.id = "review".into();
    step.max_attempts = Some(2);
    step.transitions = vec![baton_core::domain::project::Transition {
        outcome: Outcome::ReviseBuild,
        to: Some("review".into()),
    }];
    config.pipelines[0].steps = vec![step];
    let mut runtime = Script::new(&[
        r#"{"outcome":"revise_build","output":"again"}"#,
        r#"{"outcome":"revise_build","output":"again"}"#,
        r#"{"outcome":"revise_build","output":"unexpected third call"}"#,
    ]);
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "per-step-limit",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(runtime.directories.len(), 2);
    assert_eq!(run.status, Status::LimitExhausted);
    assert!(run.events.iter().any(|event| {
        event
            .message
            .as_deref()
            .is_some_and(|message| message.contains("review") && message.contains("2"))
    }));
    assert_eq!(fixture.repo().load("per-step-limit").unwrap(), run);
}

#[test]
fn per_step_limits_are_independent_and_reset_for_each_run() {
    let fixture = Fixture::new();
    let mut config = config();
    let mut first = config.pipelines[0].steps[2].clone();
    first.id = "first".into();
    first.max_attempts = Some(1);
    first.transitions = vec![baton_core::domain::project::Transition {
        outcome: Outcome::Approved,
        to: Some("second".into()),
    }];
    let mut second = first.clone();
    second.id = "second".into();
    second.transitions.clear();
    config.pipelines[0].steps = vec![first, second];
    for id in ["independent-steps-one", "independent-steps-two"] {
        let mut runtime = Script::new(&[
            r#"{"outcome":"approved","output":"first complete"}"#,
            "second complete",
        ]);
        let run =
            start_pipeline(&config, "draft", "task", id, &mut runtime, &fixture.repo()).unwrap();
        assert_eq!(run.status, Status::Succeeded);
        assert_eq!(runtime.directories.len(), 2);
        assert_eq!(
            run.steps
                .iter()
                .filter(|step| step.step_id == "first")
                .count(),
            1
        );
        assert_eq!(
            run.steps
                .iter()
                .filter(|step| step.step_id == "second")
                .count(),
            1
        );
        assert_eq!(fixture.repo().load(id).unwrap(), run);
    }
}

#[test]
fn human_pause_and_resume_retain_the_step_attempt_count() {
    let fixture = Fixture::new();
    let mut cfg = config();
    let mut human = cfg.pipelines[0].steps[3].clone();
    human.max_attempts = Some(1);
    human.transitions = vec![baton_core::domain::project::Transition {
        outcome: Outcome::Responded,
        to: Some("human".into()),
    }];
    cfg.pipelines[0].steps = vec![human];
    let mut runtime = Script::new(&[]);
    let waiting = start_pipeline(
        &cfg,
        "draft",
        "task",
        "human-limit",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(waiting.status, Status::WaitingHuman);
    let resumed = resume_human(
        "human-limit",
        Outcome::Responded,
        "continue",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(resumed.status, Status::LimitExhausted);
    assert_eq!(
        resumed
            .steps
            .iter()
            .filter(|step| step.step_id == "human" && step.status != Status::Pending)
            .count(),
        1
    );
    assert!(resumed.events.iter().any(|event| {
        event
            .message
            .as_deref()
            .is_some_and(|message| message.contains("human") && message.contains("1"))
    }));
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
        json["configSnapshot"]["pipelines"][0]["steps"][0]["type"],
        "human"
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
