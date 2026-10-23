use async_trait::async_trait;

use crate::project::{ConfigState, ProjectConfig};

#[async_trait]
pub trait ProjectConfigRepository: Send + Sync {
    async fn load(&self) -> Result<ConfigState, String>;
    async fn create(&self, config: &ProjectConfig) -> Result<(), String>;
    /// Replace only when the exact persisted revision still matches.
    async fn update(&self, expected_revision: &str, config: &ProjectConfig) -> Result<(), String>;
}

pub async fn get_project_config(
    repository: &dyn ProjectConfigRepository,
) -> Result<ConfigState, String> {
    repository.load().await
}

pub async fn initialize_project(repository: &dyn ProjectConfigRepository) -> Result<(), String> {
    repository.create(&ProjectConfig::CURRENT).await
}

pub async fn update_project_config(
    repository: &dyn ProjectConfigRepository,
    expected_revision: &str,
    config: &ProjectConfig,
) -> Result<(), String> {
    config.validate()?;
    repository.update(expected_revision, config).await
}
