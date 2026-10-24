//! Local provider metadata. Credentials are deliberately not part of this domain DTO.
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub default_model: String,
    pub base_url: Option<String>,
    pub models: Vec<String>,
    pub status: String,
}
#[async_trait]
pub trait ProviderModelCatalog: Send + Sync {
    async fn available_models(&self) -> Result<Vec<String>, ()>;
}

/// Refreshes provider availability from an infrastructure catalog without exposing credentials to
/// project configuration or API consumers.
pub async fn refresh_available_models(provider: &mut Provider, catalog: &dyn ProviderModelCatalog) {
    match catalog.available_models().await {
        Ok(models) if !models.is_empty() => {
            provider.status = "available".into();
            provider.models = models;
        }
        Ok(_) => mark_models_unavailable(provider, "No models are available for this session"),
        Err(()) => mark_models_unavailable(
            provider,
            "Could not load provider models; authenticate again",
        ),
    }
}

pub fn mark_models_unavailable(provider: &mut Provider, message: &str) {
    provider.status = message.into();
    provider.models.clear();
}

impl Provider {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty() || self.name.len() > 100 {
            return Err("Enter a provider name (1–100 characters)");
        }
        if !["openai", "chatgpt"].contains(&self.kind.as_str()) {
            return Err("Choose OpenAI API or ChatGPT");
        }
        if self.default_model.trim().is_empty() || self.default_model.len() > 100 {
            return Err("Enter a default model");
        }
        if self.kind == "chatgpt" && self.base_url.is_some() {
            return Err("ChatGPT does not support a custom API base URL");
        }
        Ok(())
    }
    pub fn available(&self, model: &str) -> bool {
        self.status == "available" && self.models.iter().any(|m| m == model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Catalog(Result<Vec<String>, ()>);

    #[async_trait]
    impl ProviderModelCatalog for Catalog {
        async fn available_models(&self) -> Result<Vec<String>, ()> {
            self.0.clone()
        }
    }

    fn provider() -> Provider {
        Provider {
            id: "local".into(),
            name: "Local".into(),
            kind: "openai".into(),
            default_model: "m".into(),
            base_url: None,
            models: vec![],
            status: "disconnected".into(),
        }
    }

    #[tokio::test]
    async fn refreshes_only_catalog_models_and_marks_failures_unavailable() {
        let mut p = provider();
        refresh_available_models(&mut p, &Catalog(Ok(vec!["one".into(), "two".into()]))).await;
        assert_eq!(p.status, "available");
        assert_eq!(p.models, ["one", "two"]);

        refresh_available_models(&mut p, &Catalog(Err(()))).await;
        assert_ne!(p.status, "available");
        assert!(p.models.is_empty());
    }

    #[test]
    fn availability_requires_verification_and_model() {
        let mut p = Provider {
            id: "local".into(),
            name: "Local".into(),
            kind: "openai".into(),
            default_model: "m".into(),
            base_url: None,
            models: vec!["m".into()],
            status: "disconnected".into(),
        };
        assert!(p.validate().is_ok());
        assert!(!p.available("m"));
        p.status = "available".into();
        assert!(p.available("m"));
        assert!(!p.available("missing"));
        assert!(!serde_json::to_string(&p).unwrap().contains("token"));
    }
}
