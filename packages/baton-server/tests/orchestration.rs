use baton_adapters::repositories::filesystem_run::FilesystemRunRepository;
use baton_adapters::runtime::FakeRuntime;
use baton_core::{
    domain::{
        pipeline::*,
        project::{Agent, Outcome, parse_config},
        runtime::RuntimeModel,
        work::CheckResult,
    },
    features::pipeline_engine::*,
    ports::{
        runs::RunRepository,
        runtime::{AgentDecision, AgentRuntime, MAX_HANDOFF_BYTES},
    },
};
use baton_server::AppState;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let fixture = Self(std::env::temp_dir().join(format!(
            "baton-run-{}-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )));
        fs::create_dir_all(&fixture.0).unwrap();
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
                "initial",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&fixture.0)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        fixture
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
fn config() -> baton_core::domain::project::ProjectConfig {
    // Keep legacy sequential orchestration assertions independent from the
    // interactive demo's intentionally conditional workflow.
    let mut config =
        parse_config(include_str!("../../baton-core/tests/fixture_config.json")).unwrap();
    let pipeline = &mut config.pipelines[0];
    pipeline
        .steps
        .retain(|step| step.id == "write" || step.id == "review");
    for step in &mut pipeline.steps {
        step.transitions.clear();
        if let baton_core::domain::project::PipelineStepKind::Agent { include_task, .. } =
            &mut step.kind
        {
            *include_task = Some(true);
        }
    }
    config
}

#[test]
fn branching_handoff_is_the_complete_response_not_the_outcome_summary() {
    use baton_core::{
        domain::project::{Outcome, Transition},
        ports::runtime::AgentDecision,
    };
    struct Runtime {
        calls: usize,
    }
    impl AgentRuntime for Runtime {
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
        ) -> Result<CheckResult, String> {
            Err("Shell execution is not configured".into())
        }
        fn list_models(&mut self) -> Result<Vec<RuntimeModel>, String> {
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
        fn take_events(&mut self) -> Vec<String> {
            Vec::new()
        }
        fn take_trace(&mut self) -> Vec<RunTraceEntry> {
            Vec::new()
        }
        fn supports_session_resumption(&self, _: &baton_core::domain::project::Agent) -> bool {
            true
        }
        fn execute(
            &mut self,
            _: &std::path::Path,
            _: &baton_core::domain::project::Agent,
            input: &str,
        ) -> Result<String, String> {
            self.calls += 1;
            if self.calls == 2 {
                assert!(input.contains("short summary"));
                assert!(!input.contains("complete detailed result"));
            }
            Ok("complete detailed result".into())
        }
        fn take_decision(&mut self) -> Option<AgentDecision> {
            Some(AgentDecision {
                outcome: Outcome::Completed,
                summary: "short summary".into(),
            })
        }
    }
    let fixture = Fixture::new();
    let mut config = config();
    config.pipelines[0].steps[0].transitions = vec![Transition {
        outcome: Outcome::Completed,
        to: Some("review".into()),
    }];
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "detailed",
        &mut Runtime { calls: 0 },
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Succeeded);
    assert_eq!(
        run.steps[0].output.as_deref(),
        Some("short summary")
    );
    assert_eq!(
        fixture.repo().load("detailed").unwrap().steps[0].output,
        run.steps[0].output
    );
}

#[test]
fn empty_and_oversized_agent_handoffs_fail_without_routing() {
    struct Runtime(String);
    impl AgentRuntime for Runtime {
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
        ) -> Result<CheckResult, String> {
            Err("Shell execution is not configured".into())
        }
        fn list_models(&mut self) -> Result<Vec<RuntimeModel>, String> {
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
        fn supports_session_resumption(&self, _: &baton_core::domain::project::Agent) -> bool {
            true
        }
        fn execute(
            &mut self,
            _: &std::path::Path,
            _: &baton_core::domain::project::Agent,
            _: &str,
        ) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }
    for (id, output) in [
        ("empty", String::new()),
        ("oversized", "x".repeat(MAX_HANDOFF_BYTES + 1)),
    ] {
        let fixture = Fixture::new();
        let run = start_pipeline(
            &config(),
            "draft",
            "task",
            id,
            &mut Runtime(output),
            &fixture.repo(),
        )
        .unwrap();
        assert_eq!(run.status, Status::Failed);
        assert_eq!(run.steps[0].status, Status::Failed);
        assert!(run.steps[0].error.is_some());
        assert!(run.steps[0].output.is_none());
    }
}

#[test]
fn assigned_skill_ids_are_forwarded_to_runtime_without_inline_context() {
    let fixture = Fixture::new();
    let mut config = config();
    config.agents[0].skill_ids.push("assigned".into());
    struct Inspect;
    impl AgentRuntime for Inspect {
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
        ) -> Result<CheckResult, String> {
            Err("Shell execution is not configured".into())
        }
        fn list_models(&mut self) -> Result<Vec<RuntimeModel>, String> {
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
        fn supports_session_resumption(&self, _: &baton_core::domain::project::Agent) -> bool {
            true
        }
        fn execute(
            &mut self,
            _: &std::path::Path,
            agent: &baton_core::domain::project::Agent,
            _: &str,
        ) -> Result<String, String> {
            if agent.id == "writer" {
                assert_eq!(agent.skill_ids, ["assigned"]);
            } else {
                assert!(agent.skill_ids.is_empty());
            }
            assert!(!agent.instructions.contains("must never reach runtime"));
            assert!(!agent.instructions.contains("authorized skill body"));
            Ok("done".into())
        }
    }
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "skills",
        &mut Inspect,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Succeeded);
    assert_eq!(
        run.config_snapshot.unwrap().agents[0].skill_ids,
        ["assigned"]
    );
}

