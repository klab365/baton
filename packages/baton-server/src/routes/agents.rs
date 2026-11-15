use crate::api::dto::{Agent as AgentDto, PiModelOption};
use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_core::features::agents::AgentError;
use baton_core::features::agents::{ListAgents, MutateAgent, Mutation};
use baton_core::features::pi_models::{ListPiModels, PiModelError};
use serde::Deserialize;

impl From<PiModelError> for ApiError {
    fn from(error: PiModelError) -> Self {
        match error {
            PiModelError::Validation(message) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message,
            },
            PiModelError::Unavailable(message) => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message,
            },
        }
    }
}

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

async fn list_pi_models(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<Json<Vec<PiModelOption>>, ApiError> {
    let models = state.mediator.send(ListPiModels { provider }).await?;
    Ok(Json(
        models
            .into_iter()
            .map(|model| PiModelOption {
                id: model.model.clone(),
                label: model.model,
            })
            .collect(),
    ))
}

async fn list(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let snapshot = state.mediator.send(ListAgents).await?;
    Ok(Json(serde_json::json!({
        "revision": snapshot.revision,
        "agents": snapshot.config.agents.into_iter().map(AgentDto::from).collect::<Vec<_>>(),
        "skills": state.mediator.send(baton_core::features::skills::ListSkills).await?,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    revision: String,
    agent: AgentDto,
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
    state
        .mediator
        .send(MutateAgent { revision, mutation })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create(
    State(state): State<AppState>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    mutate(state, body.revision, Mutation::Create(body.agent.into())).await
}
async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    mutate(
        state,
        body.revision,
        Mutation::Update(id, body.agent.into()),
    )
    .await
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
        .route("/api/pi/models/{provider}", get(list_pi_models))
        .route("/api/agents", get(list).post(create))
        .route(
            "/api/agents/{id}",
            axum::routing::put(update).delete(delete),
        )
}
