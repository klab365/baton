use crate::api::dto::{Agent as AgentDto, Pipeline as PipelineDto};
use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_core::features::pipelines::{ListPipelines, MutatePipeline};
use baton_core::features::pipelines::{Mutation, PipelineError};
use serde::Deserialize;

impl From<PipelineError> for ApiError {
    fn from(error: PipelineError) -> Self {
        let (status, message) = match error {
            PipelineError::NotFound => (StatusCode::NOT_FOUND, "Pipeline not found".into()),
            PipelineError::Validation(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
            PipelineError::Conflict(message) => (StatusCode::CONFLICT, message),
            PipelineError::Storage(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        Self { status, message }
    }
}

async fn list(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let snapshot = state.mediator.send(ListPipelines).await?;
    Ok(Json(
        serde_json::json!({"revision": snapshot.revision, "pipelines": snapshot.config.pipelines.into_iter().map(PipelineDto::from).collect::<Vec<_>>(), "agents": snapshot.config.agents.into_iter().map(AgentDto::from).collect::<Vec<_>>()}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    revision: String,
    pipeline: PipelineDto,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delete {
    revision: String,
}

async fn create(
    State(state): State<AppState>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    state
        .mediator
        .send(MutatePipeline {
            revision: body.revision,
            mutation: Mutation::Create(body.pipeline.into()),
        })
        .await?;
    Ok(StatusCode::CREATED)
}
async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    state
        .mediator
        .send(MutatePipeline {
            revision: body.revision,
            mutation: Mutation::Update(id, body.pipeline.into()),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Delete>,
) -> Result<StatusCode, ApiError> {
    state
        .mediator
        .send(MutatePipeline {
            revision: body.revision,
            mutation: Mutation::Delete(id),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snapshot = state.mediator.send(ListPipelines).await?;
    let pipeline = snapshot
        .config
        .pipelines
        .into_iter()
        .find(|item| item.id == id)
        .ok_or(PipelineError::NotFound)?;
    Ok(Json(
        serde_json::json!({"revision": snapshot.revision, "pipeline": PipelineDto::from(pipeline)}),
    ))
}
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/pipelines", get(list).post(create))
        .route(
            "/api/pipelines/{id}",
            get(detail).put(update).delete(delete),
        )
}
