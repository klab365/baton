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
        chatgpt_auth::ChatGptAuthManager, connection_test::LocalConnectionTester,
        copilot_auth::CopilotAuthManager, model_catalog::RigCredentialModelCatalog,
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
    pub(crate) copilot_auth: CopilotAuthManager,
    pub(crate) credential_operations: Arc<tokio::sync::Mutex<()>>,
}

impl AppState {
    pub fn new(project_dir: PathBuf) -> Self {
        let repository = FilesystemProjectConfigRepository::new(project_dir.clone());
        let runs = FilesystemRunRepository(project_dir.clone());
        // A running run cannot survive a process restart because adapters are in-process.
        // Convert it deterministically to a durable terminal record before serving requests.
        let _ = runs.recover_interrupted();
        let repository: Arc<dyn ProjectConfigRepository> = Arc::new(repository);
        let credential_store = FilesystemCredentialRepository::for_project(&project_dir)
            .expect("local credential storage path must be available");
        let auth_root = credential_store.root_path().to_path_buf();
        let chatgpt_auth = ChatGptAuthManager::new(auth_root.clone());
        let copilot_auth = CopilotAuthManager::new(auth_root.clone());
        let credentials: Arc<dyn baton_core::ports::credentials::CredentialRepository> =
            Arc::new(credential_store);
        let credential_types: Arc<dyn baton_core::ports::credentials::CredentialTypeCatalog> =
            Arc::new(LocalCredentialTypeCatalog);
        let credential_models: Arc<dyn baton_core::ports::credentials::CredentialModelCatalog> =
            Arc::new(RigCredentialModelCatalog {
                auth_root: auth_root.clone(),
            });
        let credential_tests = Arc::new(LocalConnectionTester {
            copilot: copilot_auth.clone(),
            auth_root: auth_root.clone(),
        });
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
            credential_tests,
            task_resources,
            run_resources,
            skills,
            skill_downloader,
        ));
        Self {
            project_dir: Arc::new(project_dir),
            mediator,
            chatgpt_auth,
            copilot_auth,
            credential_operations: Arc::new(tokio::sync::Mutex::new(())),
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

/// Holds an OS-level lock until dropped. The lock file must never be removed while in use.
pub struct ProjectProcessLock {
    _project: std::fs::File,
    _identity: std::fs::File,
}

pub fn lock_project_process(project_dir: &std::path::Path) -> anyhow::Result<ProjectProcessLock> {
    let project = project_dir.canonicalize()?;
    if !project.is_dir() {
        anyhow::bail!("Project path is not a directory: {}", project.display());
    }
    let lock_directory = project.join(".baton").join("runs");
    std::fs::create_dir_all(&lock_directory)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(lock_directory.join("process.lock"))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => anyhow::bail!(
            "Another Baton process is already using project {}",
            project.display()
        ),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    let id =
        baton_adapters::repositories::filesystem_project_config::ensure_project_identity(&project)
            .map_err(anyhow::Error::msg)?;
    let base = match std::env::var_os("BATON_CREDENTIAL_DIR") {
        Some(base) => PathBuf::from(base),
        None => PathBuf::from(
            std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?,
        )
        .join(".local/share/baton/credentials"),
    };
    let locks = base.join("project-locks");
    std::fs::create_dir_all(&locks)?;
    let identity = options.open(locks.join(format!("{id}.lock")))?;
    match identity.try_lock() {
        Ok(()) => Ok(ProjectProcessLock {
            _project: file,
            _identity: identity,
        }),
        Err(std::fs::TryLockError::WouldBlock) => {
            anyhow::bail!("Another Baton process is already using this project identity")
        }
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

pub async fn serve(project_dir: PathBuf, host: &str, port: u16) -> anyhow::Result<()> {
    let project_lock = lock_project_process(&project_dir)?;
    serve_with_project_lock(project_dir, host, port, project_lock).await
}

/// Serve a project for which the caller already holds the process lock.
pub async fn serve_with_project_lock(
    project_dir: PathBuf,
    host: &str,
    port: u16,
    _project_lock: ProjectProcessLock,
) -> anyhow::Result<()> {
    let listener = bind_available_port(host, port).await?;
    println!("Web interface: http://{}", listener.local_addr()?);
    axum::serve(listener, router(AppState::new(project_dir)))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn bind_available_port(host: &str, port: u16) -> std::io::Result<tokio::net::TcpListener> {
    let mut port = port;
    loop {
        match tokio::net::TcpListener::bind((host, port)).await {
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse && port < u16::MAX => {
                port += 1;
            }
            result => return result,
        }
    }
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
    async fn binding_skips_an_occupied_port() {
        let occupied = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = occupied.local_addr().unwrap().port();
        if port == u16::MAX {
            return;
        }
        let listener = bind_available_port("127.0.0.1", port).await.unwrap();
        assert!(listener.local_addr().unwrap().port() > port);
    }

    #[test]
    fn project_process_lock_blocks_same_project_and_releases_on_drop() {
        let project = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let first = lock_project_process(project.path()).unwrap();
        assert!(lock_project_process(project.path().join(".").as_path()).is_err());
        let _other = lock_project_process(other.path()).unwrap();
        drop(first);
        assert!(lock_project_process(project.path()).is_ok());
    }

    #[test]
    fn cloned_project_identity_shares_process_lock() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let lock = lock_project_process(first.path()).unwrap();
        std::fs::create_dir_all(second.path().join(".baton")).unwrap();
        std::fs::copy(
            first.path().join(".baton/config.json"),
            second.path().join(".baton/config.json"),
        )
        .unwrap();
        assert!(lock_project_process(second.path()).is_err());
        drop(lock);
        assert!(lock_project_process(second.path()).is_ok());
    }

    #[test]
    fn project_lock_is_released_after_process_crash() {
        const CHILD_PROJECT: &str = "BATON_TEST_LOCK_CHILD_PROJECT";
        if let Some(project) = std::env::var_os(CHILD_PROJECT) {
            let project = std::path::PathBuf::from(project);
            let _lock = lock_project_process(&project).unwrap();
            std::fs::write(project.join("lock-ready"), b"ready").unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }

        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let project = tempfile::tempdir().unwrap();
        let mut child = ChildGuard(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "web_tests::project_lock_is_released_after_process_crash",
                ])
                .env(CHILD_PROJECT, project.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !project.path().join("lock-ready").is_file() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before acquiring lock"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "child did not acquire lock in time"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(lock_project_process(project.path()).is_err());
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert!(project.path().join(".baton/runs/process.lock").is_file());
        assert!(lock_project_process(project.path()).is_ok());
    }

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
