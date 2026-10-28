use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_core::{
    domain::credentials::{Credential, CredentialStatus, CredentialType},
    features::credentials::{
        CredentialError, DeleteCredential, GetCredential, ListCredentialModels,
        ListCredentialTypes, ListCredentials, SaveCredential,
    },
};
use serde::Deserialize;

impl From<CredentialError> for ApiError {
    fn from(error: CredentialError) -> Self {
        let (status, message) = match error {
            CredentialError::NotFound => (StatusCode::NOT_FOUND, "Credential not found".into()),
            CredentialError::Conflict(message) => (StatusCode::CONFLICT, message),
            CredentialError::Validation(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
            CredentialError::Storage(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Local credential storage is unavailable; check directory permissions".into(),
            ),
        };
        Self { status, message }
    }
}
async fn list_types(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let types = state.mediator.send(ListCredentialTypes).await?;
    Ok(Json(serde_json::json!(types)))
}

async fn list_models(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let models = state.mediator.send(ListCredentialModels { id }).await?;
    Ok(Json(serde_json::json!(models)))
}

async fn list(State(state): State<AppState>) -> Result<Json<Vec<Credential>>, ApiError> {
    Ok(Json(state.mediator.send(ListCredentials).await?))
}
async fn inspect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Credential>, ApiError> {
    Ok(Json(state.mediator.send(GetCredential { id }).await?))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Save {
    name: String,
    credential_type: String,
    #[serde(default)]
    configuration: std::collections::BTreeMap<String, String>,
    secret: Option<String>,
}
async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Save>,
) -> Result<Json<Credential>, ApiError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let existing = state
        .mediator
        .send(GetCredential { id: id.clone() })
        .await
        .ok();
    let credential_type =
        CredentialType::new(body.credential_type).map_err(|message| ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message,
        })?;
    let credential = Credential {
        id,
        name: body.name,
        credential_type,
        configuration: body.configuration,
        created_at: existing.as_ref().map_or(now, |item| item.created_at),
        updated_at: now,
        status: CredentialStatus::MissingSecret,
    };
    Ok(Json(
        state
            .mediator
            .send(SaveCredential {
                credential,
                secret: body.secret,
            })
            .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authenticate {
    name: String,
}
async fn authenticate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Authenticate>,
) -> Result<Json<baton_adapters::credentials::chatgpt_auth::ChatGptAuthStatus>, ApiError> {
    if let Ok(existing) = state.mediator.send(GetCredential { id: id.clone() }).await
        && existing.credential_type.0 != "chatgpt_session"
    {
        return Err(ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "Device authentication is not supported for this credential type".into(),
        });
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let credential = Credential {
        id: id.clone(),
        name: body.name,
        credential_type: CredentialType::new("chatgpt_session").unwrap(),
        configuration: Default::default(),
        created_at: now,
        updated_at: now,
        status: CredentialStatus::MissingSecret,
    };
    state
        .mediator
        .send(SaveCredential {
            credential,
            secret: None,
        })
        .await?;
    let status = state.chatgpt_auth.start(id).map_err(|_| ApiError {
        status: StatusCode::CONFLICT,
        message: "Could not start ChatGPT authentication".into(),
    })?;
    Ok(Json(status))
}
async fn authentication_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let credential = state
        .mediator
        .send(GetCredential { id: id.clone() })
        .await?;
    if credential.credential_type.0 != "chatgpt_session" {
        return Err(ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "Not a ChatGPT session credential".into(),
        });
    }
    let status = state.chatgpt_auth.status(&id).unwrap_or_else(|| {
        baton_adapters::credentials::chatgpt_auth::ChatGptAuthStatus {
            status: if credential.status == CredentialStatus::Available {
                "available"
            } else {
                "not_started"
            }
            .into(),
            verification_uri: None,
            user_code: None,
            message: (credential.status != CredentialStatus::Available)
                .then(|| "Start ChatGPT authentication from Settings.".into()),
        }
    });
    Ok(Json(serde_json::to_value(status).unwrap()))
}
async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .mediator
        .send(DeleteCredential { id: id.clone() })
        .await?;
    state.chatgpt_auth.cancel(&id);
    Ok(StatusCode::NO_CONTENT)
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/credential-types", get(list_types))
        .route("/api/credentials", get(list))
        .route(
            "/api/credentials/{id}",
            get(inspect).put(save).delete(remove),
        )
        .route("/api/credentials/{id}/models", get(list_models))
        .route(
            "/api/credentials/{id}/authenticate",
            axum::routing::post(authenticate),
        )
        .route(
            "/api/credentials/{id}/authentication",
            get(authentication_status),
        )
}
