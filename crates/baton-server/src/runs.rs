use crate::{ApiError, AppState, StatusCode};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use baton_adapters::filesystem_run_repository::FilesystemRunRepository;
use baton_core::orchestration::{RunEvent, RunRepository, Status};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/runs", get(list))
        .route("/api/runs/{id}", get(detail))
        .route("/api/runs/{id}/cancel", post(cancel))
}
fn repository(state: &AppState) -> FilesystemRunRepository {
    FilesystemRunRepository((*state.project_dir).clone())
}
async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<baton_core::orchestration::PipelineRun>>, ApiError> {
    let _guard = state.run_lock.lock().await;
    Ok(Json(repository(&state).list().map_err(ApiError::internal)?))
}
async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<baton_core::orchestration::PipelineRun>, ApiError> {
    let _guard = state.run_lock.lock().await;
    repository(&state)
        .load(&id)
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
) -> Result<Json<baton_core::orchestration::PipelineRun>, ApiError> {
    let _guard = state.run_lock.lock().await;
    let repository = repository(&state);
    let mut run = repository.load(&id).map_err(ApiError::internal)?;
    if run.status != Status::Running {
        return Err(ApiError::conflict(
            "Only running runs can be cancelled".into(),
        ));
    }
    run.status = Status::Cancelled;
    run.ended_at = Some(baton_core::orchestration::utc_now());
    run.duration_ms = None;
    let event = RunEvent {
        run_id: run.id.clone(),
        status: Status::Cancelled,
        occurred_at: baton_core::orchestration::utc_now(),
        message: Some("Cancellation requested".into()),
    };
    run.events.push(event.clone());
    repository.save(&run).map_err(ApiError::internal)?;
    repository.publish(&event).map_err(ApiError::internal)?;
    Ok(Json(run))
}