#[test]
fn same_agent_steps_receive_distinct_prompts_and_persist_exact_runtime_input() {
    let fixture = Fixture::new();
    let mut config = config();
    let pipeline = &mut config.pipelines[0];
    pipeline.steps[0].kind = baton_core::domain::project::PipelineStepKind::Agent {
        agent_id: "writer".into(),
        prompt: "Draft the change".into(),
        include_task: None,
    };
    pipeline.steps[1].kind = baton_core::domain::project::PipelineStepKind::Agent {
        agent_id: "writer".into(),
        prompt: "Review the change".into(),
        include_task: None,
    };
    #[derive(Default)]
    struct RecordingRuntime {
        agents: Vec<String>,
        inputs: Vec<String>,
    }
    impl AgentRuntime for RecordingRuntime {
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
        ) -> Result<CheckResult, String> {
            Err("Shell execution is not configured".into())
        }
        fn list_models(&mut self) -> Result<Vec<RuntimeModel>, String> {
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
        fn supports_session_resumption(&self, _: &baton_core::domain::project::Agent) -> bool {
            true
        }
        fn execute(
            &mut self,
            _: &std::path::Path,
            agent: &baton_core::domain::project::Agent,
            input: &str,
        ) -> Result<String, String> {
            self.agents.push(agent.instructions.clone());
            self.inputs.push(input.to_owned());
            Ok("done".into())
        }
    }
    let mut runtime = RecordingRuntime::default();
    let run = start_pipeline(
        &config,
        "draft",
        "task",
        "prompt-test",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(runtime.inputs.len(), 2);
    assert_eq!(runtime.agents, ["Write clearly", "Write clearly"]);
    assert!(runtime.inputs[0].starts_with("Step instructions:\nDraft the change"));
    assert!(runtime.inputs[1].starts_with("Step instructions:\nReview the change"));
    assert_eq!(
        run.steps[0].input.as_deref(),
        Some(runtime.inputs[0].as_str())
    );
    assert_eq!(
        run.steps[1].input.as_deref(),
        Some(runtime.inputs[1].as_str())
    );
    assert!(
        run.config_snapshot.as_ref().unwrap().agents[0]
            .instructions
            .contains("Write clearly")
    );
}

#[test]
fn ordered_handoff_and_restart_safe_records() {
    let fixture = Fixture::new();
    let mut runtime = FakeRuntime::default();
    let run = start_pipeline(
        &config(),
        "draft",
        "task",
        "success",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(
        runtime.calls,
        vec![
            ("writer".into(), "task".into()),
            (
                "reviewer".into(),
                "Original task:\ntask\n\nPrevious handoff:\nwriter: task".into()
            )
        ]
    );
    assert_eq!(run.status, Status::Succeeded);
    let workspace = run.worktree.as_ref().unwrap();
    assert_ne!(workspace.path, fixture.0);
    assert_eq!(runtime.directories, vec![workspace.path.clone(); 2]);
    assert!(workspace.path.join(".git").exists());
    assert!(run.ended_at.as_ref().unwrap() >= &run.started_at);
    assert!(run.duration_ms.is_some());
    assert!(
        run.steps[1]
            .output
            .as_deref()
            .unwrap()
            .contains("writer: task")
    );
    assert_eq!(fixture.repo().load("success").unwrap(), run);
    // Reopen and deserialize from a fresh OS process with no runtime memory.
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "restart_reader"])
        .env("BATON_TEST_RESTART_DIR", &fixture.0)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(
        start_pipeline(
            &config(),
            "draft",
            "task",
            "success",
            &mut runtime,
            &fixture.repo()
        )
        .is_err()
    );
    assert_eq!(runtime.calls.len(), 2);
}

