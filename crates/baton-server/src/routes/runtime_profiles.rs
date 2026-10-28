use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_core::features::runtime_profiles::{
    GetRuntimeProfile, ListRuntimeProfiles, MutateRuntimeProfile,
    Mutation as RuntimeProfileMutation, RuntimeProfileError,
};
use serde::Deserialize;

impl From<RuntimeProfileError> for ApiError {
    fn from(error: RuntimeProfileError) -> Self {
        let (status, message) = match error {
            RuntimeProfileError::NotFound => {
                (StatusCode::NOT_FOUND, "Runtime profile not found".into())
            }
            RuntimeProfileError::Validation(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
            RuntimeProfileError::Conflict(message) => (StatusCode::CONFLICT, message),
            RuntimeProfileError::Storage(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        Self { status, message }
    }
}

async fn list(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let profiles: Vec<crate::api::dto::RuntimeProfile> = state
        .mediator
        .send(ListRuntimeProfiles)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(serde_json::json!(profiles)))
}

async fn inspect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let profile: crate::api::dto::RuntimeProfile =
        state.mediator.send(GetRuntimeProfile { id }).await?.into();
    Ok(Json(serde_json::json!({"profile": profile})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    revision: String,
    profile: crate::api::dto::RuntimeProfile,
}

async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    if id != body.profile.id {
        return Err(ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "Profile ID must match runtime.provider.model".into(),
        });
    }
    state
        .mediator
        .send(MutateRuntimeProfile {
            revision: body.revision,
            mutation: RuntimeProfileMutation::Save(body.profile.into()),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    revision: String,
}

async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Revision>,
) -> Result<StatusCode, ApiError> {
    state
        .mediator
        .send(MutateRuntimeProfile {
            revision: body.revision,
            mutation: RuntimeProfileMutation::Delete(id),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/runtime-profiles", get(list))
        .route(
            "/api/runtime-profiles/{id}",
            get(inspect).put(save).delete(remove),
        )
}
