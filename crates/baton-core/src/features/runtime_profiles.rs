//! Runtime-profile application commands and configuration mutations.
use crate::ports::project_config::ProjectConfigRepository;
use medi_rs::{MediCommand, medi_handler, medi_module};
use std::sync::Arc;

pub use crate::domain::runtime_profiles::{
    RuntimeKind, RuntimeProfile, list, profile_id, resolve, validate,
};

#[derive(Debug)]
pub enum RuntimeProfileError {
    NotFound,
    Conflict(String),
    Validation(String),
    Storage(String),
}

pub enum Mutation {
    Save(RuntimeProfile),
    Delete(String),
}

#[derive(Clone)]
pub struct RuntimeProfileRepositoryResource(pub Arc<dyn ProjectConfigRepository>);

pub async fn mutate(
    repository: &dyn ProjectConfigRepository,
    revision: &str,
    mutation: Mutation,
) -> Result<(), RuntimeProfileError> {
    let mut snapshot =
        crate::features::agents::list(repository)
            .await
            .map_err(|error| match error {
                crate::features::agents::AgentError::NotFound => RuntimeProfileError::NotFound,
                crate::features::agents::AgentError::Conflict(message) => {
                    RuntimeProfileError::Conflict(message)
                }
                crate::features::agents::AgentError::Validation(message) => {
                    RuntimeProfileError::Validation(message)
                }
                crate::features::agents::AgentError::Storage(message) => {
                    RuntimeProfileError::Storage(message)
                }
            })?;
    if snapshot.revision != revision {
        return Err(RuntimeProfileError::Conflict(
            "Configuration changed; reload before saving".into(),
        ));
    }
    match mutation {
        Mutation::Save(profile) => {
            if profile.id != profile_id(&profile.runtime_kind, profile.model.as_ref()) {
                return Err(RuntimeProfileError::Validation(
                    "Profile ID must match runtime.provider.model".into(),
                ));
            }
            snapshot
                .config
                .runtime_profiles
                .retain(|existing| existing.id != profile.id);
            snapshot.config.runtime_profiles.push(profile);
        }
        Mutation::Delete(id) => {
            if snapshot
                .config
                .agents
                .iter()
                .any(|agent| agent.runtime_profile_id == id)
            {
                return Err(RuntimeProfileError::Conflict(
                    "Runtime profile is assigned to an agent".into(),
                ));
            }
            let count = snapshot.config.runtime_profiles.len();
            snapshot
                .config
                .runtime_profiles
                .retain(|profile| profile.id != id);
            if count == snapshot.config.runtime_profiles.len() {
                return Err(RuntimeProfileError::NotFound);
            }
        }
    }
    snapshot
        .config
        .validate()
        .map_err(RuntimeProfileError::Validation)?;
    repository
        .update(revision, &snapshot.config)
        .await
        .map_err(RuntimeProfileError::Conflict)
}

#[derive(MediCommand)]
#[medi_command(return_type = Vec<RuntimeProfile>, error_type = RuntimeProfileError)]
pub struct ListRuntimeProfiles;

#[derive(MediCommand)]
#[medi_command(return_type = RuntimeProfile, error_type = RuntimeProfileError)]
pub struct GetRuntimeProfile {
    pub id: String,
}

#[derive(MediCommand)]
#[medi_command(error_type = RuntimeProfileError)]
pub struct MutateRuntimeProfile {
    pub revision: String,
    pub mutation: Mutation,
}

#[medi_handler]
async fn list_profiles_handler(
    repository: RuntimeProfileRepositoryResource,
    _command: ListRuntimeProfiles,
) -> Result<Vec<RuntimeProfile>, RuntimeProfileError> {
    let snapshot = crate::features::agents::list(repository.0.as_ref())
        .await
        .map_err(|error| match error {
            crate::features::agents::AgentError::NotFound => RuntimeProfileError::NotFound,
            crate::features::agents::AgentError::Conflict(message) => {
                RuntimeProfileError::Conflict(message)
            }
            crate::features::agents::AgentError::Validation(message) => {
                RuntimeProfileError::Validation(message)
            }
            crate::features::agents::AgentError::Storage(message) => {
                RuntimeProfileError::Storage(message)
            }
        })?;
    Ok(list(&snapshot.config))
}

#[medi_handler]
async fn get_profile_handler(
    repository: RuntimeProfileRepositoryResource,
    command: GetRuntimeProfile,
) -> Result<RuntimeProfile, RuntimeProfileError> {
    list_profiles_handler(repository, ListRuntimeProfiles)
        .await?
        .into_iter()
        .find(|profile| profile.id == command.id)
        .ok_or(RuntimeProfileError::NotFound)
}

#[medi_handler]
async fn mutate_profile_handler(
    repository: RuntimeProfileRepositoryResource,
    command: MutateRuntimeProfile,
) -> Result<(), RuntimeProfileError> {
    mutate(repository.0.as_ref(), &command.revision, command.mutation).await
}

medi_module! {
    pub manifest runtime_profiles_feature;
    resources {
        runtime_profile_repository: crate::features::runtime_profiles::RuntimeProfileRepositoryResource;
    }
    commands {
        crate::features::runtime_profiles::ListRuntimeProfiles => crate::features::runtime_profiles::list_profiles_handler;
        crate::features::runtime_profiles::GetRuntimeProfile => crate::features::runtime_profiles::get_profile_handler;
        crate::features::runtime_profiles::MutateRuntimeProfile => crate::features::runtime_profiles::mutate_profile_handler;
    }
}
