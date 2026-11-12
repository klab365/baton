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
        {"id":"plan", "name":"Plan", "type":"agent", "agentId":"writer"},
        {"id":"build", "name":"Build", "type":"agent", "agentId":"writer"},
        {"id":"review", "name":"Review", "type":"agent", "agentId":"reviewer", "transitions":[
            {"outcome":"approved", "to":"human", "maxTraversals":1},
            {"outcome":"revise_build", "to":"build", "maxTraversals":1},
            {"outcome":"revise_plan", "to":"plan", "maxTraversals":1}
        ]},
        {"id":"human", "name":"Human approval", "type":"human", "transitions":[
            {"outcome":"approved", "to":"finish", "maxTraversals":1},
            {"outcome":"rejected", "to":null, "maxTraversals":1},
            {"outcome":"responded", "to":"build", "maxTraversals":1}
        ]},
        {"id":"finish", "name":"Finish", "type":"agent", "agentId":"writer"}
    ]);
    parse_config(&json.to_string()).unwrap()
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
        r#"{"outcome":"revise_build","output":"Implemented part; next verify"}"#,
        r#"{"outcome":"revise_build","output":"Second"}"#,
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
            .contains("Implemented part; next verify")
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