#[test]
fn human_feedback_reaches_builder_after_planner_and_repository_reload() {
    use baton_core::{
        domain::project::{Outcome, PipelineStep, Transition},
        features::human_actions::resume_human,
        ports::runtime::AgentDecision,
    };
    let fixture = Fixture::new();
    let mut config = config();
    let pipeline = &mut config.pipelines[0];
    pipeline.steps[0].kind = baton_core::domain::project::PipelineStepKind::Agent {
        agent_id: "writer".into(),
        prompt: "Resume with the saved writer prompt".into(),
        include_task: None,
    };
    pipeline.steps[0].transitions = vec![
        Transition {
            outcome: Outcome::Blocked,
            to: Some("help".into()),
        },
        Transition {
            outcome: Outcome::Completed,
            to: Some("review".into()),
        },
    ];
    pipeline.steps[1].transitions = vec![Transition {
        outcome: Outcome::Completed,
        to: None,
    }];
    pipeline.steps.push(PipelineStep {
        id: "help".into(),
        name: "Help".into(),
        kind: baton_core::domain::project::PipelineStepKind::Human,
        transitions: vec![Transition {
            outcome: Outcome::Responded,
            to: Some("write".into()),
        }],
    });
    #[derive(Default)]
    struct Runtime {
        calls: usize,
        decision: Option<AgentDecision>,
    }
    impl AgentRuntime for Runtime {
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
        ) -> Result<CheckResult, String> {
            Err("Shell execution is not configured".into())
        }
        fn list_models(&mut self) -> Result<Vec<RuntimeModel>, String> {
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
        fn take_events(&mut self) -> Vec<String> {
            Vec::new()
        }
        fn take_trace(&mut self) -> Vec<RunTraceEntry> {
            Vec::new()
        }
        fn supports_session_resumption(&self, _: &baton_core::domain::project::Agent) -> bool {
            true
        }
        fn execute(
            &mut self,
            _: &std::path::Path,
            _: &baton_core::domain::project::Agent,
            input: &str,
        ) -> Result<String, String> {
            self.calls += 1;
            let (outcome, summary) =
                match self.calls {
                    1 => {
                        assert!(input.starts_with(
                            "Step instructions:\nResume with the saved writer prompt"
                        ));
                        (Outcome::Blocked, "Which backends?")
                    }
                    2 => {
                        assert!(input.starts_with(
                            "Step instructions:\nResume with the saved writer prompt"
                        ));
                        assert!(input.contains("Tokio, WASM and Embassy"));
                        (Outcome::Completed, "Plan ready")
                    }
                    3 => {
                        assert!(input.contains("Plan ready"));
                        assert!(!input.contains("Tokio, WASM and Embassy"));
                        (Outcome::Completed, "Built")
                    }
                    _ => panic!("unexpected call"),
                };
            self.decision = Some(AgentDecision {
                outcome,
                summary: summary.into(),
            });
            Ok(summary.into())
        }
        fn take_decision(&mut self) -> Option<AgentDecision> {
            self.decision.take()
        }
    }
    let run = start_pipeline(
        &config,
        "draft",
        "timers",
        "feedback",
        &mut Runtime::default(),
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::WaitingHuman);
    let mut runtime = Runtime {
        calls: 1,
        decision: None,
    };
    let run = resume_human(
        "feedback",
        Outcome::Responded,
        "Tokio, WASM and Embassy",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Succeeded);
    assert_eq!(runtime.calls, 3);
}

