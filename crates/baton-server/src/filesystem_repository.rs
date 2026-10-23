use std::path::PathBuf;

use async_trait::async_trait;
use baton_core::{
    project::{ConfigState, ProjectConfig, parse_config},
    project_config::ProjectConfigRepository,
};
use tokio::fs;

pub struct FilesystemProjectConfigRepository {
    project_dir: PathBuf,
}

impl FilesystemProjectConfigRepository {
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }
    fn config_path(&self) -> PathBuf {
        self.project_dir.join(".baton").join("config.json")
    }
}

#[async_trait]
impl ProjectConfigRepository for FilesystemProjectConfigRepository {
    async fn load(&self) -> Result<ConfigState, String> {
        match fs::read_to_string(self.config_path()).await {
            Ok(contents) => match parse_config(&contents) {
                Ok(config) => Ok(ConfigState::Ready(config)),
                Err(message) => Ok(ConfigState::Invalid(message)),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ConfigState::Missing),
            Err(error) => Err(format!("Could not read project configuration: {error}")),
        }
    }

    async fn create(&self, config: &ProjectConfig) -> Result<(), String> {
        let config_path = self.config_path();
        fs::create_dir_all(
            config_path
                .parent()
                .expect("configuration path has a parent"),
        )
        .await
        .map_err(|error| error.to_string())?;
        let temporary_path = config_path.with_extension(format!("{}.tmp", unique_suffix()));
        let contents = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
        fs::write(&temporary_path, contents)
            .await
            .map_err(|error| error.to_string())?;
        let result = fs::hard_link(&temporary_path, &config_path).await;
        let _ = fs::remove_file(&temporary_path).await;
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err("A Baton project configuration already exists.".into())
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

fn unique_suffix() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
