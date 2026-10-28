pub mod api;
mod mediator;
mod routes;

use std::{path::PathBuf, sync::Arc};

use axum::{Json, Router, http::StatusCode, response::IntoResponse};
use baton_adapters::{
    repositories::{
        filesystem_project_config::FilesystemProjectConfigRepository,
        filesystem_run::FilesystemRunRepository, filesystem_task::FilesystemTaskRepository,
    },
    runtime::ProfileRuntimeFactory,
};
use baton_core::{
    features::{runs::RunFeatureResources, tasks::TaskFeatureResources},
    ports::{
        project_config::ProjectConfigRepository, runs::RunRepository, runtime::AgentRuntimeFactory,
        tasks::TaskRepository,
    },
};

#[derive(Clone)]
pub struct AppState {
    project_dir: Arc<PathBuf>,
    mediator: Arc<mediator::ServerMediator>,
}

impl AppState {
    pub fn new(project_dir: PathBuf) -> Self {
        let repository = FilesystemProjectConfigRepository::new(project_dir.clone());
        let runs = FilesystemRunRepository(project_dir.clone());
        // A running run cannot survive a process restart because adapters are in-process.
        // Convert it deterministically to a durable terminal record before serving requests.
        let _ = runs.recover_interrupted();
        let repository: Arc<dyn ProjectConfigRepository> = Arc::new(repository);
        let task_repository: Arc<dyn TaskRepository> =
            Arc::new(FilesystemTaskRepository(project_dir.clone()));
        let run_repository: Arc<dyn RunRepository> =
            Arc::new(FilesystemRunRepository(project_dir.clone()));
        let runtime_factory: Arc<dyn AgentRuntimeFactory> = Arc::new(ProfileRuntimeFactory);
        let task_resources = TaskFeatureResources {
            tasks: task_repository,
            runs: run_repository.clone(),
            project_config: repository.clone(),
            runtime_factory: runtime_factory.clone(),
        };
        let run_resources = RunFeatureResources {
            runs: run_repository,
            runtime_factory,
        };
        let mediator = Arc::new(mediator::new(
            repository.clone(),
            task_resources,
            run_resources,
        ));
        Self {
            project_dir: Arc::new(project_dir),
            mediator,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(routes::project::router())
        .merge(routes::agents::router())
        .merge(routes::tasks::router())
        .merge(routes::pipelines::router())
        .merge(routes::runtime_profiles::router())
        .merge(routes::runs::router())
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