#[test]
fn restart_reader() {
    let Some(path) = std::env::var_os("BATON_TEST_RESTART_DIR") else {
        return;
    };
    let run = FilesystemRunRepository(path.into())
        .load("success")
        .unwrap();
    assert_eq!(run.status, Status::Succeeded);
    assert_eq!(run.task, "task");
    assert_eq!(run.steps[0].output.as_deref(), Some("writer: task"));
    assert!(
        run.steps[1]
            .input
            .as_deref()
            .unwrap()
            .contains(run.steps[0].output.as_deref().unwrap())
    );
    assert!(
        run.steps[1]
            .output
            .as_deref()
            .unwrap()
            .contains("writer: task")
    );
}

#[test]
fn failure_stops_remaining_steps_and_survives_reopen() {
    let fixture = Fixture::new();
    let mut runtime = FakeRuntime {
        fail_agent: Some("writer".into()),
        ..Default::default()
    };
    let run = start_pipeline(
        &config(),
        "draft",
        "task",
        "failed",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(runtime.calls.len(), 1);
    assert_eq!(run.status, Status::Failed);
    assert!(run.ended_at.is_some());
    assert!(run.duration_ms.is_some());
    let workspace = run.worktree.as_ref().unwrap();
    assert_eq!(workspace.lifecycle, "retained");
    assert!(workspace.path.exists());
    assert_eq!(run.steps[0].status, Status::Failed);
    assert_eq!(run.steps[1].status, Status::Pending);
    assert!(run.steps[0].error.is_some());
    assert_eq!(fixture.repo().load("failed").unwrap(), run);
}

#[test]
fn bounds_are_enforced() {
    let fixture = Fixture::new();
    let mut runtime = FakeRuntime::default();
    assert!(
        start_pipeline(
            &config(),
            "draft",
            &"x".repeat(MAX_HANDOFF_BYTES + 1),
            "long",
            &mut runtime,
            &fixture.repo()
        )
        .is_err()
    );
    let run = start_pipeline(
        &config(),
        "draft",
        &"x".repeat(MAX_HANDOFF_BYTES),
        "overflow",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Failed);
    assert!(run.steps[0].output.is_none());
    assert_eq!(runtime.calls.len(), 1);
}

#[test]
fn restart_marks_interrupted_runs_failed_without_resuming() {
    struct Crash;
    impl AgentRuntime for Crash {
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
        ) -> Result<CheckResult, String> {
            Err("Shell execution is not configured".into())
        }
        fn list_models(&mut self) -> Result<Vec<RuntimeModel>, String> {
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
        fn supports_session_resumption(&self, _: &baton_core::domain::project::Agent) -> bool {
            true
        }
        fn execute(
            &mut self,
            _: &std::path::Path,
            _: &baton_core::domain::project::Agent,
            _: &str,
        ) -> Result<String, String> {
            panic!("process interrupted")
        }
    }
    let fixture = Fixture::new();
    let _ = std::panic::catch_unwind(|| {
        start_pipeline(
            &config(),
            "draft",
            "task",
            "interrupted-restart",
            &mut Crash,
            &fixture.repo(),
        )
    });
    AppState::new(fixture.0.clone());
    let recovered = fixture.repo().load("interrupted-restart").unwrap();
    assert_eq!(recovered.status, Status::Failed);
    assert!(recovered.ended_at.is_some());
    assert!(recovered.duration_ms.is_some());
    assert!(recovered.steps[0].started_at.is_some());
    assert!(recovered.steps[0].ended_at.is_some());
    assert!(recovered.steps[0].duration_ms.is_some());
    assert!(
        recovered.steps[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("reconcile side effects"))
    );
    assert_eq!(
        fixture
            .repo()
            .load("interrupted-restart")
            .unwrap()
            .events
            .last()
            .unwrap()
            .message
            .as_deref(),
        Some("Run interrupted by server restart")
    );
}

#[test]
fn interrupted_execution_leaves_durable_running_state_without_retry() {
    struct Crash;
    impl AgentRuntime for Crash {
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
        ) -> Result<CheckResult, String> {
            Err("Shell execution is not configured".into())
        }
        fn list_models(&mut self) -> Result<Vec<RuntimeModel>, String> {
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
        fn supports_session_resumption(&self, _: &baton_core::domain::project::Agent) -> bool {
            true
        }
        fn execute(
            &mut self,
            _: &std::path::Path,
            _: &baton_core::domain::project::Agent,
            _: &str,
        ) -> Result<String, String> {
            panic!("process interrupted")
        }
    }
    let fixture = Fixture::new();
    let _ = std::panic::catch_unwind(|| {
        start_pipeline(
            &config(),
            "draft",
            "task",
            "interrupted",
            &mut Crash,
            &fixture.repo(),
        )
    });
    let run = fixture.repo().load("interrupted").unwrap();
    assert_eq!(run.status, Status::Running);
    assert!(run.ended_at.is_none());
    assert!(run.duration_ms.is_none());
    assert_eq!(run.steps[0].status, Status::Running);
    assert_eq!(run.steps[0].input.as_deref(), Some("task"));
    assert_eq!(run.steps[1].status, Status::Pending);
}

#[test]
fn non_git_setup_failure_is_durable_and_never_invokes_runtime() {
    let fixture = Fixture::new();
    fs::remove_dir_all(fixture.0.join(".git")).unwrap();
    let mut runtime = FakeRuntime::default();
    let run = start_pipeline(
        &config(),
        "draft",
        "task",
        "non-git",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Failed);
    assert!(run.setup_error.as_ref().unwrap().contains("Git project"));
    assert!(runtime.calls.is_empty());
    assert_eq!(fixture.repo().load("non-git").unwrap(), run);
}

#[test]
fn allocation_collision_prevents_runtime_execution() {
    let fixture = Fixture::new();
    fixture
        .repo()
        .worktree_manager()
        .unwrap()
        .allocate("collision")
        .unwrap();
    let mut runtime = FakeRuntime::default();
    let run = start_pipeline(
        &config(),
        "draft",
        "task",
        "collision",
        &mut runtime,
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Failed);
    assert!(run.setup_error.as_ref().unwrap().contains("collision"));
    assert!(runtime.calls.is_empty());
    assert_eq!(fixture.repo().load("collision").unwrap(), run);
}

#[test]
fn cancellation_cannot_interleave_an_activity_observation_transaction() {
    let fixture = Fixture::new();
    let repository = fixture.repo();
    let now = utc_now();
    let run = PipelineRun {
        id: "observer-cancel-race".into(),
        task_id: None,
        worktree: None,
        setup_error: None,
        session: None,
        pipeline_id: "pipeline".into(),
        task: "task".into(),
        started_at: now,
        ended_at: None,
        duration_ms: None,
        events: vec![],
        trace: vec![],
        status: Status::Running,
        steps: vec![],
        config_snapshot: None,
        current_step: None,
        handoff: None,
        pending_actions: vec![],
    };
    repository.create(&run).unwrap();
    let observation = repository.acquire_action_lease(&run.id).unwrap();
    let root = fixture.0.clone();
    let cancel = std::thread::spawn(move || {
        let repository_for_cancel = FilesystemRunRepository(root);
        baton_core::features::runs::cancel_run(&repository_for_cancel, "observer-cancel-race")
    });
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert_eq!(repository.load(&run.id).unwrap().status, Status::Running);
    drop(observation);
    assert_eq!(cancel.join().unwrap().unwrap().status, Status::Cancelled);
}

#[test]
fn observer_retries_until_a_cancellation_transaction_is_visible() {
    let fixture = Fixture::new();
    let repository = fixture.repo();
    let run = PipelineRun {
        id: "observer-cancel-retry".into(),
        task_id: None,
        worktree: None,
        setup_error: None,
        session: None,
        pipeline_id: "pipeline".into(),
        task: "task".into(),
        started_at: utc_now(),
        ended_at: None,
        duration_ms: None,
        events: vec![],
        trace: vec![],
        status: Status::Running,
        steps: vec![],
        config_snapshot: None,
        current_step: None,
        handoff: None,
        pending_actions: vec![],
    };
    repository.create(&run).unwrap();
    let lease = repository.acquire_action_lease(&run.id).unwrap();
    let root = fixture.0.clone();
    let cancel = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(30));
        let repository = FilesystemRunRepository(root);
        let mut run = repository.load("observer-cancel-retry").unwrap();
        run.status = Status::Cancelled;
        run.ended_at = Some(utc_now());
        repository.save(&run).unwrap();
        drop(lease);
    });
    let cancelled = || {
        repository
            .load(&run.id)
            .is_ok_and(|run| run.status == Status::Cancelled)
    };
    let result = baton_core::features::pipeline_engine::acquire_action_lease_retry(
        &repository,
        &run.id,
        &cancelled,
    );
    assert!(matches!(result, Err(error) if error.contains("cancelled")));
    cancel.join().unwrap();
    assert_eq!(repository.load(&run.id).unwrap().status, Status::Cancelled);
}

