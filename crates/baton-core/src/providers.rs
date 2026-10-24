//! Local provider metadata. Credentials are deliberately not part of this domain DTO.
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
