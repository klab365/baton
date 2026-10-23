use crate::{
    project::{ConfigSnapshot, ConfigState, Pipeline},
    project_config::ProjectConfigRepository,
};

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
