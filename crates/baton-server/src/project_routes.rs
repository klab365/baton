use super::*;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use baton_core::{
    project::{ConfigState, ProjectConfig},
    project_config::{get_project_config, initialize_project},
};

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

async fn status(State(state): State<AppState>) -> Result<Json<ProjectResponse>, ApiError> {
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

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/project", get(status))
        .route("/api/project/initialize", post(initialize))
}
