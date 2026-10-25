use async_trait::async_trait;
use baton_core::{
    project::{ConfigSnapshot, ConfigState, ProjectConfig},
    project_config::ProjectConfigRepository,
    tools,
};
use std::sync::Mutex;

struct Repository(Mutex<ConfigSnapshot>);
#[async_trait]
impl ProjectConfigRepository for Repository {
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
fn repository() -> Repository {
    Repository(Mutex::new(ConfigSnapshot {
        config: ProjectConfig::CURRENT,
        revision: "one".into(),
    }))
}
#[tokio::test]
async fn catalog_only_enables_known_tools_and_prevents_orphaned_assignments() {
    let repository = repository();
    assert!(
        tools::definitions()
            .iter()
            .any(|tool| tool.id == "builtin.read.v1")
    );
    tools::enable(&repository, "one", "builtin.read.v1")
        .await
        .unwrap();
    assert_eq!(
        tools::list(&repository).await.unwrap().config.tools[0].id,
        "builtin.read.v1"
    );
    assert!(matches!(
        tools::enable(&repository, "onex", "unknown.v1").await,
        Err(tools::ToolError::Validation(_))
    ));
    let mut snapshot = tools::list(&repository).await.unwrap();
    snapshot.config.agents.push(baton_core::project::Agent {
        id: "assigned".into(),
        system_prompt: "test".into(),
        model: baton_core::project::ModelSettings {
            provider: "fake".into(),
            model: "fake".into(),
            max_output_tokens: None,
        },
        rules: vec![],
        tool_ids: vec!["builtin.read.v1".into()],
        skill_ids: vec![],
    });
    repository
        .update(&snapshot.revision, &snapshot.config)
        .await
        .unwrap();
    let revision = tools::list(&repository).await.unwrap().revision;
    assert!(matches!(
        tools::disable(&repository, &revision, "builtin.read.v1").await,
        Err(tools::ToolError::Conflict(_))
    ));
}
