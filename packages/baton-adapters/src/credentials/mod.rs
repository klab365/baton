//! Local credential vault adapter. Metadata and secret values use separate files.
pub mod chatgpt_auth;
pub mod connection_test;
pub mod copilot_auth;
pub mod model_catalog;
use async_trait::async_trait;
use baton_core::{
    domain::credentials::{Credential, CredentialStatus, CredentialType},
    ports::credentials::{
        CredentialAuthentication, CredentialConfigurationField, CredentialLease,
        CredentialRepository, CredentialTypeCatalog, CredentialTypeDescriptor,
    },
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
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
            LocalCredentialType::GithubCopilotSession,
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
            supports_test: matches!(
                kind,
                LocalCredentialType::OllamaApiKey
                    | LocalCredentialType::OpenaiApiKey
                    | LocalCredentialType::ChatgptSession
                    | LocalCredentialType::GithubCopilotSession
            ),
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
    leases: Arc<Mutex<std::collections::HashMap<String, isize>>>,
}

struct LocalCredentialLease {
    leases: Arc<Mutex<std::collections::HashMap<String, isize>>>,
    ids: Vec<String>,
    exclusive: bool,
}
impl CredentialLease for LocalCredentialLease {}
impl Drop for LocalCredentialLease {
    fn drop(&mut self) {
        let mut leases = self
            .leases
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for id in &self.ids {
            if self.exclusive || leases.get(id) == Some(&1) {
                leases.remove(id);
            } else if let Some(count) = leases.get_mut(id) {
                *count -= 1;
            }
        }
    }
}

impl FilesystemCredentialRepository {
    pub fn for_project(project: &Path) -> Result<Self, String> {
        let base = match std::env::var_os("BATON_CREDENTIAL_DIR") {
            Some(root) => PathBuf::from(root),
            None => {
                let home = std::env::var_os("HOME")
                    .ok_or("Local credential storage is unavailable; HOME is not set")?;
                PathBuf::from(home).join(".local/share/baton/credentials")
            }
        };
        Self::for_project_in(project, &base)
    }

