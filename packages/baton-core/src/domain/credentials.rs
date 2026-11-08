//! Local credential metadata. Secret material is deliberately excluded.
use serde::{Deserialize, Serialize};

/// Opaque integration-specific credential identifier. Concrete kinds belong to adapters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct CredentialType(pub String);

impl CredentialType {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty()
            || !value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
        {
            return Err(
                "Credential type must use lowercase ASCII letters, digits, '-' or '_'".into(),
            );
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatus {
    Available,
    MissingSecret,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Credential {
    pub id: String,
    pub name: String,
    pub credential_type: CredentialType,
    /// Non-secret integration configuration, e.g. a custom Ollama base URL.
    #[serde(default)]
    pub configuration: std::collections::BTreeMap<String, String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub status: CredentialStatus,
}

impl Credential {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
        {
            return Err("Credential ID must contain only ASCII letters, digits, '-' or '_'".into());
        }
        if self.name.trim().is_empty() {
            return Err("Credential name must not be empty".into());
        }
        if self.created_at > self.updated_at {
            return Err("Credential creation time must not be after its update time".into());
        }
        Ok(())
    }
}
