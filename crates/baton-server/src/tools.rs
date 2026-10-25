use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_core::tools::{self, ToolError};

impl From<ToolError> for ApiError {
    fn from(error: ToolError) -> Self {
        let (status, message) = match error {
            ToolError::NotFound => (StatusCode::NOT_FOUND, "Tool is not enabled".into()),
            ToolError::Validation(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
            ToolError::Conflict(message) => (StatusCode::CONFLICT, message),
            ToolError::Storage(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        Self { status, message }
    }
}

async fn list(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let snapshot = tools::list(state.repository.as_ref()).await?;
    Ok(Json(serde_json::json!({
        "revision": snapshot.revision,
        "enabled": snapshot.config.tools,
        "catalog": tools::definitions(),
    })))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    revision: String,
}
async fn enable(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Change>,
) -> Result<StatusCode, ApiError> {
    tools::enable(state.repository.as_ref(), &body.revision, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn disable(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Change>,
) -> Result<StatusCode, ApiError> {
    tools::disable(state.repository.as_ref(), &body.revision, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub fn router() -> Router<AppState> {
    Router::new().route("/api/tools", get(list)).route(
        "/api/tools/{id}",
        axum::routing::post(enable).delete(disable),
    )
}
