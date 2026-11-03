pub mod api;
mod mediator;
mod routes;

use std::{path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    extract::OriginalUri,
    http::{StatusCode, header},
    response::IntoResponse,
};

/// Version shared by the CLI and the running web server.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

async fn web_fallback(OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain")],
            &b"Not found"[..],
        );
    }
    let asset = embedded_asset(path);
    if asset.is_none()
        && (path.starts_with("_app/")
            || path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.contains('.')))
    {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain")],
            &b"Not found"[..],
        );
    }
    let content_type = if asset.is_none() || path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else {
        "application/octet-stream"
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        asset.unwrap_or_else(|| embedded_asset("index.html").expect("embedded index")),
    )
}
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
        .route(
            "/api/version",
            axum::routing::get(|| async { Json(serde_json::json!({ "version": VERSION })) }),
        )
        .merge(routes::project::router())
        .merge(routes::agents::router())
        .merge(routes::skills::router())
        .merge(routes::credentials::router())
        .merge(routes::tasks::router())
        .merge(routes::pipelines::router())
        .merge(routes::runs::router())
        .fallback(web_fallback)
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

#[cfg(test)]
mod web_tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    #[ignore = "requires a web build embedded via BATON_WEB_DIR"]
    async fn serves_embedded_web_build() {
        let app = Router::new().fallback(web_fallback);
        let response = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        let asset = html
            .split("href=\"/_app/immutable/entry/")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("index must reference an embedded SvelteKit entry");
        let uri = format!("/_app/immutable/entry/{asset}");
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/javascript")
        );
        assert!(
            !axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn serves_spa_routes_without_masking_missing_api_or_assets() {
        let app = Router::new().fallback(web_fallback);
        for (path, status) in [
            ("/", StatusCode::OK),
            ("/tasks/example", StatusCode::OK),
            ("/api/missing", StatusCode::NOT_FOUND),
            ("/_app/missing.js", StatusCode::NOT_FOUND),
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{path}");
        }
    }
}
