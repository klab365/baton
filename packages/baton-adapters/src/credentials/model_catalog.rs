use async_trait::async_trait;
use baton_core::{
    domain::credentials::Credential,
    ports::credentials::{CredentialModelCatalog, ModelDescriptor},
};
use rig::catalog::Catalog;
use serde_json::Value;
use std::{collections::HashSet, path::PathBuf, time::Duration};

#[derive(Clone)]
pub struct RigCredentialModelCatalog {
    pub auth_root: PathBuf,
}

fn allowed_models(provider: &str, body: &Value) -> Result<HashSet<String>, String> {
    let (field, id_field) = match provider {
        "copilot" => ("data", "id"),
        "chatgpt" => ("models", "slug"),
        _ => return Err("Unsupported session provider".into()),
    };
    let rows = body
        .get(field)
        .and_then(Value::as_array)
        .ok_or("Invalid provider model list")?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            if provider == "copilot" {
                if row.get("model_picker_enabled").and_then(Value::as_bool) != Some(true)
                    || row.pointer("/policy/state").and_then(Value::as_str) == Some("disabled")
                    || row
                        .pointer("/capabilities/supports/tool_calls")
                        .and_then(Value::as_bool)
                        == Some(false)
                {
                    return None;
                }
            } else if row
                .get("visibility")
                .and_then(Value::as_str)
                .is_some_and(|visibility| visibility != "list")
            {
                return None;
            }
            row.get(id_field).and_then(Value::as_str).map(str::to_owned)
        })
        .collect())
}

impl RigCredentialModelCatalog {
    async fn session_models(
        &self,
        credential: &Credential,
        provider: &str,
    ) -> Result<HashSet<String>, String> {
        credential.validate().map_err(|_| "Invalid credential ID")?;
        let directory = self.auth_root.join("auth").join(&credential.id);
        if !directory.join(".session").is_file() {
            return Err("Session is missing; reconnect the credential".into());
        }
        let http = rig::http_client::ReqwestClient::default();
        let (context, url) = if provider == "copilot" {
            use rig::providers::copilot::auth::{
                AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore,
            };
            let auth = Authenticator::new(
                AuthSource::OAuth,
                FileTokenStore::new(&directory),
                DeviceCodeHandler::default(),
                false,
            );
            let context = tokio::time::timeout(Duration::from_secs(15), auth.auth_context(&http))
                .await
                .map_err(|_| "Copilot authentication timed out")?
                .map_err(|_| "Copilot authentication failed; reconnect the credential")?;
            let url = super::copilot_auth::models_url(
                context
                    .api_base
                    .as_deref()
                    .unwrap_or("https://api.githubcopilot.com"),
            )?;
            (context, url)
        } else {
            use rig::providers::chatgpt::auth::{
                AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore,
            };
            let auth = Authenticator::new(
                AuthSource::OAuth,
                FileTokenStore::new(&directory),
                DeviceCodeHandler::default(),
                false,
            );
            let context = tokio::time::timeout(Duration::from_secs(15), auth.auth_context(&http))
                .await
                .map_err(|_| "ChatGPT authentication timed out")?
                .map_err(|_| "ChatGPT authentication failed; reconnect the credential")?;
            let url = reqwest::Url::parse(
                "https://chatgpt.com/backend-api/codex/models?client_version=0.166.0",
            )
            .expect("constant URL");
            (context, url)
        };
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "Could not initialize model listing")?;
        let mut request = client.get(url).bearer_auth(context.api_key.expose());
        if provider == "copilot" {
            request = request
                .header("user-agent", "GitHubCopilotChat/0.35.0")
                .header("editor-version", "vscode/1.107.0")
                .header("editor-plugin-version", "copilot-chat/0.35.0")
                .header("copilot-integration-id", "vscode-chat");
        } else if let Some(account) = context.account_id.as_deref() {
            request = request.header("chatgpt-account-id", account);
        }
        let response = request
            .send()
            .await
            .map_err(|_| "Could not reach the provider to list models")?;
        if !response.status().is_success() {
            return Err(
                "Provider rejected the model-list request; check access and reconnect if needed"
                    .into(),
            );
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| "Invalid provider model list")?;
        allowed_models(provider, &body)
    }
}

