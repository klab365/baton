use baton_core::{
    project::ProjectConfig,
    tasks::{Task, TaskInput, TaskStatus},
};

fn config() -> ProjectConfig {
    serde_json::from_str(include_str!("../../../examples/demo/.baton/config.json")).unwrap()
}

fn input(pipeline_id: &str) -> TaskInput {
    TaskInput {
        title: "Implement a feature".into(),
        description: "Feature details".into(),
        acceptance_criteria: "Tests pass".into(),
        pipeline_id: pipeline_id.into(),
        status: TaskStatus::Draft,
    }
}

#[test]
fn tasks_validate_inputs_and_preserve_snapshots_until_pipeline_changes() {
    let config = config();
    let pipeline = config.pipelines[0].id.clone();
    assert!(
        Task::create(
            "task".into(),
            input("missing"),
            config.clone(),
            "2025-01-01T00:00:00.000Z".into()
        )
        .is_err()
    );
    let mut invalid = input(&pipeline);
    invalid.title = " ".into();
    assert!(
        Task::create(
            "task".into(),
            invalid,
            config.clone(),
            "2025-01-01T00:00:00.000Z".into()
        )
        .is_err()
    );

    let mut task = Task::create(
        "task".into(),
        input(&pipeline),
        config.clone(),
        "2025-01-01T00:00:00.000Z".into(),
    )
    .unwrap();
    assert!(matches!(task.status, TaskStatus::Draft));
    let original = task.config_snapshot.clone();
    let mut changed = config;
    changed.pipelines[0].name = "Changed".into();
    task.update(input(&pipeline), changed, "2025-01-02T00:00:00.000Z".into())
        .unwrap();
    assert_eq!(task.config_snapshot, original);
    assert_eq!(task.created_at, "2025-01-01T00:00:00.000Z");
    assert_eq!(task.updated_at, "2025-01-02T00:00:00.000Z");
}
