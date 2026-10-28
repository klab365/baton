use async_trait::async_trait;
use baton_core::{
    domain::project::{ConfigSnapshot, ConfigState, ProjectConfig},
    features::pipelines::{self, Mutation, PipelineError},
    ports::project_config::ProjectConfigRepository,
};
use std::sync::Mutex;

struct MemoryRepository(Mutex<ConfigSnapshot>);

#[async_trait]
impl ProjectConfigRepository for MemoryRepository {
    async fn load(&self) -> Result<ConfigState, String> {
        Ok(ConfigState::Ready(self.0.lock().unwrap().clone()))
    }

    async fn create(&self, _: &ProjectConfig) -> Result<(), String> {
        unreachable!("the fixture is already initialized")
    }

    async fn update(&self, expected_revision: &str, config: &ProjectConfig) -> Result<(), String> {
        let mut snapshot = self.0.lock().unwrap();
        if snapshot.revision != expected_revision {
            return Err("Configuration changed; reload before saving".into());
        }
        snapshot.config = config.clone();
        snapshot.revision = format!("revision-{}", snapshot.revision.len());
        Ok(())
    }
}

fn fixture() -> (MemoryRepository, ProjectConfig) {
    let config: ProjectConfig = serde_json::from_str(include_str!("fixture_config.json")).unwrap();
    let repository = MemoryRepository(Mutex::new(ConfigSnapshot {
        config: config.clone(),
        revision: "initial".into(),
    }));
    (repository, config)
}

#[tokio::test]
async fn pipeline_crud_validates_and_preserves_project_configuration() {
    let (repository, original) = fixture();
    let snapshot = pipelines::list(&repository).await.unwrap();
    let mut pipeline = original.pipelines[0].clone();
    pipeline.id = "new-chain".into();

    let mut invalid = pipeline.clone();
    invalid.name = " ".into();
    assert!(matches!(
        pipelines::mutate(&repository, &snapshot.revision, Mutation::Create(invalid)).await,
        Err(PipelineError::Validation(_))
    ));
    let mut invalid = pipeline.clone();
    invalid.steps.clear();
    assert!(matches!(
        pipelines::mutate(&repository, &snapshot.revision, Mutation::Create(invalid)).await,
        Err(PipelineError::Validation(_))
    ));
    let mut invalid = pipeline.clone();
    invalid.steps[0].agent_id = "missing".into();
    assert!(matches!(
        pipelines::mutate(&repository, &snapshot.revision, Mutation::Create(invalid)).await,
        Err(PipelineError::Validation(_))
    ));

    pipelines::mutate(
        &repository,
        &snapshot.revision,
        Mutation::Create(pipeline.clone()),
    )
    .await
    .unwrap();
    assert!(matches!(
        pipelines::mutate(
            &repository,
            &snapshot.revision,
            Mutation::Delete(pipeline.id.clone())
        )
        .await,
        Err(PipelineError::Conflict(_))
    ));

    let snapshot = pipelines::list(&repository).await.unwrap();
    pipeline.name = "Updated chain".into();
    pipeline.steps.reverse();
    pipelines::mutate(
        &repository,
        &snapshot.revision,
        Mutation::Update(pipeline.id.clone(), pipeline.clone()),
    )
    .await
    .unwrap();
    let snapshot = pipelines::list(&repository).await.unwrap();
    assert_eq!(snapshot.config.pipelines.last(), Some(&pipeline));
    assert_eq!(snapshot.config.agents, original.agents);
    assert_eq!(snapshot.config.skills, original.skills);

    pipelines::mutate(
        &repository,
        &snapshot.revision,
        Mutation::Delete(pipeline.id),
    )
    .await
    .unwrap();
    assert_eq!(pipelines::list(&repository).await.unwrap().config, original);
}
