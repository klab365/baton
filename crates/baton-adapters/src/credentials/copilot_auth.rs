//! Per-credential GitHub Copilot device authentication.
use rig::providers::copilot::auth::{AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopilotAuthStatus {
    pub status: String,
    pub verification_uri: Option<String>,
    pub user_code: Option<String>,
    pub message: Option<String>,
}

struct Flow {
    state: Arc<Mutex<CopilotAuthStatus>>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct CopilotAuthManager {
    root: PathBuf,
    flows: Arc<Mutex<HashMap<String, Flow>>>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

fn models_url(base: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base).map_err(|_| "Invalid Copilot service endpoint")?;
    let host = url.host_str().unwrap_or("");
    if url.scheme() != "https"
        || !(host == "githubcopilot.com" || host.ends_with(".githubcopilot.com"))
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Invalid Copilot service endpoint".into());
    }
    url.set_path("/models");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

impl CopilotAuthManager {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            flows: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start(&self, id: String) -> Result<CopilotAuthStatus, String> {
        if !valid_id(&id) {
            return Err("Invalid credential ID".into());
        }
        let mut flows = self
            .flows
            .lock()
            .map_err(|_| "Authentication state is unavailable")?;
        if flows.get(&id).is_some_and(|flow| !flow.task.is_finished()) {
            return Err("Authentication is already in progress".into());
        }
        let directory = self.root.join("auth").join(&id);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(&directory)
                .map_err(|_| "Local credential storage is unavailable")?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&directory)
            .map_err(|_| "Local credential storage is unavailable")?;
        let state = Arc::new(Mutex::new(CopilotAuthStatus {
            status: "pending".into(),
            verification_uri: None,
            user_code: None,
            message: None,
        }));
        let callback_state = state.clone();
        let handler = DeviceCodeHandler::new(move |prompt| {
            if let Ok(mut current) = callback_state.lock() {
                current.verification_uri = Some(prompt.verification_uri);
                current.user_code = Some(prompt.user_code);
            }
        });
        let task_state = state.clone();
        let task = tokio::spawn(async move {
            let authenticator = Authenticator::new(
                AuthSource::OAuth,
                FileTokenStore::new(&directory),
                handler,
                true,
            );
            let result = authenticator
                .auth_context(&rig::http_client::ReqwestClient::default())
                .await;
            if let Ok(mut current) = task_state.lock() {
                current.user_code = None;
                current.verification_uri = None;
                if result.is_ok() && std::fs::write(directory.join(".session"), b"").is_ok() {
                    current.status = "available".into();
                } else {
                    current.status = "failed".into();
                    current.message = Some("GitHub Copilot authentication failed or timed out. Check your Copilot access and try again.".into());
                }
            }
        });
        let status = state
            .lock()
            .map_err(|_| "Authentication state is unavailable")?
            .clone();
        flows.insert(id, Flow { state, task });
        Ok(status)
    }

    /// Check the selected session against Copilot without exposing provider errors or tokens.
    pub async fn test_connection(&self, id: &str) -> Result<(), String> {
        if !valid_id(id) {
            return Err("Invalid credential ID".into());
        }
        let directory = self.root.join("auth").join(id);
        if !directory.join(".session").is_file() || !directory.join("copilot.json").is_file() {
            return Err(
                "Copilot session is missing. Delete this credential, connect the account again in Settings, and reassign agents.".into(),
            );
        }
        let authenticator = Authenticator::new(
            AuthSource::OAuth,
            FileTokenStore::new(directory),
            DeviceCodeHandler::default(),
            false,
        );
        let context = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            authenticator.auth_context(&rig::http_client::ReqwestClient::default()),
        )
        .await
        .map_err(|_| "Copilot authentication timed out. Check your network connection.")?
        .map_err(|_| "Copilot authentication failed. Check your access; if the session was revoked, delete this credential, connect the account again, and reassign agents.".to_string())?;
        let base = context
            .api_base
            .as_deref()
            .unwrap_or("https://api.githubcopilot.com");
        let url = models_url(base)?;
        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|_| "Could not initialize the Copilot connection test")?
            .get(url)
            .bearer_auth(context.api_key.expose())
            .header("editor-version", "vscode/1.96.2")
            .header("editor-plugin-version", "copilot-chat/0.26.7")
            .header("user-agent", "GitHubCopilotChat")
            .send()
            .await
            .map_err(|_| "Could not reach GitHub Copilot. Check your network connection.")?;
        match response.status().as_u16() {
            200..=299 => Ok(()),
            401 | 403 => Err("GitHub Copilot rejected the session or access. Check your subscription; delete this credential, connect the account again, and reassign agents.".into()),
            _ => Err("GitHub Copilot connection test failed. Try again later.".into()),
        }
    }

    pub fn status(&self, id: &str) -> Option<CopilotAuthStatus> {
        let flows = self.flows.lock().ok()?;
        flows.get(id)?.state.lock().ok().map(|state| state.clone())
    }

    pub async fn cancel(&self, id: &str) {
        if !valid_id(id) {
            return;
        }
        let flow = self
            .flows
            .lock()
            .ok()
            .and_then(|mut flows| flows.remove(id));
        if let Some(flow) = flow {
            flow.task.abort();
            let _ = flow.task.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_test_restricts_token_destination() {
        assert_eq!(
            models_url("https://api.githubcopilot.com/other?token=discard#fragment")
                .unwrap()
                .as_str(),
            "https://api.githubcopilot.com/models"
        );
        for base in [
            "http://api.githubcopilot.com",
            "https://githubcopilot.com.attacker.example",
            "https://notgithubcopilot.com",
            "https://user@api.githubcopilot.com",
            "https://api.githubcopilot.com:8443",
            "not a url",
        ] {
            assert!(models_url(base).is_err(), "unexpectedly accepted {base}");
        }
    }

    #[tokio::test]
    async fn cancel_waits_until_pending_flow_stops() {
        struct OnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp.path().into());
        let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let guard = OnDrop(stopped.clone());
        let task = tokio::spawn(async move {
            let _guard = guard;
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        let state = Arc::new(Mutex::new(CopilotAuthStatus {
            status: "pending".into(),
            verification_uri: None,
            user_code: None,
            message: None,
        }));
        manager
            .flows
            .lock()
            .unwrap()
            .insert("test".into(), Flow { state, task });
        started_rx.await.unwrap();
        manager.cancel("test").await;
        assert!(stopped.load(std::sync::atomic::Ordering::SeqCst));
        assert!(manager.status("test").is_none());
    }

    #[tokio::test]
    async fn connection_test_rejects_missing_or_invalid_sessions_without_network() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp.path().into());
        assert!(manager.test_connection("../other").await.is_err());
        assert!(
            manager
                .test_connection("missing")
                .await
                .unwrap_err()
                .contains("missing")
        );
    }

    #[tokio::test]
    async fn rejects_invalid_ids_without_touching_storage() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp.path().into());
        for id in ["", "..", "../other", "a/b"] {
            assert!(manager.start(id.into()).is_err());
            manager.cancel(id).await;
        }
        assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
    }
}
