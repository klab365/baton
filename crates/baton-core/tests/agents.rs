use async_trait::async_trait;
use baton_core::{
    agents::{self, AgentError, Mutation},
    project::{ConfigSnapshot, ConfigState, ProjectConfig},
    project_config::ProjectConfigRepository,
};
use std::sync::Mutex;

struct MemoryRepository(Mutex<ConfigSnapshot>);

#[async_trait]
impl ProjectConfigRepository for MemoryRepository {
    async fn load(&self) -> Result<ConfigState, String> {
        Ok(ConfigState::Ready(self.0.lock().unwrap().clone()))
    }
    async fn create(&self, _: &ProjectConfig) -> Result<(), String> {
        unreachable!()
    }
    async fn update(&self, revision: &str, config: &ProjectConfig) -> Result<(), String> {
        let mut snapshot = self.0.lock().unwrap();
        if snapshot.revision != revision {
            return Err("Configuration changed; reload before saving".into());
        }
        snapshot.config = config.clone();
        snapshot.revision.push('x');
        Ok(())
    }
}

fn fixture() -> (MemoryRepository, ProjectConfig) {
    let config: ProjectConfig =
        serde_json::from_str(include_str!("../../../examples/demo/.baton/config.json")).unwrap();
    (
        MemoryRepository(Mutex::new(ConfigSnapshot {
            config: config.clone(),
            revision: "initial".into(),
        })),
        config,
    )
}

#[tokio::test]
async fn agents_crud_require_available_models_and_unreferenced_deletion() {
    let (repository, original) = fixture();
    let snapshot = agents::list(&repository).await.unwrap();
    let mut agent = original.agents[0].clone();
    agent.id = "new-agent".into();

    assert!(matches!(
        agents::mutate(&repository, &snapshot.revision, Mutation::Create(agent.clone()), |_, _| false).await,
        Err(AgentError::Validation(message)) if message.contains("not configured")
    ));
    agents::mutate(
        &repository,
        &snapshot.revision,
        Mutation::Create(agent.clone()),
        |p, m| p == "fake" && m == "deterministic",
    )
    .await
    .unwrap();

    let snapshot = agents::list(&repository).await.unwrap();
    agent.system_prompt = "A revised prompt.".into();
    agents::mutate(
        &repository,
        &snapshot.revision,
        Mutation::Update(agent.id.clone(), agent.clone()),
        |_, _| true,
    )
    .await
    .unwrap();

    let snapshot = agents::list(&repository).await.unwrap();
    assert!(matches!(
        agents::mutate(&repository, &snapshot.revision, Mutation::Delete("writer".into()), |_, _| true).await,
        Err(AgentError::Conflict(message)) if message.contains("Draft and review") && message.contains("Write")
    ));
    agents::mutate(
        &repository,
        &snapshot.revision,
        Mutation::Delete(agent.id.clone()),
        |_, _| true,
    )
    .await
    .unwrap();
    assert_eq!(agents::list(&repository).await.unwrap().config, original);
}
