use baton_core::domain::{
    project::ProjectConfig,
    tasks::{Task, TaskInput, TaskStatus},
};

fn config() -> ProjectConfig {
    serde_json::from_str(include_str!("fixture_config.json")).unwrap()
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
fn tasks_validate_inputs_without_storing_project_configuration() {
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
    let mut attempted_start = input(&pipeline);
    attempted_start.status = TaskStatus::Doing;
    assert!(
        task.update(
            attempted_start,
            config.clone(),
            "2025-01-02T00:00:00.000Z".into()
        )
        .is_err()
    );
    assert_eq!(task.status, TaskStatus::Draft);
    let mut changed = config;
    changed.pipelines[0].name = "Changed".into();
    task.update(input(&pipeline), changed, "2025-01-02T00:00:00.000Z".into())
        .unwrap();
    let serialized = serde_json::to_value(&task).unwrap();
    assert!(serialized.get("configSnapshot").is_none());
    assert_eq!(task.created_at, "2025-01-01T00:00:00.000Z");
    assert_eq!(task.updated_at, "2025-01-02T00:00:00.000Z");
}
