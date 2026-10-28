//! Local credential vault adapter. Metadata and secret values use separate files.
pub mod chatgpt_auth;
pub mod model_catalog;
use async_trait::async_trait;
use baton_core::{
    domain::credentials::{Credential, CredentialStatus, CredentialType},
    ports::credentials::{
        CredentialAuthentication, CredentialConfigurationField, CredentialRepository,
        CredentialTypeCatalog, CredentialTypeDescriptor,
    },
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Credential types currently understood by the local adapter integrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalCredentialType {
    OllamaApiKey,
    OpenaiApiKey,
    ChatgptSession,
    GithubPat,
    GithubCopilotSession,
    AzureDevopsPat,
    SonarToken,
}

impl LocalCredentialType {
    pub fn as_id(self) -> &'static str {
        match self {
            Self::OllamaApiKey => "ollama_api_key",
            Self::OpenaiApiKey => "openai_api_key",
            Self::ChatgptSession => "chatgpt_session",
            Self::GithubPat => "github_pat",
            Self::GithubCopilotSession => "github_copilot_session",
            Self::AzureDevopsPat => "azure_devops_pat",
            Self::SonarToken => "sonar_token",
        }
    }
    pub fn parse(value: &CredentialType) -> Option<Self> {
        Some(match value.0.as_str() {
            "ollama_api_key" => Self::OllamaApiKey,
            "openai_api_key" => Self::OpenaiApiKey,
            "chatgpt_session" => Self::ChatgptSession,
            "github_pat" => Self::GithubPat,
            "github_copilot_session" => Self::GithubCopilotSession,
            "azure_devops_pat" => Self::AzureDevopsPat,
            "sonar_token" => Self::SonarToken,
            _ => return None,
        })
    }
}

pub struct LocalCredentialTypeCatalog;

impl CredentialTypeCatalog for LocalCredentialTypeCatalog {
    fn list(&self) -> Vec<CredentialTypeDescriptor> {
        [
            LocalCredentialType::OllamaApiKey,
            LocalCredentialType::OpenaiApiKey,
            LocalCredentialType::ChatgptSession,
            LocalCredentialType::GithubPat,
            LocalCredentialType::GithubCopilotSession,
            LocalCredentialType::AzureDevopsPat,
            LocalCredentialType::SonarToken,
        ]
        .into_iter()
        .map(|kind| CredentialTypeDescriptor {
            id: CredentialType(kind.as_id().into()),
            provider: match kind {
                LocalCredentialType::OllamaApiKey => Some("ollama".into()),
                LocalCredentialType::OpenaiApiKey => Some("openai".into()),
                LocalCredentialType::ChatgptSession => Some("chatgpt".into()),
                LocalCredentialType::GithubCopilotSession => Some("github-copilot".into()),
                LocalCredentialType::GithubPat
                | LocalCredentialType::AzureDevopsPat
                | LocalCredentialType::SonarToken => None,
            },
            label: match kind {
                LocalCredentialType::OllamaApiKey => "Ollama API key",
                LocalCredentialType::OpenaiApiKey => "OpenAI API key",
                LocalCredentialType::ChatgptSession => "ChatGPT session",
                LocalCredentialType::GithubPat => "GitHub personal access token",
                LocalCredentialType::GithubCopilotSession => "GitHub Copilot session",
                LocalCredentialType::AzureDevopsPat => "Azure DevOps personal access token",
                LocalCredentialType::SonarToken => "Sonar token",
            }
            .into(),
            supports_test: matches!(kind, LocalCredentialType::OpenaiApiKey),
            authentication: match kind {
                LocalCredentialType::ChatgptSession | LocalCredentialType::GithubCopilotSession => {
                    CredentialAuthentication::DeviceCode
                }
                _ => CredentialAuthentication::SecretInput,
            },
            configuration_fields: if kind == LocalCredentialType::OllamaApiKey {
                vec![CredentialConfigurationField {
                    id: "baseUrl".into(),
                    label: "Ollama base URL".into(),
                    required: false,
                    secret: false,
                }]
            } else {
                Vec::new()
            },
        })
        .collect()
    }

    fn get(&self, id: &CredentialType) -> Option<CredentialTypeDescriptor> {
        self.list()
            .into_iter()
            .find(|descriptor| descriptor.id == *id)
    }
}

pub struct FilesystemCredentialRepository {
    root: PathBuf,
}

impl FilesystemCredentialRepository {
    pub fn for_local_user() -> Result<Self, String> {
        if let Some(root) = std::env::var_os("BATON_CREDENTIAL_DIR") {
            return Ok(Self {
                root: PathBuf::from(root),
            });
        }
        let home = std::env::var_os("HOME")
            .ok_or("Local credential storage is unavailable; HOME is not set")?;
        Ok(Self {
            root: PathBuf::from(home).join(".local/share/baton/credentials"),
        })
    }

