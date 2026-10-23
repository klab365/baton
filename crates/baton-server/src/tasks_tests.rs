use super::*;
use axum::{Json, extract::State};
use baton_core::{
    project::ProjectConfig,
    tasks::{Task, TaskInput},
};

fn input(pipeline_id: &str) -> TaskInput {
    TaskInput {
        title: "Implement a feature".into(),
        description: "Feature details and context".into(),
        acceptance_criteria: "Tests pass".into(),
        pipeline_id: pipeline_id.into(),
    }
}

#[tokio::test]
async fn task_lifecycle_survives_reopen_and_preserves_snapshot() {
    let dir = std::env::temp_dir().join(format!(
        "baton-tasks-{}-{}",
        std::process::id(),
        super::tasks::test_timestamp()
    ));
    let config: ProjectConfig =
        serde_json::from_str(include_str!("../../../examples/demo/.baton/config.json")).unwrap();
    let pipeline = config.pipelines[0].id.clone();
    assert!(
        Task::create(
            "bad".into(),
            input("missing"),
            config.clone(),
            "2025-01-01T00:00:00.000Z".into()
        )
        .is_err()
    );
    let mut empty = input(&pipeline);
    empty.title = " ".into();
    assert!(
        Task::create(
            "bad".into(),
            empty,
            config.clone(),
            "2025-01-01T00:00:00.000Z".into()
        )
        .is_err()
    );
    let state = AppState::new(dir.clone());
    let task = Task::create(
        "01941f7c-5a00-7000-8000-000000000001".into(),
        input(&pipeline),
        config.clone(),
        "2025-01-01T00:00:00.000Z".into(),
    )
    .unwrap();
    super::tasks::test_save(&state, &task);
    let reopened = AppState::new(dir.clone());
    let mut task = super::tasks::test_load(&reopened, "01941f7c-5a00-7000-8000-000000000001");
    let original = task.config_snapshot.clone();
    let mut changed = config;
    changed.pipelines[0].name = "Changed".into();
    task.update(input(&pipeline), changed, "2025-01-02T00:00:00.000Z".into())
        .unwrap();
    assert_eq!(task.config_snapshot, original);
    assert_eq!(task.created_at, "2025-01-01T00:00:00.000Z");
    assert_eq!(task.updated_at, "2025-01-02T00:00:00.000Z");
    super::tasks::test_save(&reopened, &task);
    let Json(tasks) = super::tasks::list(State(reopened.clone()))
        .await
        .unwrap_or_else(|_| panic!("list failed"));
    assert_eq!(tasks.len(), 1);
    super::tasks::delete(
        State(reopened.clone()),
        axum::extract::Path("01941f7c-5a00-7000-8000-000000000001".into()),
    )
    .await
    .unwrap_or_else(|_| panic!("delete failed"));
    let Json(tasks) = super::tasks::list(State(reopened))
        .await
        .unwrap_or_else(|_| panic!("list failed"));
    assert!(tasks.is_empty());
    assert!(!dir.join(".baton/config.json").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
