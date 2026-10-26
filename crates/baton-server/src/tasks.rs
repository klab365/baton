use super::*;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use baton_adapters::filesystem_run_repository::FilesystemRunRepository;
use baton_core::orchestration::{Status, start_pipeline_for_task};
use baton_core::{
    project::{ConfigState, ProjectConfig},
    project_config::get_project_config,
    tasks::{Task, TaskInput},
};
use std::fs;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tasks", get(list).post(create))
        .route("/api/tasks/{id}", get(get_one).put(update).delete(delete))
        .route("/api/tasks/{id}/runs", get(list_runs).post(start_run))
}

fn now() -> String {
    baton_core::orchestration::utc_now()
}
fn bad(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: message.into(),
    }
}
fn io(error: std::io::Error) -> ApiError {
    ApiError {
        status: if error.kind() == std::io::ErrorKind::NotFound {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        },
        message: error.to_string(),
    }
}
fn path(state: &AppState, id: &str) -> Result<PathBuf, ApiError> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(bad("Invalid task ID"));
    }
    Ok(state
        .project_dir
        .join(".baton/tasks")
        .join(format!("{id}.json")))
}
fn task_repository(
    state: &AppState,
) -> baton_adapters::filesystem_task_repository::FilesystemTaskRepository {
    baton_adapters::filesystem_task_repository::FilesystemTaskRepository(
        (*state.project_dir).clone(),
    )
}
fn load(state: &AppState, id: &str) -> Result<Task, ApiError> {
    use baton_core::tasks::TaskRepository;
    task_repository(state).load(id).map_err(|e| {
        if e.contains("No such file") {
            ApiError {
                status: StatusCode::NOT_FOUND,
                message: e,
            }
        } else {
            ApiError::internal(e)
        }
    })
}
fn save(state: &AppState, task: &Task) -> Result<(), ApiError> {
    use baton_core::tasks::TaskRepository;
    task_repository(state)
        .save(task)
        .map_err(ApiError::internal)
}
async fn config(state: &AppState) -> Result<ProjectConfig, ApiError> {
    match get_project_config(state.repository.as_ref())
        .await
        .map_err(ApiError::internal)?
    {
        ConfigState::Ready(snapshot) => Ok(snapshot.config),
        _ => Err(bad("A valid project configuration is required")),
    }
}
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<Task>>, ApiError> {
    let _guard = state.task_lock.lock().await;
    let entries = match fs::read_dir(state.project_dir.join(".baton/tasks")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Json(vec![])),
        Err(e) => return Err(io(e)),
    };
    let mut tasks = vec![];
    for entry in entries {
        let entry = entry.map_err(io)?;
        if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
            tasks.push(load(
                &state,
                entry
                    .path()
                    .file_stem()
                    .unwrap()
                    .to_str()
                    .ok_or_else(|| bad("Invalid task filename"))?,
            )?);
        }
    }
    tasks.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(Json(tasks))
}
pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Task>, ApiError> {
    let _guard = state.task_lock.lock().await;
    Ok(Json(load(&state, &id)?))
}
pub async fn create(
    State(state): State<AppState>,
    Json(input): Json<TaskInput>,
) -> Result<(StatusCode, Json<Task>), ApiError> {
    let _guard = state.task_lock.lock().await;
    let config = config(&state).await?;
    let mut id = uuid::Uuid::now_v7().to_string();
    while path(&state, &id)?.exists() {
        id.push('x');
    }
    let task = Task::create(id, input, config.clone(), now()).map_err(bad)?;
    save(&state, &task)?;
    Ok((StatusCode::CREATED, Json(task)))
}
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<TaskInput>,
) -> Result<Json<Task>, ApiError> {
    let _guard = state.task_lock.lock().await;
    load(&state, &id)?;
    let config = config(&state).await?;
    let _run_guard = state.run_lock.lock().await;
    let task = baton_core::tasks::update_task(
        &task_repository(&state),
        &FilesystemRunRepository((*state.project_dir).clone()),
        &id,
        input,
        config,
        now(),
    )
    .map_err(|message| {
        if message.contains("active run") {
            ApiError::conflict(message)
        } else {
            bad(message)
        }
    })?;
    Ok(Json(task))
}
async fn list_runs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<baton_core::orchestration::PipelineRun>>, ApiError> {
    let _guard = state.run_lock.lock().await;
    load(&state, &id)?;
    let runs = FilesystemRunRepository((*state.project_dir).clone())
        .list()
        .map_err(ApiError::internal)?;
    Ok(Json(
        runs.into_iter()
            .filter(|run| run.task_id.as_deref() == Some(&id))
            .collect(),
    ))
}

async fn start_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<baton_core::orchestration::PipelineRun>), ApiError> {
    let _guard = state.run_lock.lock().await;
    let task = load(&state, &id)?;
    let repository = FilesystemRunRepository((*state.project_dir).clone());
    if repository
        .list()
        .map_err(ApiError::internal)?
        .iter()
        .any(|run| {
            run.task_id.as_deref() == Some(&id)
                && matches!(run.status, Status::Running | Status::WaitingHuman)
        })
    {
        return Err(ApiError::conflict(
            "This task already has an active run".into(),
        ));
    }
    let run_id = uuid::Uuid::now_v7().to_string();
    let prompt = format!(
        "{}\n\nAcceptance criteria:\n{}",
        task.description, task.acceptance_criteria,
    );
    let config = task.config_snapshot;
    let mut credentials = runtime_for(&config, &task.pipeline_id).await?;
    if !state.active_tasks.lock().await.insert(id.clone()) {
        return Err(ApiError::conflict(
            "This task already has an active run".into(),
        ));
    }
    let pipeline_id = task.pipeline_id;
    let task_id = task.id;
    // Do not hold the control-plane lock while an adapter runs: cancellation
    // reads the durable record and is intentionally allowed to proceed.
    drop(_guard);
    let result = tokio::task::spawn_blocking(move || {
        start_pipeline_for_task(
            &config,
            &pipeline_id,
            &prompt,
            &run_id,
            Some(task_id),
            &mut credentials,
            &repository,
        )
    })
    .await;
    state.active_tasks.lock().await.remove(&id);
    let run = result
        .map_err(|error| ApiError::internal(error.to_string()))?
        .map_err(bad)?;
    Ok((StatusCode::CREATED, Json(run)))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let _task_guard = state.task_lock.lock().await;
    let _run_guard = state.run_lock.lock().await;
    baton_core::tasks::delete_task(
        &task_repository(&state),
        &FilesystemRunRepository((*state.project_dir).clone()),
        &id,
    )
    .map_err(ApiError::conflict)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn runtime_for(
    config: &ProjectConfig,
    pipeline_id: &str,
) -> Result<baton_adapters::ProfileRuntime, ApiError> {
    config.validate().map_err(bad)?;
    if !config.pipelines.iter().any(|p| p.id == pipeline_id) {
        return Err(bad("Selected pipeline does not exist"));
    }
    Ok(baton_adapters::ProfileRuntime::new(config.clone()))
}