    #[cfg(test)]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn token_store_dir(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        {
            return Err("Invalid credential ID".into());
        }
        let path = self.root.join("auth").join(id);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(&path).map_err(
                |_| "Local credential storage is unavailable; check directory permissions",
            )?;
        }
        #[cfg(not(unix))]
        fs::create_dir_all(&path)
            .map_err(|_| "Local credential storage is unavailable; check directory permissions")?;
        Ok(path)
    }

    pub fn root_path(&self) -> &Path {
        &self.root
    }

    fn metadata_path(&self) -> PathBuf {
        self.root.join("metadata.json")
    }
    fn secret_path(&self, id: &str) -> PathBuf {
        self.root.join("secrets").join(id)
    }
    fn session_marker_path(&self, id: &str) -> PathBuf {
        self.root.join("auth").join(id).join(".session")
    }

    fn ensure_root(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(self.root.join("secrets")).map_err(|_| {
                "Local credential storage is unavailable; check directory permissions".to_string()
            })?;
        }
        #[cfg(not(unix))]
        fs::create_dir_all(self.root.join("secrets")).map_err(|_| {
            "Local credential storage is unavailable; check directory permissions".to_string()
        })?;
        Ok(())
    }

    fn read_metadata(&self) -> Result<Vec<Credential>, String> {
        match fs::read_to_string(self.metadata_path()) {
            Ok(contents) => serde_json::from_str(&contents)
                .map_err(|_| "Local credential metadata is invalid".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(_) => {
                Err("Local credential storage is unavailable; check directory permissions".into())
            }
        }
    }

    fn write_metadata(&self, credentials: &[Credential]) -> Result<(), String> {
        self.ensure_root()?;
        let contents = serde_json::to_vec_pretty(credentials)
            .map_err(|_| "Could not serialize credential metadata")?;
        write_private(&self.metadata_path(), &contents)
    }
}

#[async_trait]
impl CredentialRepository for FilesystemCredentialRepository {
    async fn list(&self) -> Result<Vec<Credential>, String> {
        let mut credentials = self.read_metadata()?;
        for credential in &mut credentials {
            let present = self.secret_path(&credential.id).is_file()
                || self.session_marker_path(&credential.id).is_file();
            credential.status = if present {
                CredentialStatus::Available
            } else {
                CredentialStatus::MissingSecret
            };
        }
        Ok(credentials)
    }

    async fn get(&self, id: &str) -> Result<Option<Credential>, String> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .find(|credential| credential.id == id))
    }

    async fn save(&self, credential: &Credential, secret: Option<&str>) -> Result<(), String> {
        credential.validate()?;
        if credential.configuration.keys().any(|key| key != "baseUrl")
            || (credential.credential_type.0 != "ollama_api_key"
                && !credential.configuration.is_empty())
        {
            return Err("Unsupported credential configuration".into());
        }
        if let Some(base_url) = credential.configuration.get("baseUrl")
            && (!(base_url.starts_with("http://") || base_url.starts_with("https://"))
                || base_url.chars().any(char::is_whitespace)
                || base_url.ends_with("://"))
        {
            return Err("Ollama base URL must be a valid HTTP(S) URL".into());
        }
        if let Some(secret) = secret {
            if secret.trim().is_empty() {
                return Err("Credential secret must not be empty".into());
            }
            self.ensure_root()?;
            write_private(&self.secret_path(&credential.id), secret.as_bytes())?;
        }
        let mut credentials = self.read_metadata()?;
        credentials.retain(|item| item.id != credential.id);
        credentials.push(credential.clone());
        self.write_metadata(&credentials)
    }

    fn secret_sync(&self, id: &str) -> Result<Option<String>, String> {
        match fs::read_to_string(self.secret_path(id)) {
            Ok(secret) => Ok(Some(secret)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => {
                Err("Local credential storage is unavailable; check directory permissions".into())
            }
        }
    }

    fn get_sync(&self, id: &str) -> Result<Option<Credential>, String> {
        let mut credentials = self.read_metadata()?;
        let Some(credential) = credentials
            .iter_mut()
            .find(|credential| credential.id == id)
        else {
            return Ok(None);
        };
        let present = self.secret_path(id).is_file() || self.session_marker_path(id).is_file();
        credential.status = if present {
            CredentialStatus::Available
        } else {
            CredentialStatus::MissingSecret
        };
        Ok(Some(credential.clone()))
    }

    async fn secret(&self, id: &str) -> Result<Option<String>, String> {
        match fs::read_to_string(self.secret_path(id)) {
            Ok(secret) => Ok(Some(secret)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => {
                Err("Local credential storage is unavailable; check directory permissions".into())
            }
        }
    }

    async fn delete(&self, id: &str) -> Result<(), String> {
        let mut credentials = self.read_metadata()?;
        let before = credentials.len();
        credentials.retain(|item| item.id != id);
        if before == credentials.len() {
            return Ok(());
        }
        self.write_metadata(&credentials)?;
        match fs::remove_file(self.secret_path(id)) {
            Ok(()) | Err(std::io::Error { .. }) => {}
        }
        let _ = fs::remove_dir_all(self.root.join("auth").join(id));
        Ok(())
    }
}

fn write_private(path: &Path, contents: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|_| "Local credential storage is unavailable; check directory permissions")?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|_| "Local credential storage is unavailable; check directory permissions")?;
    fs::rename(temporary, path)
        .map_err(|_| "Local credential storage is unavailable; check directory permissions".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use baton_core::ports::credentials::CredentialTypeCatalog;

    #[test]
    fn catalog_exposes_ollama_base_url_as_non_secret_configuration() {
        let catalog = LocalCredentialTypeCatalog;
        let kind = CredentialType::new("ollama_api_key").unwrap();
        let descriptor = catalog.get(&kind).unwrap();
        assert_eq!(descriptor.configuration_fields.len(), 1);
        let field = &descriptor.configuration_fields[0];
        assert_eq!(field.id, "baseUrl");
        assert!(!field.secret);
    }
}