#[async_trait]
impl CredentialModelCatalog for RigCredentialModelCatalog {
    async fn list_models(
        &self,
        credential: &Credential,
        secret: Option<&str>,
    ) -> Result<Vec<ModelDescriptor>, String> {
        if credential.credential_type.0 == "ollama_api_key" {
            let mut config = rig::providers::ollama::OllamaConfig::new();
            if let Some(base_url) = credential.configuration.get("baseUrl") {
                config = config.with_base_url(base_url);
            }
            if let Some(secret) = secret {
                config = config.with_api_key(secret);
            }
            let models = config.client().list_models().await.map_err(|_| {
                "Could not list models from the configured Ollama server".to_string()
            })?;
            return Ok(models
                .into_iter()
                .map(|model| ModelDescriptor {
                    label: model.name.clone().unwrap_or_else(|| model.id.clone()),
                    id: model.id,
                })
                .collect());
        }
        let provider = match credential.credential_type.0.as_str() {
            "openai_api_key" => "openai",
            "chatgpt_session" => "chatgpt",
            "github_copilot_session" => "copilot",
            _ => return Err("This credential type does not provide models".into()),
        };
        let allowed = if provider == "chatgpt" || provider == "copilot" {
            Some(self.session_models(credential, provider).await?)
        } else {
            None
        };
        let catalog = Catalog::builtin();
        let models = catalog
            .iter()
            .filter(|model| {
                model.provider.vendor() == provider
                    && !model.deprecated
                    && (provider == "openai" || model.tools)
                    && allowed
                        .as_ref()
                        .is_none_or(|allowed| allowed.contains(&model.id))
            })
            .map(|model| ModelDescriptor {
                id: model.id.clone(),
                label: model.display_name.clone(),
            })
            .collect::<Vec<_>>();
        if models.is_empty() {
            return Err("No supported models are available for this credential".into());
        }
        Ok(models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    #[ignore = "requires configured provider sessions"]
    async fn live_session_catalogs() {
        use baton_core::domain::credentials::{CredentialStatus, CredentialType};
        let root = std::env::var_os("BATON_PROBE_ROOT").expect("BATON_PROBE_ROOT");
        let catalog = RigCredentialModelCatalog {
            auth_root: root.into(),
        };
        for (kind, env) in [
            ("chatgpt_session", "BATON_PROBE_CHATGPT"),
            ("github_copilot_session", "BATON_PROBE_COPILOT"),
        ] {
            let credential = Credential {
                id: std::env::var(env).expect(env),
                name: "probe".into(),
                credential_type: CredentialType::new(kind).unwrap(),
                configuration: Default::default(),
                created_at: 0,
                updated_at: 0,
                status: CredentialStatus::Available,
            };
            let models = catalog
                .list_models(&credential, None)
                .await
                .expect("live model list");
            eprintln!(
                "{kind}: {} models; gpt-5.3-instant={}, claude-opus-4.6={}",
                models.len(),
                models.iter().any(|m| m.id == "gpt-5.3-instant"),
                models.iter().any(|m| m.id == "claude-opus-4.6")
            );
        }
    }

    #[test]
    fn filters_provider_models_and_rejects_bad_payloads() {
        let copilot = json!({"data":[{"id":"available","model_picker_enabled":true,"policy":{"state":"enabled"}},{"id":"hidden","model_picker_enabled":false},{"id":"disabled","model_picker_enabled":true,"policy":{"state":"disabled"}},{"id":"no-tools","model_picker_enabled":true,"capabilities":{"supports":{"tool_calls":false}}}]});
        assert_eq!(
            allowed_models("copilot", &copilot).unwrap(),
            HashSet::from(["available".into()])
        );
        let chatgpt = json!({"models":[{"slug":"visible","visibility":"list"},{"slug":"hidden","visibility":"hide"}]});
        assert_eq!(
            allowed_models("chatgpt", &chatgpt).unwrap(),
            HashSet::from(["visible".into()])
        );
        assert!(allowed_models("chatgpt", &json!({})).is_err());
    }
}