    pub fn for_project_in(project: &Path, base: &Path) -> Result<Self, String> {
        let project = project
            .canonicalize()
            .map_err(|_| "Project directory is unavailable")?;
        if !project.is_dir() {
            return Err("Project path is not a directory".into());
        }
        let config_path = project.join(".baton/project.yml");
        match fs::read_to_string(config_path) {
            Ok(contents) => {
                let config: serde_yaml_ng::Value = serde_yaml_ng::from_str(&contents)
                    .map_err(|_| "Project configuration is invalid")?;
                if let Some(id) = config.get("projectId") {
                    let id = id
                        .as_str()
                        .ok_or("Project credential identity is invalid")?;
                    let id = uuid::Uuid::parse_str(id)
                        .map_err(|_| "Project credential identity is invalid")?;
                    return Ok(Self {
                        root: base.join("projects").join(id.to_string()),
                        leases: Default::default(),
                    });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Project configuration is unavailable".into()),
        }
        let directory = project.join(".baton").join("runs");
        fs::create_dir_all(&directory).map_err(|_| "Project identity storage is unavailable")?;
        let identity = directory.join("credential-project-id");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&identity) {
            Ok(mut file) => {
                file.write_all(uuid::Uuid::now_v7().to_string().as_bytes())
                    .and_then(|()| file.sync_all())
                    .map_err(|_| "Project identity storage is unavailable")?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("Project identity storage is unavailable".into()),
        }
        let id =
            fs::read_to_string(identity).map_err(|_| "Project identity storage is unavailable")?;
        let id =
            uuid::Uuid::parse_str(&id).map_err(|_| "Project credential identity is invalid")?;
        Ok(Self {
            root: base.join("projects").join(id.to_string()),
            leases: Default::default(),
        })
    }

    #[cfg(test)]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            leases: Default::default(),
        }
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

    fn status(&self, credential: &Credential) -> CredentialStatus {
        let Some(descriptor) = LocalCredentialTypeCatalog.get(&credential.credential_type) else {
            return CredentialStatus::Unavailable;
        };
        let present = match descriptor.authentication {
            CredentialAuthentication::DeviceCode => {
                self.session_marker_path(&credential.id).is_file()
                    && (credential.credential_type.0 != "github_copilot_session"
                        || self
                            .root
                            .join("auth")
                            .join(&credential.id)
                            .join("copilot.json")
                            .is_file())
            }
            CredentialAuthentication::SecretInput => self.secret_path(&credential.id).is_file(),
        };
        if present {
            CredentialStatus::Available
        } else {
            CredentialStatus::MissingSecret
        }
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
    fn acquire_run_leases(
        &self,
        ids: &[String],
    ) -> Result<Option<Box<dyn CredentialLease>>, String> {
        let mut ids = ids.to_vec();
        ids.sort();
        ids.dedup();
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "Credential lease state is unavailable")?;
        if ids
            .iter()
            .any(|id| leases.get(id).is_some_and(|count| *count < 0))
        {
            return Err("A credential is being deleted".into());
        }
        for id in &ids {
            *leases.entry(id.clone()).or_default() += 1;
        }
        drop(leases);
        Ok(Some(Box::new(LocalCredentialLease {
            leases: self.leases.clone(),
            ids,
            exclusive: false,
        })))
    }

    fn acquire_delete_lease(&self, id: &str) -> Result<Option<Box<dyn CredentialLease>>, String> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "Credential lease state is unavailable")?;
        if leases.contains_key(id) {
            return Err("Credential is in use by a running operation".into());
        }
        leases.insert(id.into(), -1);
        drop(leases);
        Ok(Some(Box::new(LocalCredentialLease {
            leases: self.leases.clone(),
            ids: vec![id.into()],
            exclusive: true,
        })))
    }

    async fn list(&self) -> Result<Vec<Credential>, String> {
        let mut credentials = self.read_metadata()?;
        for credential in &mut credentials {
            credential.status = self.status(credential);
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
        let _lease = self.acquire_delete_lease(&credential.id)?;
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
        credential.status = self.status(credential);
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
        match fs::remove_file(self.secret_path(id)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(
                    "Could not remove local credential secret; check directory permissions".into(),
                );
            }
        }
        match fs::remove_dir_all(self.root.join("auth").join(id)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(
                    "Could not remove local credential session; check directory permissions".into(),
                );
            }
        }
        self.write_metadata(&credentials)
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
    fn catalog_only_lists_supported_provider_credentials() {
        let ids: Vec<_> = LocalCredentialTypeCatalog
            .list()
            .into_iter()
            .map(|descriptor| descriptor.id.0)
            .collect();
        assert_eq!(
            ids,
            [
                "ollama_api_key",
                "openai_api_key",
                "chatgpt_session",
                "github_copilot_session"
            ]
        );
    }

    #[tokio::test]
    async fn unsupported_legacy_credentials_are_unavailable_even_with_a_secret() {
        let temp = tempfile::tempdir().unwrap();
        let repository = FilesystemCredentialRepository::new(temp.path().into());
        let credential = Credential {
            id: "legacy".into(),
            name: "Legacy".into(),
            credential_type: CredentialType::new("legacy_unsupported").unwrap(),
            configuration: Default::default(),
            created_at: 0,
            updated_at: 0,
            status: CredentialStatus::Available,
        };
        repository
            .save(&credential, Some("old-secret"))
            .await
            .unwrap();
        assert_eq!(
            repository.list().await.unwrap()[0].status,
            CredentialStatus::Unavailable
        );
        assert_eq!(
            repository.get_sync("legacy").unwrap().unwrap().status,
            CredentialStatus::Unavailable
        );
    }

    #[tokio::test]
    async fn running_credential_cannot_be_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let repository = FilesystemCredentialRepository::new(temp.path().into());
        let credential = Credential {
            id: "shared".into(),
            name: "shared".into(),
            credential_type: CredentialType::new("openai_api_key").unwrap(),
            configuration: Default::default(),
            created_at: 0,
            updated_at: 0,
            status: CredentialStatus::Available,
        };
        repository
            .save(&credential, Some("original"))
            .await
            .unwrap();
        let lease = repository.acquire_run_leases(&["shared".into()]).unwrap();
        assert!(
            repository
                .save(&credential, Some("replacement"))
                .await
                .is_err()
        );
        assert_eq!(
            repository.secret_sync("shared").unwrap().as_deref(),
            Some("original")
        );
        drop(lease);
        repository
            .save(&credential, Some("replacement"))
            .await
            .unwrap();
    }

    #[test]
    fn run_and_delete_leases_exclude_each_other_and_release_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let repository = FilesystemCredentialRepository::new(temp.path().into());
        let run = repository
            .acquire_run_leases(&["a".into(), "a".into()])
            .unwrap();
        let second_run = repository.acquire_run_leases(&["a".into()]).unwrap();
        assert!(repository.acquire_delete_lease("a").is_err());
        drop(run);
        assert!(repository.acquire_delete_lease("a").is_err());
        drop(second_run);
        let delete = repository.acquire_delete_lease("a").unwrap();
        assert!(
            repository
                .acquire_run_leases(&["a".into(), "b".into()])
                .is_err()
        );
        assert!(repository.acquire_delete_lease("b").is_ok());
        drop(delete);
        assert!(repository.acquire_run_leases(&["a".into()]).is_ok());
    }

    #[tokio::test]
    async fn same_shared_id_resolves_to_each_local_vaults_secret() {
        let temp = tempfile::tempdir().unwrap();
        let first = FilesystemCredentialRepository::new(temp.path().join("first"));
        let second = FilesystemCredentialRepository::new(temp.path().join("second"));
        let credential = Credential {
            id: "openai-default".into(),
            name: "openai-default".into(),
            credential_type: CredentialType::new("openai_api_key").unwrap(),
            configuration: Default::default(),
            created_at: 0,
            updated_at: 0,
            status: CredentialStatus::Available,
        };
        first.save(&credential, Some("first-secret")).await.unwrap();
        second
            .save(&credential, Some("second-secret"))
            .await
            .unwrap();
        assert_eq!(
            first.get_sync("openai-default").unwrap().unwrap().id,
            credential.id
        );
        assert_eq!(
            second.get_sync("openai-default").unwrap().unwrap().id,
            credential.id
        );
        assert_eq!(
            first.secret_sync("openai-default").unwrap().as_deref(),
            Some("first-secret")
        );
        assert_eq!(
            second.secret_sync("openai-default").unwrap().as_deref(),
            Some("second-secret")
        );
    }

    #[test]
    fn project_vaults_are_isolated_and_identity_survives_move() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        let moved = temp.path().join("moved");
        let vault = temp.path().join("vault");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let first_root = FilesystemCredentialRepository::for_project_in(&first, &vault)
            .unwrap()
            .root;
        let second_root = FilesystemCredentialRepository::for_project_in(&second, &vault)
            .unwrap()
            .root;
        assert_ne!(first_root, second_root);
        std::fs::rename(&first, &moved).unwrap();
        assert_eq!(
            first_root,
            FilesystemCredentialRepository::for_project_in(&moved, &vault)
                .unwrap()
                .root
        );
    }

    #[tokio::test]
    async fn deletion_failure_preserves_metadata_for_retry() {
        let temp = tempfile::tempdir().unwrap();
        let repository = FilesystemCredentialRepository::new(temp.path().into());
        let credential = Credential {
            id: "broken".into(),
            name: "Broken".into(),
            credential_type: CredentialType::new("openai_api_key").unwrap(),
            configuration: Default::default(),
            created_at: 0,
            updated_at: 0,
            status: CredentialStatus::MissingSecret,
        };
        repository.save(&credential, None).await.unwrap();
        std::fs::create_dir(repository.secret_path("broken")).unwrap();
        assert!(repository.delete("broken").await.is_err());
        assert!(repository.get("broken").await.unwrap().is_some());
        std::fs::remove_dir(repository.secret_path("broken")).unwrap();
        repository.delete("broken").await.unwrap();
        assert!(repository.get("broken").await.unwrap().is_none());
    }

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
