use super::*;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use baton_core::{
    domain::project::ConfigState,
    features::project_config::{GetProjectConfig, InitializeProject},
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
        config: crate::api::dto::ProjectConfig,
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
    let response = match state
        .mediator
        .send(GetProjectConfig)
        .await
        .map_err(ApiError::internal)?
    {
        ConfigState::Missing => ProjectResponse::Missing { name, directory },
        ConfigState::Ready(config) => ProjectResponse::Ready {
            name,
            directory,
            config: config.config.into(),
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
    state
        .mediator
        .send(InitializeProject)
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

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/project", get(status))
        .route("/api/project/initialize", post(initialize))
}
