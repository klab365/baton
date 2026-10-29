use async_trait::async_trait;
use baton_core::{
    domain::project::{ConfigSnapshot, ConfigState, ProjectConfig},
    features::agents::{self, AgentError, Mutation},
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
    let config: ProjectConfig = serde_json::from_str(
        r#"{"schemaVersion":7,"agents":[{"id":"writer","systemPrompt":"Write clearly","runtimeKind":"rig","model":{"provider":"openai","model":"gpt-4.1"},"credentialId":null,"skillIds":[]}],"pipelines":[{"id":"draft","name":"Draft and review","steps":[{"id":"write","name":"Write","agentId":"writer","human":false,"transitions":[]}]}]}"#,
    ).unwrap();
    (
        MemoryRepository(Mutex::new(ConfigSnapshot {
            config: config.clone(),
            revision: "initial".into(),
        })),
        config,
    )
}

#[tokio::test]
async fn agents_can_be_configured_directly_and_unreferenced_agents_deleted() {
    let (repository, original) = fixture();
    let snapshot = agents::list(&repository).await.unwrap();
    let mut agent = original.agents[0].clone();
    agent.id = "new-agent".into();
    agents::mutate(
        &repository,
        &snapshot.revision,
        Mutation::Create(agent.clone()),
    )
    .await
    .unwrap();

    let snapshot = agents::list(&repository).await.unwrap();
    agent.system_prompt = "A revised prompt.".into();
    agents::mutate(
        &repository,
        &snapshot.revision,
        Mutation::Update(agent.id.clone(), agent.clone()),
    )
    .await
    .unwrap();

    let snapshot = agents::list(&repository).await.unwrap();
    assert!(
        matches!(agents::mutate(&repository, &snapshot.revision, Mutation::Delete("writer".into())).await,
        Err(AgentError::Conflict(message)) if message.contains("Draft and review") && message.contains("Write"))
    );
    agents::mutate(
        &repository,
        &snapshot.revision,
        Mutation::Delete(agent.id.clone()),
    )
    .await
    .unwrap();
    assert_eq!(agents::list(&repository).await.unwrap().config, original);
}
