mod filesystem_repository;
mod pipelines;
mod project_routes;
pub mod run_repository;
mod tasks;

use std::{path::PathBuf, sync::Arc};

use axum::{Json, Router, http::StatusCode, response::IntoResponse};
use baton_core::project_config::ProjectConfigRepository;
use filesystem_repository::FilesystemProjectConfigRepository;

#[derive(Clone)]
pub struct AppState {
    project_dir: Arc<PathBuf>,
    repository: Arc<dyn ProjectConfigRepository>,
    task_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AppState {
    pub fn new(project_dir: PathBuf) -> Self {
        let repository = FilesystemProjectConfigRepository::new(project_dir.clone());
        Self {
            project_dir: Arc::new(project_dir),
            repository: Arc::new(repository),
            task_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(project_routes::router())
        .merge(tasks::router())
        .merge(pipelines::router())
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

struct ApiError {
    status: StatusCode,
    message: String,
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
