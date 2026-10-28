//! Credential-vault use cases. Responses contain metadata only.
use crate::{
    domain::credentials::{Credential, CredentialStatus},
    ports::{
        credentials::{
            CredentialModelCatalog, CredentialRepository, CredentialTypeCatalog,
            CredentialTypeDescriptor, ModelDescriptor,
        },
        project_config::ProjectConfigRepository,
    },
};
use medi_rs::{MediCommand, medi_handler, medi_module};
use std::sync::Arc;

#[derive(Debug)]
pub enum CredentialError {
    NotFound,
    Conflict(String),
    Validation(String),
    Storage(String),
}

#[derive(Clone)]
pub struct CredentialRepositoryResource(
    pub Arc<dyn CredentialRepository>,
    pub Arc<dyn CredentialTypeCatalog>,
    pub Arc<dyn ProjectConfigRepository>,
    pub Arc<dyn CredentialModelCatalog>,
);

#[derive(MediCommand)]
#[medi_command(return_type = Vec<Credential>, error_type = CredentialError)]
pub struct ListCredentials;

#[derive(MediCommand)]
#[medi_command(return_type = Vec<CredentialTypeDescriptor>, error_type = CredentialError)]
pub struct ListCredentialTypes;

#[derive(MediCommand)]
#[medi_command(return_type = Vec<ModelDescriptor>, error_type = CredentialError)]
pub struct ListCredentialModels {
    pub id: String,
}

#[derive(MediCommand)]
#[medi_command(return_type = Credential, error_type = CredentialError)]
pub struct GetCredential {
    pub id: String,
}

#[derive(MediCommand)]
#[medi_command(return_type = Credential, error_type = CredentialError)]
pub struct SaveCredential {
    pub credential: Credential,
    pub secret: Option<String>,
}

#[derive(MediCommand)]
#[medi_command(error_type = CredentialError)]
pub struct DeleteCredential {
    pub id: String,
}

fn storage(error: String) -> CredentialError {
    CredentialError::Storage(error)
}

#[medi_handler]
async fn list_types_handler(
    repository: CredentialRepositoryResource,
    _: ListCredentialTypes,
) -> Result<Vec<CredentialTypeDescriptor>, CredentialError> {
    Ok(repository.1.list())
}

#[medi_handler]
async fn list_models_handler(
    repository: CredentialRepositoryResource,
    command: ListCredentialModels,
) -> Result<Vec<ModelDescriptor>, CredentialError> {
    let credential = repository
        .0
        .get(&command.id)
        .await
        .map_err(storage)?
        .ok_or(CredentialError::NotFound)?;
    if credential.status != CredentialStatus::Available {
        return Err(CredentialError::Conflict(
            "Credential is not available; reconnect or replace it first".into(),
        ));
    }
    let descriptor = repository
        .1
        .get(&credential.credential_type)
        .ok_or_else(|| CredentialError::Validation("Unsupported credential type".into()))?;
    if descriptor.provider.is_none() {
        return Err(CredentialError::Validation(
            "This credential type does not provide models".into(),
        ));
    }
    let secret = repository.0.secret(&command.id).await.map_err(storage)?;
    repository
        .3
        .list_models(&credential, secret.as_deref())
        .await
        .map_err(storage)
}

#[medi_handler]
async fn list_handler(
    repository: CredentialRepositoryResource,
    _: ListCredentials,
) -> Result<Vec<Credential>, CredentialError> {
    repository.0.list().await.map_err(storage)
}
#[medi_handler]
async fn get_handler(
    repository: CredentialRepositoryResource,
    command: GetCredential,
) -> Result<Credential, CredentialError> {
    repository
        .0
        .get(&command.id)
        .await
        .map_err(storage)?
        .ok_or(CredentialError::NotFound)
}
#[medi_handler]
async fn save_handler(
    repository: CredentialRepositoryResource,
    command: SaveCredential,
) -> Result<Credential, CredentialError> {
    command
        .credential
        .validate()
        .map_err(CredentialError::Validation)?;
    if repository
        .1
        .get(&command.credential.credential_type)
        .is_none()
    {
        return Err(CredentialError::Validation(
            "Unsupported credential type".into(),
        ));
    }
    if command.credential.credential_type.0 == "chatgpt_session" && command.secret.is_some() {
        return Err(CredentialError::Validation(
            "ChatGPT sessions must be created through device authentication".into(),
        ));
    }
    let existing = repository
        .0
        .get(&command.credential.id)
        .await
        .map_err(storage)?;
    if existing.is_none()
        && command.secret.is_none()
        && command.credential.credential_type.0 != "chatgpt_session"
    {
        return Err(CredentialError::Validation(
            "A secret is required when creating a credential".into(),
        ));
    }
    let mut credential = command.credential;
    credential.status = if command.secret.is_some() {
        CredentialStatus::Available
    } else {
        existing
            .as_ref()
            .map_or(CredentialStatus::MissingSecret, |item| item.status.clone())
    };
    repository
        .0
        .save(&credential, command.secret.as_deref())
        .await
        .map_err(storage)?;
    repository
        .0
        .get(&credential.id)
        .await
        .map_err(storage)?
        .ok_or(CredentialError::NotFound)
}
#[medi_handler]
async fn delete_handler(
    credentials: CredentialRepositoryResource,
    command: DeleteCredential,
) -> Result<(), CredentialError> {
    if credentials
        .0
        .get(&command.id)
        .await
        .map_err(storage)?
        .is_none()
    {
        return Err(CredentialError::NotFound);
    }
    let state = credentials
        .2
        .load()
        .await
        .map_err(CredentialError::Storage)?;
    if let crate::domain::project::ConfigState::Ready(snapshot) = state
        && let Some(agent) = snapshot
            .config
            .agents
            .iter()
            .find(|agent| agent.credential_id.as_deref() == Some(command.id.as_str()))
    {
        return Err(CredentialError::Conflict(format!(
            "Credential is referenced by agent {:?}; remove the reference before deleting it",
            agent.id
        )));
    }
    credentials.0.delete(&command.id).await.map_err(storage)
}

medi_module! {
    pub manifest credentials_feature;
    resources {
        credential_repository: crate::features::credentials::CredentialRepositoryResource;
    }
    commands {
        crate::features::credentials::ListCredentials => crate::features::credentials::list_handler;
        crate::features::credentials::ListCredentialTypes => crate::features::credentials::list_types_handler;
        crate::features::credentials::ListCredentialModels => crate::features::credentials::list_models_handler;
        crate::features::credentials::GetCredential => crate::features::credentials::get_handler;
        crate::features::credentials::SaveCredential => crate::features::credentials::save_handler;
        crate::features::credentials::DeleteCredential => crate::features::credentials::delete_handler;
    }
}
