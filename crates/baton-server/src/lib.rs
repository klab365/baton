mod agents;
mod pipelines;
mod project_routes;
mod runs;
mod runtime_profiles;
mod tasks;

use std::{collections::HashSet, path::PathBuf, sync::Arc};

use axum::{Json, Router, http::StatusCode, response::IntoResponse};
use baton_adapters::{
    filesystem_project_config_repository::FilesystemProjectConfigRepository,
    filesystem_run_repository::FilesystemRunRepository,
};
use baton_core::project_config::ProjectConfigRepository;

#[derive(Clone)]
pub struct AppState {
    project_dir: Arc<PathBuf>,
    repository: Arc<dyn ProjectConfigRepository>,
    task_lock: Arc<tokio::sync::Mutex<()>>,
    run_lock: Arc<tokio::sync::Mutex<()>>,
    active_tasks: Arc<tokio::sync::Mutex<HashSet<String>>>,
}

impl AppState {
    pub fn new(project_dir: PathBuf) -> Self {
        let repository = FilesystemProjectConfigRepository::new(project_dir.clone());
        let runs = FilesystemRunRepository(project_dir.clone());
        // A running run cannot survive a process restart because adapters are in-process.
        // Convert it deterministically to a durable terminal record before serving requests.
        let _ = runs.recover_interrupted();
        Self {
            project_dir: Arc::new(project_dir),
            repository: Arc::new(repository),
            task_lock: Arc::new(tokio::sync::Mutex::new(())),
            run_lock: Arc::new(tokio::sync::Mutex::new(())),
            active_tasks: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(project_routes::router())
        .merge(agents::router())
        .merge(tasks::router())
        .merge(pipelines::router())
        .merge(runtime_profiles::router())
        .merge(runs::router())
        .with_state(state)
}

pub async fn serve(project_dir: PathBuf, host: &str, port: u16) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind((host, port)).await?;
    axum::serve(listener, router(AppState::new(project_dir)))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}
impl ApiError {
    fn conflict(message: String) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message,
        }
    }
    fn internal(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}
