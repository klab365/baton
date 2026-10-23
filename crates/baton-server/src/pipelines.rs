use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_core::{
    pipelines::{self, Mutation, PipelineError},
    project::Pipeline,
};
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
    let snapshot = pipelines::list(state.repository.as_ref()).await?;
    Ok(Json(
        serde_json::json!({"revision": snapshot.revision, "pipelines": snapshot.config.pipelines, "agents": snapshot.config.agents}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    revision: String,
    pipeline: Pipeline,
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
    pipelines::mutate(
        state.repository.as_ref(),
        &body.revision,
        Mutation::Create(body.pipeline),
    )
    .await?;
    Ok(StatusCode::CREATED)
}
async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Save>,
) -> Result<StatusCode, ApiError> {
    pipelines::mutate(
        state.repository.as_ref(),
        &body.revision,
        Mutation::Update(id, body.pipeline),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Delete>,
) -> Result<StatusCode, ApiError> {
    pipelines::mutate(
        state.repository.as_ref(),
        &body.revision,
        Mutation::Delete(id),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snapshot = pipelines::list(state.repository.as_ref()).await?;
    let pipeline = snapshot
        .config
        .pipelines
        .into_iter()
        .find(|item| item.id == id)
        .ok_or(PipelineError::NotFound)?;
    Ok(Json(
        serde_json::json!({"revision": snapshot.revision, "pipeline": pipeline}),
    ))
}
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/pipelines", get(list).post(create))
        .route(
            "/api/pipelines/{id}",
            get(detail).put(update).delete(delete),
        )
}
