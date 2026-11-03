//! Provider-specific implementations of credential connection checks.
use async_trait::async_trait;
use baton_core::{domain::credentials::Credential, ports::credentials::CredentialConnectionTester};

use super::copilot_auth::CopilotAuthManager;

pub struct LocalConnectionTester {
    pub copilot: CopilotAuthManager,
    pub auth_root: std::path::PathBuf,
}

async fn check_authenticated_endpoint(
    url: &str,
    token: &str,
    account_id: Option<&str>,
    provider: &str,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| format!("Could not initialize {provider} connection test"))?;
    let mut request = client.get(url).bearer_auth(token);
    if let Some(account_id) = account_id {
        request = request.header("chatgpt-account-id", account_id);
    }
    let response = request
        .send()
        .await
        .map_err(|_| format!("Could not reach {provider}. Check your network connection."))?;
    match response.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err(format!(
            "{provider} rejected the credential. Check access and reconnect in Settings."
        )),
        _ => Err(format!(
            "{provider} connection test failed. Try again later."
        )),
    }
}

#[async_trait]
impl CredentialConnectionTester for LocalConnectionTester {
    async fn test_connection(
        &self,
        credential: &Credential,
        secret: Option<&str>,
    ) -> Result<(), String> {
        match credential.credential_type.0.as_str() {
            "github_copilot_session" => self.copilot.test_connection(&credential.id).await,
            "openai_api_key" => {
                let secret = secret
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("OpenAI credential has no stored secret")?;
                check_authenticated_endpoint(
                    "https://api.openai.com/v1/models",
                    secret,
                    None,
                    "OpenAI",
                )
                .await
            }
            "chatgpt_session" => {
                use rig::providers::chatgpt::auth::{
                    AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore,
                };
                credential.validate().map_err(|_| "Invalid credential ID")?;
                let directory = self.auth_root.join("auth").join(&credential.id);
                if !directory.join(".session").is_file()
                    || !directory.join("chatgpt.json").is_file()
                {
                    return Err("ChatGPT session is missing. Reconnect in Settings.".into());
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
                .map_err(|_| "ChatGPT authentication timed out. Check your network connection.")?
                .map_err(|_| "ChatGPT authentication failed. Reconnect in Settings.")?;
                check_authenticated_endpoint(
                    "https://chatgpt.com/backend-api/codex/models?client_version=0.1.0",
                    context.api_key.expose(),
                    context.account_id.as_deref(),
                    "ChatGPT",
                )
                .await
            }
            "ollama_api_key" => {
                let base = credential
                    .configuration
                    .get("baseUrl")
                    .map(String::as_str)
                    .unwrap_or("http://localhost:11434");
                let mut url = reqwest::Url::parse(base).map_err(|_| "Invalid Ollama server URL")?;
                if !matches!(url.scheme(), "http" | "https")
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err("Invalid Ollama server URL".into());
                }
                let path = format!("{}/api/tags", url.path().trim_end_matches('/'));
                url.set_path(&path);
                url.set_query(None);
                url.set_fragment(None);
                let client = reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(std::time::Duration::from_secs(15))
                    .build()
                    .map_err(|_| "Could not initialize Ollama connection test")?;
                let mut request = client.get(url);
                if let Some(secret) = secret {
                    request = request.bearer_auth(secret);
                }
                let response = request.send().await.map_err(|_| "Could not reach the configured Ollama server. Check its address and network access.")?;
                match response.status().as_u16() {
                    200..=299 => Ok(()),
                    401 | 403 => Err("Ollama rejected the credential. Check the API key and server configuration.".into()),
                    _ => Err("Ollama connection test failed. Check the server configuration.".into()),
                }
            }
            _ => Err("Connection testing is not supported for this credential type".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baton_core::domain::credentials::{CredentialStatus, CredentialType};
    use std::io::{Read, Write};

    #[tokio::test]
    async fn authenticated_check_uses_token_and_account_without_leaking_errors() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let n = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..n]).to_lowercase();
            assert!(request.contains("authorization: bearer session-token"));
            assert!(request.contains("chatgpt-account-id: selected-account"));
            stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 13\r\nConnection: close\r\n\r\nsession-token").unwrap();
        });
        let error = check_authenticated_endpoint(
            &format!("http://{address}/models"),
            "session-token",
            Some("selected-account"),
            "ChatGPT",
        )
        .await
        .unwrap_err();
        assert!(error.contains("rejected"));
        assert!(!error.contains("session-token"));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn missing_chatgpt_session_and_openai_secret_fail_without_network() {
        let root = tempfile::tempdir().unwrap();
        let tester = LocalConnectionTester {
            copilot: CopilotAuthManager::new(root.path().into()),
            auth_root: root.path().into(),
        };
        let mut credential = Credential {
            id: "test".into(),
            name: "test".into(),
            credential_type: CredentialType::new("chatgpt_session").unwrap(),
            configuration: Default::default(),
            created_at: 0,
            updated_at: 0,
            status: CredentialStatus::Available,
        };
        assert!(
            tester
                .test_connection(&credential, None)
                .await
                .unwrap_err()
                .contains("missing")
        );
        credential.credential_type = CredentialType::new("openai_api_key").unwrap();
        assert!(
            tester
                .test_connection(&credential, None)
                .await
                .unwrap_err()
                .contains("secret")
        );
    }

    #[tokio::test]
    async fn ollama_test_uses_selected_server_and_secret_and_sanitizes_errors() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let n = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..n]).to_lowercase();
            assert!(request.starts_with("get /api/tags "));
            assert!(request.contains("authorization: bearer selected-secret"));
            stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 15\r\nConnection: close\r\n\r\nselected-secret").unwrap();
        });
        let credential = Credential {
            id: "test".into(),
            name: "Test".into(),
            credential_type: CredentialType::new("ollama_api_key").unwrap(),
            configuration: [("baseUrl".into(), format!("http://{address}"))].into(),
            created_at: 0,
            updated_at: 0,
            status: CredentialStatus::Available,
        };
        let root = tempfile::tempdir().unwrap();
        let tester = LocalConnectionTester {
            copilot: CopilotAuthManager::new(root.path().into()),
            auth_root: root.path().into(),
        };
        let error = tester
            .test_connection(&credential, Some("selected-secret"))
            .await
            .unwrap_err();
        assert!(error.contains("rejected"));
        assert!(!error.contains("selected-secret"));
        server.join().unwrap();
    }
}
