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
    pub(crate) run_updates: tokio::sync::broadcast::Sender<String>,
}

impl AppState {
    pub fn new(project_dir: PathBuf) -> Self {
        let repository = FilesystemProjectConfigRepository::new(project_dir.clone());
        let runs = FilesystemRunRepository(project_dir.clone());
        // A running run cannot survive a process restart because adapters are in-process.
        // Convert it deterministically to a durable terminal record before serving requests.
        runs.recover_interrupted()
            .unwrap_or_else(|error| panic!("failed to recover interrupted runs: {error}"));
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
        let (run_updates, _) = tokio::sync::broadcast::channel(256);
        let task_repository: Arc<dyn TaskRepository> = Arc::new(NotifyingTaskRepository {
            inner: FilesystemTaskRepository(project_dir.clone()),
            updates: run_updates.clone(),
        });
        let run_repository: Arc<dyn RunRepository> = Arc::new(NotifyingRunRepository {
            inner: FilesystemRunRepository(project_dir.clone()),
            updates: run_updates.clone(),
        });
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
            tasks: task_repository.clone(),
            runs: run_repository.clone(),
            project_config: repository.clone(),
            runtime_factory: runtime_factory.clone(),
            skills: skills.clone(),
        };
        let run_resources = RunFeatureResources {
            runs: run_repository,
            tasks: task_repository,
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
            run_updates,
        }
    }
}

struct NotifyingTaskRepository {
    inner: FilesystemTaskRepository,
    updates: tokio::sync::broadcast::Sender<String>,
}

impl TaskRepository for NotifyingTaskRepository {
    fn acquire_operation_lease(
        &self,
        id: &str,
    ) -> Result<Box<dyn baton_core::ports::tasks::TaskOperationLease>, String> {
        self.inner.acquire_operation_lease(id)
    }
    fn load(&self, id: &str) -> Result<baton_core::domain::tasks::Task, String> {
        self.inner.load(id)
    }
    fn list(&self) -> Result<Vec<baton_core::domain::tasks::Task>, String> {
        self.inner.list()
    }
    fn save(&self, task: &baton_core::domain::tasks::Task) -> Result<(), String> {
        self.inner.save(task)?;
        let _ = self.updates.send(format!("task:{}", task.id));
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        self.inner.delete(id)?;
        let _ = self.updates.send(format!("task:{id}"));
        Ok(())
    }
}

struct NotifyingRunRepository {
    inner: FilesystemRunRepository,
    updates: tokio::sync::broadcast::Sender<String>,
}

