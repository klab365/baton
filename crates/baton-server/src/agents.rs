use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_core::{
    agents::{self, AgentError, Mutation},
    project::Agent,
};
use serde::Deserialize;

impl From<AgentError> for ApiError {
    fn from(error: AgentError) -> Self {
        let (status, message) = match error {
            AgentError::NotFound => (StatusCode::NOT_FOUND, "Agent not found".into()),
            AgentError::Validation(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
            AgentError::Conflict(message) => (StatusCode::CONFLICT, message),
            AgentError::Storage(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        Self { status, message }
    }
}

async fn list(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let snapshot = agents::list(state.repository.as_ref()).await?;
    let profiles = baton_core::runtime_profiles::list(&snapshot.config);
    Ok(Json(serde_json::json!({
        "revision": snapshot.revision,
        "agents": snapshot.config.agents,
        "skills": snapshot.config.skills,
        "runtimeProfiles": profiles,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    revision: String,
    agent: Agent,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delete {
    revision: String,
}

async fn mutate(
    state: AppState,
    revision: String,
    mutation: Mutation,
) -> Result<StatusCode, ApiError> {
    let snapshot = agents::list(state.repository.as_ref()).await?;
    agents::mutate(state.repository.as_ref(), &revision, mutation, |id| {
        baton_core::runtime_profiles::resolve(&snapshot.config, id).is_ok()
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create(
    State(state): State<AppState>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    mutate(state, body.revision, Mutation::Create(body.agent)).await
}
async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    mutate(state, body.revision, Mutation::Update(id, body.agent)).await
}
async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Delete>,
) -> Result<StatusCode, ApiError> {
    mutate(state, body.revision, Mutation::Delete(id)).await
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/agents", get(list).post(create))
        .route(
            "/api/agents/{id}",
            axum::routing::put(update).delete(delete),
        )
}
