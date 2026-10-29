pub mod api;
mod mediator;
mod routes;

use std::{path::PathBuf, sync::Arc};

use axum::{Json, Router, http::StatusCode, response::IntoResponse};
use baton_adapters::{
    credentials::{
        FilesystemCredentialRepository, LocalCredentialTypeCatalog,
        chatgpt_auth::ChatGptAuthManager, model_catalog::RigCredentialModelCatalog,
    },
    repositories::{
        filesystem_project_config::FilesystemProjectConfigRepository,
        filesystem_run::FilesystemRunRepository, filesystem_task::FilesystemTaskRepository,
    },
    runtime::ConfiguredRuntimeFactory,
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
    pub(crate) chatgpt_auth: ChatGptAuthManager,
}

impl AppState {
    pub fn new(project_dir: PathBuf) -> Self {
        let repository = FilesystemProjectConfigRepository::new(project_dir.clone());
        let runs = FilesystemRunRepository(project_dir.clone());
        // A running run cannot survive a process restart because adapters are in-process.
        // Convert it deterministically to a durable terminal record before serving requests.
        let _ = runs.recover_interrupted();
        let repository: Arc<dyn ProjectConfigRepository> = Arc::new(repository);
        let credential_store = FilesystemCredentialRepository::for_local_user()
            .expect("local credential storage path must be available");
        let auth_root = credential_store.root_path().to_path_buf();
        let chatgpt_auth = ChatGptAuthManager::new(auth_root.clone());
        let credentials: Arc<dyn baton_core::ports::credentials::CredentialRepository> =
            Arc::new(credential_store);
        let credential_types: Arc<dyn baton_core::ports::credentials::CredentialTypeCatalog> =
            Arc::new(LocalCredentialTypeCatalog);
        let credential_models: Arc<dyn baton_core::ports::credentials::CredentialModelCatalog> =
            Arc::new(RigCredentialModelCatalog);
        let task_repository: Arc<dyn TaskRepository> =
            Arc::new(FilesystemTaskRepository(project_dir.clone()));
        let run_repository: Arc<dyn RunRepository> =
            Arc::new(FilesystemRunRepository(project_dir.clone()));
        let runtime_factory: Arc<dyn AgentRuntimeFactory> = Arc::new(ConfiguredRuntimeFactory {
            project_dir: project_dir.clone(),
            credentials: credentials.clone(),
            auth_root,
        });
        let skills: Arc<dyn baton_core::ports::skills::SkillRepository> = Arc::new(
            baton_adapters::skills::FilesystemSkillRepository::new(project_dir.clone()),
        );
        let skill_downloader: Arc<dyn baton_core::ports::skill_downloader::SkillDownloader> =
            Arc::new(baton_adapters::skill_downloader::GithubSkillDownloader::default());
        let task_resources = TaskFeatureResources {
            tasks: task_repository,
            runs: run_repository.clone(),
            project_config: repository.clone(),
            runtime_factory: runtime_factory.clone(),
            skills: skills.clone(),
        };
        let run_resources = RunFeatureResources {
            runs: run_repository,
            runtime_factory,
        };
        let mediator = Arc::new(mediator::new(
            repository.clone(),
            credentials,
            credential_types,
            credential_models,
            task_resources,
            run_resources,
            skills,
            skill_downloader,
        ));
        Self {
            project_dir: Arc::new(project_dir),
            mediator,
            chatgpt_auth,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(routes::project::router())
        .merge(routes::agents::router())
        .merge(routes::skills::router())
        .merge(routes::credentials::router())
        .merge(routes::tasks::router())
        .merge(routes::pipelines::router())
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
