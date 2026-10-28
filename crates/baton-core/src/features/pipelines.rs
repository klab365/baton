use crate::{
    domain::project::{ConfigSnapshot, ConfigState, Pipeline},
    ports::project_config::ProjectConfigRepository,
};
use medi_rs::{MediCommand, medi_handler, medi_module};
use std::sync::Arc;

#[derive(Debug)]
pub enum PipelineError {
    NotFound,
    Conflict(String),
    Validation(String),
    Storage(String),
}

pub async fn list(
    repository: &dyn ProjectConfigRepository,
) -> Result<ConfigSnapshot, PipelineError> {
    match repository.load().await.map_err(PipelineError::Storage)? {
        ConfigState::Ready(snapshot) => Ok(snapshot),
        ConfigState::Missing => Err(PipelineError::Conflict(
            "Initialize the project first".into(),
        )),
        ConfigState::Invalid(message) => Err(PipelineError::Conflict(message)),
    }
}

pub enum Mutation {
    Create(Pipeline),
    Update(String, Pipeline),
    Delete(String),
}

#[derive(Clone)]
pub struct PipelineRepositoryResource(pub Arc<dyn ProjectConfigRepository>);

#[derive(MediCommand)]
#[medi_command(return_type = ConfigSnapshot, error_type = PipelineError)]
pub struct ListPipelines;

#[derive(MediCommand)]
#[medi_command(error_type = PipelineError)]
pub struct MutatePipeline {
    pub revision: String,
    pub mutation: Mutation,
}

pub async fn mutate(
    repository: &dyn ProjectConfigRepository,
    revision: &str,
    mutation: Mutation,
) -> Result<(), PipelineError> {
    let mut snapshot = list(repository).await?;
    if snapshot.revision != revision {
        return Err(PipelineError::Conflict(
            "Configuration changed; reload before saving".into(),
        ));
    }
    match mutation {
        Mutation::Create(pipeline) => {
            if snapshot
                .config
                .pipelines
                .iter()
                .any(|item| item.id == pipeline.id)
            {
                return Err(PipelineError::Conflict("Pipeline ID already exists".into()));
            }
            snapshot.config.pipelines.push(pipeline);
        }
        Mutation::Update(id, pipeline) => {
            if id != pipeline.id {
                return Err(PipelineError::Validation(
                    "Pipeline ID cannot be changed".into(),
                ));
            }
            let existing = snapshot
                .config
                .pipelines
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or(PipelineError::NotFound)?;
            *existing = pipeline;
        }
        Mutation::Delete(id) => {
            let index = snapshot
                .config
                .pipelines
                .iter()
                .position(|item| item.id == id)
                .ok_or(PipelineError::NotFound)?;
            snapshot.config.pipelines.remove(index);
        }
    }
    snapshot
        .config
        .validate()
        .map_err(PipelineError::Validation)?;
    repository
        .update(revision, &snapshot.config)
        .await
        .map_err(PipelineError::Conflict)
}

#[medi_handler]
async fn list_pipelines_handler(
    repository: PipelineRepositoryResource,
    _command: ListPipelines,
) -> Result<ConfigSnapshot, PipelineError> {
    list(repository.0.as_ref()).await
}

#[medi_handler]
async fn mutate_pipeline_handler(
    repository: PipelineRepositoryResource,
    command: MutatePipeline,
) -> Result<(), PipelineError> {
    mutate(repository.0.as_ref(), &command.revision, command.mutation).await
}

medi_module! {
    pub manifest pipelines_feature;
    resources { pipeline_repository: crate::features::pipelines::PipelineRepositoryResource; }
    commands {
        crate::features::pipelines::ListPipelines => crate::features::pipelines::list_pipelines_handler;
        crate::features::pipelines::MutatePipeline => crate::features::pipelines::mutate_pipeline_handler;
    }
}
