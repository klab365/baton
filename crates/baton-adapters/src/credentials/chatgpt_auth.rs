use rig::providers::chatgpt::auth::{AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptAuthStatus {
    pub status: String,
    pub verification_uri: Option<String>,
    pub user_code: Option<String>,
    pub message: Option<String>,
}
impl ChatGptAuthStatus {
    fn new(status: &str) -> Self {
        Self {
            status: status.into(),
            verification_uri: None,
            user_code: None,
            message: None,
        }
    }
}

struct Flow {
    state: Arc<Mutex<ChatGptAuthStatus>>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct ChatGptAuthManager {
    root: PathBuf,
    flows: Arc<Mutex<HashMap<String, Flow>>>,
}
impl ChatGptAuthManager {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            flows: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start(&self, id: String) -> Result<ChatGptAuthStatus, String> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        {
            return Err("Invalid credential ID".into());
        }
        let directory = self.root.join("auth").join(&id);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(&directory)
                .map_err(|_| "Local credential storage is unavailable".to_string())?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&directory)
            .map_err(|_| "Local credential storage is unavailable".to_string())?;

        let mut flows = self
            .flows
            .lock()
            .map_err(|_| "Authentication state is unavailable")?;
        if let Some(existing) = flows.get(&id)
            && !existing.task.is_finished()
        {
            return Err("Authentication is already in progress".into());
        }
        if let Some(old) = flows.remove(&id) {
            old.task.abort();
        }
        let state = Arc::new(Mutex::new(ChatGptAuthStatus::new("pending")));
        let callback_state = state.clone();
        let handler = DeviceCodeHandler::new(move |prompt| {
            if let Ok(mut current) = callback_state.lock() {
                current.verification_uri = Some(prompt.verification_uri);
                current.user_code = Some(prompt.user_code);
            }
        });
        let task_state = state.clone();
        let session_marker = directory.join(".session");
        let store = FileTokenStore::new(directory);
        let task = tokio::spawn(async move {
            let authenticator = Authenticator::new(AuthSource::OAuth, store, handler, true);
            let result = authenticator
                .sign_in_with_device_code(&rig::http_client::ReqwestClient::default())
                .await;
            if let Ok(mut current) = task_state.lock() {
                match result {
                    Ok(_) => {
                        let _ = std::fs::write(&session_marker, b"");
                        current.status = "available".into();
                        current.user_code = None;
                        current.message = None;
                    }
                    Err(_) => {
                        current.status = "failed".into();
                        current.user_code = None;
                        current.message =
                            Some("ChatGPT authentication failed or timed out. Try again.".into());
                    }
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

    pub fn status(&self, id: &str) -> Option<ChatGptAuthStatus> {
        let flows = self.flows.lock().ok()?;
        flows.get(id)?.state.lock().ok().map(|state| state.clone())
    }

    pub fn cancel(&self, id: &str) {
        if let Ok(mut flows) = self.flows.lock()
            && let Some(flow) = flows.remove(id)
        {
            flow.task.abort();
        }
        let _ = std::fs::remove_dir_all(self.root.join("auth").join(id));
    }
}