#[test]
fn cancelled_run_retains_its_workspace() {
    struct Cancel(FilesystemRunRepository);
    impl AgentRuntime for Cancel {
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
        ) -> Result<CheckResult, String> {
            Err("Shell execution is not configured".into())
        }
        fn list_models(&mut self) -> Result<Vec<RuntimeModel>, String> {
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
        fn supports_session_resumption(&self, _: &baton_core::domain::project::Agent) -> bool {
            true
        }
        fn execute(
            &mut self,
            directory: &std::path::Path,
            _: &baton_core::domain::project::Agent,
            _: &str,
        ) -> Result<String, String> {
            let mut run = self.0.load("cancelled").unwrap();
            assert_eq!(directory, run.worktree.as_ref().unwrap().path);
            fs::write(directory.join("inspection.txt"), "partial work").unwrap();
            run.status = Status::Cancelled;
            self.0.save(&run).unwrap();
            Ok("discarded".into())
        }
    }
    let fixture = Fixture::new();
    let run = start_pipeline(
        &config(),
        "draft",
        "task",
        "cancelled",
        &mut Cancel(fixture.repo()),
        &fixture.repo(),
    )
    .unwrap();
    assert_eq!(run.status, Status::Cancelled);
    assert_eq!(run.steps[0].status, Status::Cancelled);
    assert!(run.steps[0].ended_at.is_some());
    assert!(run.steps[0].duration_ms.is_some());
    assert_eq!(run.steps[1].status, Status::Pending);
    let workspace = run.worktree.unwrap();
    assert_eq!(workspace.lifecycle, "retained");
    assert!(workspace.path.join("inspection.txt").exists());
}

