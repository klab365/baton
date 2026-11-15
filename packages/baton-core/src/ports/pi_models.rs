use crate::domain::runtime::RuntimeModel;
use async_trait::async_trait;

/// Lists models reported by the locally installed Pi CLI without inspecting credentials.
#[async_trait]
pub trait PiModelCatalog: Send + Sync {
    async fn list_models(&self, provider: &str) -> Result<Vec<RuntimeModel>, String>;
}
