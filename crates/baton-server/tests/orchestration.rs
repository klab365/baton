use baton_adapters::adapters::FakeRuntime;
use baton_core::{orchestration::*, project::parse_config};
use baton_server::run_repository::FilesystemRunRepository;
use std::{fs, path::PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "baton-run-{}-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )))
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
    parse_config(include_str!("../../../examples/demo/.baton/config.json")).unwrap()
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
            ("reviewer".into(), "writer: task".into())
        ]
    );
    assert_eq!(run.status, Status::Succeeded);
    assert!(run.ended_at.as_ref().unwrap() >= &run.started_at);
    assert!(run.duration_ms.is_some());
    assert_eq!(
        run.steps[1].output.as_deref(),
        Some("reviewer: writer: task")
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
    assert_eq!(run.steps[1].input, run.steps[0].output);
    assert_eq!(
        run.steps[1].output.as_deref(),
        Some("reviewer: writer: task")
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
fn interrupted_execution_leaves_durable_running_state_without_retry() {
    struct Crash;
    impl AgentRuntime for Crash {
        fn execute(&mut self, _: &baton_core::project::Agent, _: &str) -> Result<String, String> {
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
