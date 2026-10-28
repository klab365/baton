use async_trait::async_trait;
use baton_core::{
    domain::credentials::Credential,
    ports::credentials::{CredentialModelCatalog, ModelDescriptor},
};
use rig::{catalog::Catalog, providers::registry::ProviderId};

#[derive(Clone, Copy, Default)]
pub struct RigCredentialModelCatalog;

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
            "github_copilot_session" => "github-copilot",
            _ => return Err("This credential type does not provide models".into()),
        };
        let catalog = Catalog::builtin();
        let provider_id = ProviderId::catalog(provider)
            .ok_or_else(|| "No model catalog is available for this credential type".to_string())?;
        let models = catalog
            .iter()
            .filter(|model| model.provider.vendor() == provider && !model.deprecated)
            .map(|model| ModelDescriptor {
                id: model.id.clone(),
                label: model.display_name.clone(),
            })
            .collect::<Vec<_>>();
        if models.is_empty() {
            return Err("No models are listed for this provider".into());
        }
        let _ = provider_id;
        Ok(models)
    }
}
