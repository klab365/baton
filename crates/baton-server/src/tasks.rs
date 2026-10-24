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
    let task = Task::create(id, input, config.clone(), now()).map_err(bad)?;
    crate::providers::ensure_pipeline_available(&config, &task.pipeline_id).await?;
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
    let config = config(&state).await?;
    task.update(input, config.clone(), now()).map_err(bad)?;
    crate::providers::ensure_pipeline_available(&config, &task.pipeline_id).await?;
    save(&state, &task)?;
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
        .any(|run| run.task_id.as_deref() == Some(&id) && run.status == Status::Running)
    {
        return Err(ApiError::conflict(
            "This task already has an active run".into(),
        ));
    }
    if !state.active_tasks.lock().await.insert(id.clone()) {
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
    let mut credentials = std::collections::HashMap::new();
    let pipeline = config
        .pipelines
        .iter()
        .find(|pipeline| pipeline.id == task.pipeline_id)
        .ok_or_else(|| bad("Selected pipeline does not exist"))?;
    for step in &pipeline.steps {
        let agent = config
            .agents
            .iter()
            .find(|agent| agent.id == step.agent_id)
            .ok_or_else(|| bad("Pipeline references unknown agent"))?;
        if credentials.contains_key(&agent.model.provider) {
            continue;
        }
        if agent.model.provider == "fake" {
            credentials.insert("fake".to_owned(), (None, None));
            continue;
        }
        let (provider, key) = crate::providers::execution_provider(&agent.model.provider).await?;
        credentials.insert(provider.id.clone(), (Some(provider), key));
    }
    let pipeline_id = task.pipeline_id;
    let task_id = task.id;
    // Do not hold the control-plane lock while an adapter runs: cancellation
    // reads the durable record and is intentionally allowed to proceed.
    drop(_guard);
    let result = tokio::task::spawn_blocking(move || {
        struct ConfiguredRuntime(
            std::collections::HashMap<
                String,
                (Option<baton_core::providers::Provider>, Option<String>),
            >,
        );
        impl baton_core::orchestration::AgentRuntime for ConfiguredRuntime {
            fn execute(
                &mut self,
                agent: &baton_core::project::Agent,
                input: &str,
            ) -> Result<String, String> {
                let (provider, key) = self
                    .0
                    .get(&agent.model.provider)
                    .ok_or_else(|| "Configured provider not found".to_owned())?;
                let Some(provider) = provider else {
                    return baton_adapters::FakeRuntime::default().execute(agent, input);
                };
                match provider.kind.as_str() {
                    "openai" => baton_adapters::OpenAiRigRuntime.execute_with_provider(
                        agent,
                        input,
                        provider.base_url.as_deref(),
                        key.as_deref().ok_or_else(|| {
                            "OpenAI API key is unavailable; reconnect provider in Settings"
                                .to_owned()
                        })?,
                    ),
                    "chatgpt" => baton_adapters::ChatGptRigRuntime::new(
                        crate::providers::token_dir(&provider.id)
                            .map_err(|_| "Provider token storage is unavailable".to_owned())?,
                    )
                    .execute_configured(agent, input, false),
                    _ => Err("Unsupported configured provider".into()),
                }
            }
        }
        start_pipeline_for_task(
            &config,
            &pipeline_id,
            &prompt,
            &run_id,
            Some(task_id),
            &mut ConfiguredRuntime(credentials),
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
    let _guard = state.task_lock.lock().await;
    fs::remove_file(path(&state, &id)?).map_err(io)?;
    Ok(StatusCode::NO_CONTENT)
}
