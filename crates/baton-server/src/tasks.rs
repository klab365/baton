use super::*;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
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
fn load(state: &AppState, id: &str) -> Result<Task, ApiError> {
    serde_json::from_slice(&fs::read(path(state, id)?).map_err(io)?)
        .map_err(|e| ApiError::internal(e.to_string()))
}
fn save(state: &AppState, task: &Task) -> Result<(), ApiError> {
    use std::io::Write;
    let path = path(state, &task.id)?;
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent).map_err(io)?;
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary).map_err(io)?;
    file.write_all(
        &serde_json::to_vec_pretty(task).map_err(|e| ApiError::internal(e.to_string()))?,
    )
    .map_err(io)?;
    file.sync_all().map_err(io)?;
    fs::rename(temporary, &path).map_err(io)?;
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(io)
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
    let task = Task::create(id, input, config, now()).map_err(bad)?;
    save(&state, &task)?;
    Ok((StatusCode::CREATED, Json(task)))
}
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<TaskInput>,
) -> Result<Json<Task>, ApiError> {
    let _guard = state.task_lock.lock().await;
    let mut task = load(&state, &id)?;
    task.update(input, config(&state).await?, now())
        .map_err(bad)?;
    save(&state, &task)?;
    Ok(Json(task))
}
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let _guard = state.task_lock.lock().await;
    fs::remove_file(path(&state, &id)?).map_err(io)?;
    Ok(StatusCode::NO_CONTENT)
}