#[test]
fn cancellation_during_allocation_retains_identity_without_invocation() {
    use baton_core::{domain::worktrees::RunWorktree, ports::worktrees::WorktreeManager};
    struct CancelDuringAllocation(PathBuf);
    impl WorktreeManager for CancelDuringAllocation {
        fn allocate(&self, id: &str) -> Result<RunWorktree, String> {
            let repository = FilesystemRunRepository(self.0.clone());
            let workspace = repository.worktree_manager()?.allocate(id)?;
            let mut run = repository.load(id)?;
            run.status = Status::Cancelled;
            repository.save(&run)?;
            Ok(workspace)
        }
        fn cleanup(&self, _: &str, _: &mut RunWorktree) -> Result<(), String> {
            unreachable!()
        }
    }
    struct Repository(PathBuf);
    impl RunRepository for Repository {
        fn worktree_manager(&self) -> Result<Box<dyn WorktreeManager>, String> {
            Ok(Box::new(CancelDuringAllocation(self.0.clone())))
        }
        fn create(&self, run: &PipelineRun) -> Result<(), String> {
            FilesystemRunRepository(self.0.clone()).create(run)
        }
        fn save(&self, run: &PipelineRun) -> Result<(), String> {
            FilesystemRunRepository(self.0.clone()).save(run)
        }
        fn load(&self, id: &str) -> Result<PipelineRun, String> {
            FilesystemRunRepository(self.0.clone()).load(id)
        }
        fn list(&self) -> Result<Vec<PipelineRun>, String> {
            FilesystemRunRepository(self.0.clone()).list()
        }
    }
    let fixture = Fixture::new();
    let mut runtime = FakeRuntime::default();
    let run = start_pipeline(
        &config(),
        "draft",
        "task",
        "cancel-setup",
        &mut runtime,
        &Repository(fixture.0.clone()),
    )
    .unwrap();
    assert_eq!(run.status, Status::Cancelled);
    assert!(runtime.calls.is_empty());
    assert!(run.worktree.as_ref().unwrap().path.exists());
    assert_eq!(fixture.repo().load("cancel-setup").unwrap(), run);
}
