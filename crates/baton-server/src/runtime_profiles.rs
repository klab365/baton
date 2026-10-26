use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};

async fn profiles(
    state: &AppState,
) -> Result<Vec<baton_core::runtime_profiles::RuntimeProfile>, ApiError> {
    let snapshot = baton_core::agents::list(state.repository.as_ref()).await?;
    Ok(baton_core::runtime_profiles::list(&snapshot.config))
}

async fn list(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!(profiles(&state).await?)))
}

/// Returns configuration only. Runtime availability and credentials are adapter concerns;
/// this endpoint deliberately does not inspect a host runtime or credential store.
async fn inspect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let profile = profiles(&state)
        .await?
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or(ApiError {
            status: StatusCode::NOT_FOUND,
            message: "Runtime profile not found".into(),
        })?;
    Ok(Json(serde_json::json!({"profile": profile})))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    revision: String,
    profile: baton_core::runtime_profiles::RuntimeProfile,
}

async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    if id != body.profile.id
        || id
            != baton_core::runtime_profiles::profile_id(
                &body.profile.runtime_kind,
                body.profile.model.as_ref(),
            )
    {
        return Err(ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "Profile ID must match runtime.provider.model".into(),
        });
    }
    let mut snapshot = baton_core::agents::list(state.repository.as_ref()).await?;
    if snapshot.revision != body.revision {
        return Err(ApiError::conflict(
            "Configuration changed; reload before saving".into(),
        ));
    }
    snapshot
        .config
        .runtime_profiles
        .retain(|profile| profile.id != id);
    snapshot.config.runtime_profiles.push(body.profile);
    snapshot.config.validate().map_err(|message| ApiError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        message,
    })?;
    state
        .repository
        .update(&snapshot.revision, &snapshot.config)
        .await
        .map_err(ApiError::conflict)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Revision>,
) -> Result<StatusCode, ApiError> {
    let mut snapshot = baton_core::agents::list(state.repository.as_ref()).await?;
    if snapshot.revision != body.revision {
        return Err(ApiError::conflict(
            "Configuration changed; reload before saving".into(),
        ));
    }
    if snapshot
        .config
        .agents
        .iter()
        .any(|agent| agent.runtime_profile_id == id)
    {
        return Err(ApiError::conflict(
            "Runtime profile is assigned to an agent".into(),
        ));
    }
    let count = snapshot.config.runtime_profiles.len();
    snapshot
        .config
        .runtime_profiles
        .retain(|profile| profile.id != id);
    if count == snapshot.config.runtime_profiles.len() {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: "Runtime profile not found".into(),
        });
    }
    state
        .repository
        .update(&snapshot.revision, &snapshot.config)
        .await
        .map_err(ApiError::conflict)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    revision: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/runtime-profiles", get(list))
        .route(
            "/api/runtime-profiles/{id}",
            get(inspect).put(save).delete(remove),
        )
}
