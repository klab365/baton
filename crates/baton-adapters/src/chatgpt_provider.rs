use async_trait::async_trait;
use baton_core::providers::{ProviderCatalogError, ProviderModelCatalog};
use rig_core::providers::chatgpt::auth::{
    AuthContext, AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore,
};
use std::{path::PathBuf, time::Duration};

const MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models";
// The Codex catalog requires a current Codex client version to return subscription models.
const CLIENT_VERSION: &str = "0.163.0";

#[derive(Debug, Clone)]
pub struct DeviceCode {
    pub verification_uri: String,
    pub user_code: String,
}

/// A catalog bound to the context resolved by a successful sign-in.
pub struct AuthenticatedChatGptCatalog {
    context: AuthContext,
}

/// Local ChatGPT OAuth session and model-catalog adapter.
pub struct ChatGptSession {
    token_dir: PathBuf,
}

impl ChatGptSession {
    pub fn new(token_dir: PathBuf) -> Self {
        Self { token_dir }
    }

    pub async fn authenticate(
        &self,
        on_device_code: impl Fn(DeviceCode) + Send + Sync + 'static,
    ) -> Result<AuthenticatedChatGptCatalog, ProviderCatalogError> {
        let auth = Authenticator::new(
            AuthSource::OAuth,
            FileTokenStore::new(&self.token_dir),
            DeviceCodeHandler::new(move |prompt| {
                on_device_code(DeviceCode {
                    verification_uri: prompt.verification_uri,
                    user_code: prompt.user_code,
                });
            }),
            true,
        );
        let context = tokio::time::timeout(
            Duration::from_secs(900),
            auth.auth_context(&rig_reqwest::shared()),
        )
        .await
        .map_err(|_| ProviderCatalogError::Authentication)?
        .map_err(|_| ProviderCatalogError::Authentication)?;
        Ok(AuthenticatedChatGptCatalog { context })
    }
}

#[async_trait]
impl ProviderModelCatalog for AuthenticatedChatGptCatalog {
    async fn available_models(&self) -> Result<Vec<String>, ProviderCatalogError> {
        let context = &self.context;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ProviderCatalogError::CatalogUnavailable)?;
        let mut url = reqwest::Url::parse(MODELS_URL)
            .map_err(|_| ProviderCatalogError::CatalogUnavailable)?;
        url.query_pairs_mut()
            .append_pair("client_version", CLIENT_VERSION);
        let mut request = client
            .get(url)
            .bearer_auth(context.api_key.expose())
            .header("originator", "rig")
            .header("session-id", uuid::Uuid::new_v4().to_string());
        if let Some(account_id) = &context.account_id {
            request = request.header("ChatGPT-Account-Id", account_id);
        }
        let payload = request
            .send()
            .await
            .map_err(|_| ProviderCatalogError::CatalogUnavailable)?
            .error_for_status()
            .map_err(|_| ProviderCatalogError::CatalogUnavailable)?
            .json::<serde_json::Value>()
            .await
            .map_err(|_| ProviderCatalogError::CatalogUnavailable)?;
        models_from_catalog(&payload).map_err(|_| ProviderCatalogError::CatalogUnavailable)
    }
}

fn models_from_catalog(payload: &serde_json::Value) -> Result<Vec<String>, ()> {
    let mut models = payload["models"]
        .as_array()
        .ok_or(())?
        .iter()
        .filter(|model| {
            model["supported_in_api"] == true && model["visibility"].as_str() == Some("list")
        })
        .filter_map(|model| model["slug"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_visible_api_models() {
        let payload = serde_json::json!({"models": [
            {"slug": "visible", "visibility": "list", "supported_in_api": true},
            {"slug": "hidden", "visibility": "hide", "supported_in_api": true},
            {"slug": "unsupported", "visibility": "list", "supported_in_api": false},
            {"slug": "visible", "visibility": "list", "supported_in_api": true}
        ]});
        assert_eq!(models_from_catalog(&payload).unwrap(), ["visible"]);
    }
}
