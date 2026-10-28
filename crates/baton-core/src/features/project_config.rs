use medi_rs::{MediCommand, medi_handler, medi_module};
use std::sync::Arc;

use crate::{
    domain::project::{ConfigState, ProjectConfig},
    ports::project_config::ProjectConfigRepository,
};

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

#[derive(MediCommand)]
#[medi_command(return_type = ConfigState, error_type = String)]
pub struct GetProjectConfig;

#[derive(MediCommand)]
#[medi_command(error_type = String)]
pub struct InitializeProject;

#[medi_handler]
async fn get_project_config_handler(
    repository: Arc<dyn ProjectConfigRepository>,
    _command: GetProjectConfig,
) -> Result<ConfigState, String> {
    get_project_config(repository.as_ref()).await
}

#[medi_handler]
async fn initialize_project_handler(
    repository: Arc<dyn ProjectConfigRepository>,
    _command: InitializeProject,
) -> Result<(), String> {
    initialize_project(repository.as_ref()).await
}

medi_module! {
    pub manifest project_config;
    resources {
        repository: Arc<dyn crate::ports::project_config::ProjectConfigRepository>;
    }
    commands {
        crate::features::project_config::GetProjectConfig => crate::features::project_config::get_project_config_handler;
        crate::features::project_config::InitializeProject => crate::features::project_config::initialize_project_handler;
    }
}
