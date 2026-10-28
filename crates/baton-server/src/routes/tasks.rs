use super::*;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_core::features::tasks::{
    CreateTask, DeleteTask, GetTask, ListTaskRuns, ListTasks, StartTaskRun,
    UpdateTask as UpdateTaskCommand,
};
use baton_core::{
    domain::{
        pipeline::PipelineRun,
        tasks::{Task, TaskInput},
    },
    features::{pipeline_engine::utc_now, tasks::TaskRunStartError},
};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tasks", get(list).post(create))
        .route("/api/tasks/{id}", get(get_one).put(update).delete(delete))
        .route("/api/tasks/{id}/runs", get(list_runs).post(start_run))
}

fn now() -> String {
    utc_now()
}
fn bad(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: message.into(),
    }
}
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<Task>>, ApiError> {
    state
        .mediator
        .send(ListTasks)
        .await
        .map(Json)
        .map_err(ApiError::internal)
}
pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Task>, ApiError> {
    state
        .mediator
        .send(GetTask { id })
        .await
        .map(Json)
        .map_err(|message| ApiError {
            status: if message.contains("No such file") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            },
            message,
        })
}
pub async fn create(
    State(state): State<AppState>,
    Json(input): Json<TaskInput>,
) -> Result<(StatusCode, Json<Task>), ApiError> {
    let id = uuid::Uuid::now_v7().to_string();
    let task = state
        .mediator
        .send(CreateTask {
            id,
            input,
            now: now(),
        })
        .await
        .map_err(bad)?;
    Ok((StatusCode::CREATED, Json(task)))
}
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<TaskInput>,
) -> Result<Json<Task>, ApiError> {
    let task = state
        .mediator
        .send(UpdateTaskCommand {
            id,
            input,
            now: now(),
        })
        .await
        .map_err(|message| {
            if message.contains("active run") {
                ApiError::conflict(message)
            } else if message.contains("No such file") {
                ApiError {
                    status: StatusCode::NOT_FOUND,
                    message,
                }
            } else {
                bad(message)
            }
        })?;
    Ok(Json(task))
}
async fn list_runs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<PipelineRun>>, ApiError> {
    state
        .mediator
        .send(ListTaskRuns { task_id: id })
        .await
        .map_err(|message| {
            if message.contains("No such file") {
                ApiError {
                    status: StatusCode::NOT_FOUND,
                    message,
                }
            } else {
                ApiError::internal(message)
            }
        })
        .map(Json)
}

async fn start_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<PipelineRun>), ApiError> {
    let run_id = uuid::Uuid::now_v7().to_string();
    let run = state
        .mediator
        .send(StartTaskRun {
            task_id: id,
            run_id,
        })
        .await
        .map_err(|error| match error {
            TaskRunStartError::NotFound => ApiError {
                status: StatusCode::NOT_FOUND,
                message: "Task not found".into(),
            },
            TaskRunStartError::Conflict(message) => ApiError::conflict(message),
            TaskRunStartError::Infrastructure(message) => ApiError::internal(message),
            TaskRunStartError::Pipeline(message) => bad(message),
        })?;
    Ok((StatusCode::CREATED, Json(run)))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .mediator
        .send(DeleteTask { id })
        .await
        .map_err(ApiError::conflict)?;
    Ok(StatusCode::NO_CONTENT)
}
