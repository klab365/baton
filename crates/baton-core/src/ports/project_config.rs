use crate::domain::project::{ConfigState, ProjectConfig};
use async_trait::async_trait;

#[async_trait]
pub trait ProjectConfigRepository: Send + Sync {
    async fn load(&self) -> Result<ConfigState, String>;
    async fn create(&self, config: &ProjectConfig) -> Result<(), String>;
    /// Replace only when the exact persisted revision still matches.
    async fn update(&self, expected_revision: &str, config: &ProjectConfig) -> Result<(), String>;
}
