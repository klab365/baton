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
        .route("/api/runs/{id}/actions", post(human_action))
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
    if !matches!(run.status, Status::Running | Status::WaitingHuman) {
        return Err(ApiError::conflict(
            "Only running or waiting runs can be cancelled".into(),
        ));
    }
    run.status = Status::Cancelled;
    run.pending_actions.clear();
    for step in &mut run.steps {
        if matches!(step.status, Status::Running | Status::WaitingHuman) {
            step.status = Status::Cancelled;
        }
    }
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

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HumanAction {
    outcome: baton_core::project::Outcome,
    #[serde(default)]
    response: String,
}

async fn human_action(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(action): Json<HumanAction>,
) -> Result<Json<baton_core::orchestration::PipelineRun>, ApiError> {
    let guard = state.run_lock.lock().await;
    let repository = repository(&state);
    let run = repository.load(&id).map_err(ApiError::internal)?;
    if run.status != Status::WaitingHuman {
        return Err(ApiError::conflict("Run is not waiting for a human".into()));
    }
    let config = run
        .config_snapshot
        .as_ref()
        .ok_or_else(|| ApiError::internal("Missing run snapshot".into()))?;
    let mut runtime = crate::tasks::runtime_for(config, &run.pipeline_id).await?;
    let owner = run.task_id.clone().unwrap_or_else(|| run.id.clone());
    if !state.active_tasks.lock().await.insert(owner.clone()) {
        return Err(ApiError::conflict(
            "Run action is already in progress".into(),
        ));
    }
    drop(guard);
    let result = tokio::task::spawn_blocking(move || {
        baton_core::orchestration::resume_human(
            &id,
            action.outcome,
            &action.response,
            &mut runtime,
            &repository,
        )
    })
    .await;
    state.active_tasks.lock().await.remove(&owner);
    result
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map(Json)
        .map_err(|message| ApiError {
            status: StatusCode::BAD_REQUEST,
            message,
        })
}
