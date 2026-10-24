use baton_adapters::FakeRuntime;
use baton_adapters::filesystem_run_repository::FilesystemRunRepository;
use baton_core::{orchestration::*, project::parse_config};
use baton_server::AppState;
use std::{fs, path::PathBuf};

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
fn config() -> baton_core::project::ProjectConfig {
    // Keep legacy sequential orchestration assertions independent from the
    // interactive demo's intentionally conditional workflow.
    let mut config =
        parse_config(include_str!("../../../examples/demo/.baton/config.json")).unwrap();
    let pipeline = &mut config.pipelines[0];
    pipeline
        .steps
        .retain(|step| step.id == "write" || step.id == "review");
    for step in &mut pipeline.steps {
        step.human = false;
        step.transitions.clear();
    }
    config
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
            ("reviewer".into(), "Original task:\ntask\n\nPrevious step output and handoff (including any reviewer feedback):\nwriter: task".into())
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
        fn execute(
            &mut self,
            _: &std::path::Path,
            _: &baton_core::project::Agent,
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
    assert!(recovered.duration_ms.is_none());
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
        fn execute(
            &mut self,
            _: &std::path::Path,
            _: &baton_core::project::Agent,
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
fn cancelled_run_retains_its_workspace() {
    struct Cancel(FilesystemRunRepository);
    impl AgentRuntime for Cancel {
        fn execute(
            &mut self,
            directory: &std::path::Path,
            _: &baton_core::project::Agent,
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
    assert_eq!(run.steps[1].status, Status::Pending);
    let workspace = run.worktree.unwrap();
    assert_eq!(workspace.lifecycle, "retained");
    assert!(workspace.path.join("inspection.txt").exists());
}

#[test]
fn cancellation_during_allocation_retains_identity_without_invocation() {
    use baton_core::worktrees::{RunWorktree, WorktreeManager};
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
