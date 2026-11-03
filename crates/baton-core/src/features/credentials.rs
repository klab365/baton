//! Credential-vault use cases. Responses contain metadata only.
use crate::{
    domain::credentials::{Credential, CredentialStatus},
    ports::{
        credentials::{
            CredentialAuthentication, CredentialConnectionTester, CredentialModelCatalog,
            CredentialRepository, CredentialTypeCatalog, CredentialTypeDescriptor, ModelDescriptor,
        },
        runs::RunRepository,
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
    pub Arc<dyn RunRepository>,
    pub Arc<dyn CredentialModelCatalog>,
    pub Arc<dyn CredentialConnectionTester>,
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
pub struct TestCredentialConnection {
    pub id: String,
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
fn validate_secret_input(
    device_session: bool,
    exists: bool,
    has_secret: bool,
) -> Result<(), CredentialError> {
    if device_session && has_secret {
        return Err(CredentialError::Validation(
            "Sessions must be created through device authentication".into(),
        ));
    }
    if !device_session && !exists && !has_secret {
        return Err(CredentialError::Validation(
            "A secret is required when creating a credential".into(),
        ));
    }
    Ok(())
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
    let descriptor = repository
        .1
        .get(&command.credential.credential_type)
        .ok_or_else(|| CredentialError::Validation("Unsupported credential type".into()))?;
    let device_session = descriptor.authentication == CredentialAuthentication::DeviceCode;
    let existing = repository
        .0
        .get(&command.credential.id)
        .await
        .map_err(storage)?;
    if existing
        .as_ref()
        .is_some_and(|old| old.credential_type != command.credential.credential_type)
    {
        return Err(CredentialError::Validation(
            "Credential type cannot be changed".into(),
        ));
    }
    validate_secret_input(device_session, existing.is_some(), command.secret.is_some())?;
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
async fn test_connection_handler(
    repository: CredentialRepositoryResource,
    command: TestCredentialConnection,
) -> Result<(), CredentialError> {
    let credential = repository
        .0
        .get(&command.id)
        .await
        .map_err(storage)?
        .ok_or(CredentialError::NotFound)?;
    let descriptor = repository
        .1
        .get(&credential.credential_type)
        .ok_or_else(|| CredentialError::Validation("Unsupported credential type".into()))?;
    if !descriptor.supports_test {
        return Err(CredentialError::Validation(
            "Connection testing is not supported for this credential type".into(),
        ));
    }
    if credential.status != CredentialStatus::Available {
        return Err(CredentialError::Conflict(
            "Credential is not available; reconnect or replace it first".into(),
        ));
    }
    let secret = repository.0.secret(&command.id).await.map_err(storage)?;
    repository
        .4
        .test_connection(&credential, secret.as_deref())
        .await
        .map_err(CredentialError::Conflict)
}

#[medi_handler]
async fn delete_handler(
    credentials: CredentialRepositoryResource,
    command: DeleteCredential,
) -> Result<(), CredentialError> {
    let _lease = credentials
        .0
        .acquire_delete_lease(&command.id)
        .map_err(CredentialError::Conflict)?;
    if credentials
        .0
        .get(&command.id)
        .await
        .map_err(storage)?
        .is_none()
    {
        return Err(CredentialError::NotFound);
    }
    let runs = credentials.2.list().map_err(CredentialError::Storage)?;
    if let Some(run) = runs
        .iter()
        .find(|run| running_run_uses_credential(run, &command.id))
    {
        return Err(CredentialError::Conflict(format!(
            "Credential is used by running run {:?}; wait for it to finish before deleting it",
            run.id
        )));
    }
    credentials.0.delete(&command.id).await.map_err(storage)
}

fn running_run_uses_credential(run: &crate::domain::pipeline::PipelineRun, id: &str) -> bool {
    if run.status != crate::domain::pipeline::Status::Running {
        return false;
    }
    // A legacy running record without a usable snapshot cannot be checked safely.
    let Some(config) = run.config_snapshot.as_ref() else {
        return true;
    };
    let Some(pipeline) = config
        .pipelines
        .iter()
        .find(|pipeline| pipeline.id == run.pipeline_id)
    else {
        return true;
    };
    pipeline
        .steps
        .iter()
        .filter(|step| !step.human)
        .any(|step| {
            config
                .agents
                .iter()
                .find(|agent| agent.id == step.agent_id)
                .is_none_or(|agent| agent.credential_id.as_deref() == Some(id))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pipeline::{PipelineRun, Status};

    fn run() -> PipelineRun {
        serde_json::from_value(serde_json::json!({
            "id": "run", "pipelineId": "pipeline", "task": "task", "startedAt": "now",
            "endedAt": null, "durationMs": null, "status": "running", "steps": [],
            "configSnapshot": {
                "schemaVersion": 7,
                "agents": [
                    {"id": "agent", "systemPrompt": "prompt", "credentialId": "used", "skillIds": []},
                    {"id": "other", "systemPrompt": "prompt", "credentialId": "not-in-pipeline", "skillIds": []}
                ],
                "pipelines": [{"id": "pipeline", "steps": [{"id": "step", "name": "Step", "agentId": "agent"}]}]
            }
        })).unwrap()
    }

    #[test]
    fn secret_validation_depends_on_authentication_kind() {
        assert!(validate_secret_input(true, false, false).is_ok());
        assert!(validate_secret_input(true, true, true).is_err());
        assert!(validate_secret_input(false, false, false).is_err());
        assert!(validate_secret_input(false, false, true).is_ok());
        assert!(validate_secret_input(false, true, false).is_ok());
    }

    #[test]
    fn only_running_pipelines_using_the_credential_block_deletion() {
        let mut run = run();
        assert!(running_run_uses_credential(&run, "used"));
        assert!(!running_run_uses_credential(&run, "not-in-pipeline"));
        assert!(!running_run_uses_credential(&run, "unrelated"));
        run.status = Status::WaitingHuman;
        assert!(!running_run_uses_credential(&run, "used"));
        run.status = Status::Succeeded;
        assert!(!running_run_uses_credential(&run, "used"));
        run.status = Status::Running;
        run.config_snapshot = None;
        assert!(running_run_uses_credential(&run, "unrelated"));
    }
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
        crate::features::credentials::TestCredentialConnection => crate::features::credentials::test_connection_handler;
    }
}
