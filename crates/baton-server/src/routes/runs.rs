use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use baton_core::domain::pipeline::PipelineRun;
use baton_core::features::runs::{CancelRun, GetRun, ListRuns, RespondToRun};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/runs", get(list))
        .route("/api/runs/{id}", get(detail))
        .route("/api/runs/{id}/cancel", post(cancel))
        .route("/api/runs/{id}/actions", post(human_action))
}

async fn list(State(state): State<AppState>) -> Result<Json<Vec<PipelineRun>>, ApiError> {
    state
        .mediator
        .send(ListRuns)
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PipelineRun>, ApiError> {
    state
        .mediator
        .send(GetRun { id })
        .await
        .map(Json)
        .map_err(|error| ApiError {
            status: if error.contains("No such file") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            },
            message: error,
        })
}

async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PipelineRun>, ApiError> {
    state
        .mediator
        .send(CancelRun { id })
        .await
        .map(Json)
        .map_err(|message| {
            if message.starts_with("Only running or waiting") {
                ApiError::conflict(message)
            } else {
                ApiError::internal(message)
            }
        })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HumanAction {
    outcome: baton_core::domain::project::Outcome,
    #[serde(default)]
    response: String,
}

async fn human_action(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(action): Json<HumanAction>,
) -> Result<Json<PipelineRun>, ApiError> {
    state
        .mediator
        .send(RespondToRun {
            id,
            outcome: action.outcome,
            response: action.response,
        })
        .await
        .map(Json)
        .map_err(|message| {
            if message.contains("Run action is already in progress")
                || message.contains("not waiting for a human")
            {
                ApiError::conflict(message)
            } else {
                ApiError {
                    status: StatusCode::BAD_REQUEST,
                    message,
                }
            }
        })
}
