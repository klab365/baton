mod filesystem_repository;
pub mod run_repository;

use std::{path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use baton_core::{
    project::{ConfigState, ProjectConfig},
    project_config::{ProjectConfigRepository, get_project_config, initialize_project},
};
use filesystem_repository::FilesystemProjectConfigRepository;

#[derive(Clone)]
pub struct AppState {
    project_dir: Arc<PathBuf>,
    repository: Arc<dyn ProjectConfigRepository>,
}

impl AppState {
    pub fn new(project_dir: PathBuf) -> Self {
        let repository = FilesystemProjectConfigRepository::new(project_dir.clone());
        Self {
            project_dir: Arc::new(project_dir),
            repository: Arc::new(repository),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
enum ProjectResponse {
    Missing {
        name: String,
        directory: PathBuf,
    },
    Ready {
        name: String,
        directory: PathBuf,
        config: ProjectConfig,
    },
    Invalid {
        name: String,
        directory: PathBuf,
        message: String,
    },
}

fn project_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
        .to_owned()
}

async fn project_status(State(state): State<AppState>) -> Result<Json<ProjectResponse>, ApiError> {
    let directory = state.project_dir.as_ref().clone();
    let name = project_name(&directory);
    let response = match get_project_config(state.repository.as_ref())
        .await
        .map_err(ApiError::internal)?
    {
        ConfigState::Missing => ProjectResponse::Missing { name, directory },
        ConfigState::Ready(config) => ProjectResponse::Ready {
            name,
            directory,
            config: config.config,
        },
        ConfigState::Invalid(message) => ProjectResponse::Invalid {
            name,
            directory,
            message,
        },
    };
    Ok(Json(response))
}

async fn initialize(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    initialize_project(state.repository.as_ref())
        .await
        .map_err(|message| {
            if message.contains("already exists") {
                ApiError::conflict(message)
            } else {
                ApiError::internal(message)
            }
        })?;
    Ok(StatusCode::CREATED)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/project", get(project_status))
        .route("/api/project/initialize", post(initialize))
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