impl RunRepository for NotifyingRunRepository {
    fn acquire_action_lease(
        &self,
        id: &str,
    ) -> Result<Box<dyn baton_core::ports::runs::RunActionLease>, String> {
        self.inner.acquire_action_lease(id)
    }
    fn worktree_manager(
        &self,
    ) -> Result<Box<dyn baton_core::ports::worktrees::WorktreeManager>, String> {
        self.inner.worktree_manager()
    }
    fn create(&self, run: &baton_core::domain::pipeline::PipelineRun) -> Result<(), String> {
        self.inner.create(run)?;
        let _ = self.updates.send(run.id.clone());
        Ok(())
    }
    fn save(&self, run: &baton_core::domain::pipeline::PipelineRun) -> Result<(), String> {
        self.inner.save(run)?;
        let _ = self.updates.send(run.id.clone());
        Ok(())
    }
    fn load(&self, id: &str) -> Result<baton_core::domain::pipeline::PipelineRun, String> {
        self.inner.load(id)
    }
    fn list(&self) -> Result<Vec<baton_core::domain::pipeline::PipelineRun>, String> {
        RunRepository::list(&self.inner)
    }
    fn publish(&self, event: &baton_core::domain::pipeline::RunEvent) -> Result<(), String> {
        self.inner.publish(event)
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
    let address = listener.local_addr()?;
    println!("Web interface: http://{address}");
    let app = router(AppState::new(project_dir)).layer(axum::middleware::from_fn(
        move |request, next| request_boundary(address.port(), request, next),
    ));
    tokio::select! {
        result = axum::serve(listener, app).into_future() => result?,
        result = tokio::signal::ctrl_c() => result?,
    }
    Ok(())
}

async fn request_boundary(
    port: u16,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::Method;
    let headers = request.headers();
    let mut hosts = headers.get_all(header::HOST).iter();
    let host = hosts.next().and_then(|value| value.to_str().ok());
    let valid_host = host
        .and_then(|value| value.parse::<axum::http::uri::Authority>().ok())
        .is_some_and(|authority| {
            let name = authority.host();
            let local = name.eq_ignore_ascii_case("localhost")
                || name
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback());
            local && authority.port_u16() == Some(port) && !authority.as_str().contains('@')
        });
    if !valid_host || hosts.next().is_some() {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        let mut origins = headers.get_all(header::ORIGIN).iter();
        if let Some(origin) = origins.next() {
            let valid_origin = origin
                .to_str()
                .ok()
                .and_then(|value| value.strip_prefix("http://"))
                == host;
            if !valid_origin || origins.next().is_some() {
                return StatusCode::FORBIDDEN.into_response();
            }
        } else if headers
            .get("sec-fetch-site")
            .is_some_and(|value| value != "none" && value != "same-origin")
        {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    next.run(request).await
}

async fn bind_available_port(host: &str, port: u16) -> std::io::Result<tokio::net::TcpListener> {
    let addresses: Vec<_> = tokio::net::lookup_host((host, port)).await?.collect();
    if addresses.is_empty() || addresses.iter().any(|address| !address.ip().is_loopback()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Baton only listens on loopback addresses",
        ));
    }
    let mut port = port;
    loop {
        let addresses: Vec<_> = addresses
            .iter()
            .map(|address| std::net::SocketAddr::new(address.ip(), port))
            .collect();
        match tokio::net::TcpListener::bind(addresses.as_slice()).await {
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
    async fn request_boundary_rejects_untrusted_requests_before_mutation() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let count = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .fallback({
                let count = count.clone();
                move || async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            })
            .layer(axum::middleware::from_fn(|request, next| {
                request_boundary(5173, request, next)
            }));
        for (host, origin, expected) in [
            (
                "127.0.0.1:5173",
                Some("http://127.0.0.1:5173"),
                StatusCode::OK,
            ),
            ("localhost:5173", None, StatusCode::OK),
            ("[::1]:5173", None, StatusCode::OK),
            (
                "127.0.0.1:5173",
                Some("http://evil.test"),
                StatusCode::FORBIDDEN,
            ),
            ("127.0.0.1:5173", Some("null"), StatusCode::FORBIDDEN),
            ("evil.test:5173", None, StatusCode::FORBIDDEN),
            ("localhost:9999", None, StatusCode::FORBIDDEN),
            ("localhost", None, StatusCode::FORBIDDEN),
        ] {
            let mut request = Request::post("/api/tasks")
                .header(header::HOST, host)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            assert_eq!(
                app.clone()
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status(),
                expected
            );
        }
        for request in [
            Request::get("/").body(Body::empty()).unwrap(),
            Request::get("/")
                .header(header::HOST, "localhost:5173")
                .header(header::HOST, "evil.test:5173")
                .body(Body::empty())
                .unwrap(),
            Request::post("/")
                .header(header::HOST, "localhost:5173")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::ORIGIN, "http://evil.test")
                .body(Body::empty())
                .unwrap(),
            Request::post("/")
                .header(header::HOST, "localhost:5173")
                .header("sec-fetch-site", "cross-site")
                .body(Body::empty())
                .unwrap(),
        ] {
            assert_eq!(
                app.clone().oneshot(request).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[test]
    #[should_panic(expected = "failed to recover interrupted runs")]
    fn startup_surfaces_run_recovery_errors() {
        let project = tempfile::tempdir().unwrap();
        let runs = project.path().join(".baton/runs");
        std::fs::create_dir_all(&runs).unwrap();
        std::fs::write(runs.join("broken.json"), b"invalid json").unwrap();
        AppState::new(project.path().to_path_buf());
    }

    #[tokio::test]
    async fn rejects_external_bind_addresses() {
        assert!(bind_available_port("0.0.0.0", 0).await.is_err());
        assert!(bind_available_port("::", 0).await.is_err());
        assert!(bind_available_port("127.0.0.1", 0).await.is_ok());
    }

    #[tokio::test]
    async fn board_stream_notifies_task_and_run_changes() {
        use tokio_stream::StreamExt;
        let project = tempfile::tempdir().unwrap();
        let state = AppState::new(project.path().to_path_buf());
        let tasks = NotifyingTaskRepository {
            inner: FilesystemTaskRepository(project.path().to_path_buf()),
            updates: state.run_updates.clone(),
        };
        let runs = NotifyingRunRepository {
            inner: FilesystemRunRepository(project.path().to_path_buf()),
            updates: state.run_updates.clone(),
        };
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/tasks/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        assert!(
            String::from_utf8_lossy(&body.next().await.unwrap().unwrap())
                .contains("data: connected")
        );
        let task: baton_core::domain::tasks::Task = serde_json::from_value(serde_json::json!({
            "id": "task-1", "title": "Task", "description": "Test", "pipelineId": "p",
            "status": "draft", "createdAt": "now", "updatedAt": "now"
        }))
        .unwrap();
        tasks.save(&task).unwrap();
        assert!(
            String::from_utf8_lossy(
                &tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
            )
            .contains("data: changed")
        );
        let run: baton_core::domain::pipeline::PipelineRun =
            serde_json::from_value(serde_json::json!({
                "id": "run-1", "pipelineId": "p", "task": "task", "startedAt": "now",
                "endedAt": null, "durationMs": null, "status": "running", "steps": []
            }))
            .unwrap();
        runs.create(&run).unwrap();
        assert!(
            String::from_utf8_lossy(
                &tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
            )
            .contains("data: changed")
        );
        tasks.delete("task-1").unwrap();
        assert!(
            String::from_utf8_lossy(
                &tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
            )
            .contains("data: changed")
        );
    }

    #[tokio::test]
    async fn run_update_stream_sends_initial_and_persisted_changes() {
        use tokio_stream::StreamExt;
        let project = tempfile::tempdir().unwrap();
        let state = AppState::new(project.path().to_path_buf());
        let repository = NotifyingRunRepository {
            inner: FilesystemRunRepository(project.path().to_path_buf()),
            updates: state.run_updates.clone(),
        };
        let mut run: baton_core::domain::pipeline::PipelineRun =
            serde_json::from_value(serde_json::json!({
                "id": "run-1", "pipelineId": "pipeline", "task": "task", "startedAt": "now",
                "endedAt": null, "durationMs": null, "status": "running", "steps": []
            }))
            .unwrap();
        repository.create(&run).unwrap();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/runs/run-1/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        let initial = body.next().await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&initial).contains("event: update"));
        run.status = baton_core::domain::pipeline::Status::WaitingHuman;
        repository.save(&run).unwrap();
        let update = tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&update).contains("data: changed"));
        assert_eq!(repository.load("run-1").unwrap().status, run.status);
    }

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
