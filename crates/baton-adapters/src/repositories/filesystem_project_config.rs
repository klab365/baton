use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use baton_core::{
    domain::project::{ConfigSnapshot, ConfigState, ProjectConfig, parse_config},
    ports::project_config::ProjectConfigRepository,
};

pub struct FilesystemProjectConfigRepository {
    project_dir: PathBuf,
}

impl FilesystemProjectConfigRepository {
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }
    fn baton_dir(&self) -> PathBuf {
        self.project_dir.join(".baton")
    }
    fn config_path(&self) -> PathBuf {
        self.baton_dir().join("config.json")
    }
    async fn ensure_runs_gitignore(&self) -> Result<(), String> {
        let path = self.baton_dir().join(".gitignore");
        tokio::task::spawn_blocking(move || {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => file
                    .write_all(b"runs/\ntasks/\n")
                    .map_err(|e| format!("Could not write {}: {e}", path.display())),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                Err(error) => Err(format!("Could not create {}: {error}", path.display())),
            }
        })
        .await
        .map_err(|e| format!("Gitignore write task failed: {e}"))?
    }

    async fn persist(&self, expected: Option<&str>, config: &ProjectConfig) -> Result<(), String> {
        config.validate()?;
        let contents = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
        let path = self.config_path();
        let expected = expected.map(str::to_owned);
        tokio::task::spawn_blocking(move || persist(&path, expected.as_deref(), &contents))
            .await
            .map_err(|e| format!("Configuration write task failed: {e}"))?
    }
}

#[async_trait]
impl ProjectConfigRepository for FilesystemProjectConfigRepository {
    async fn load(&self) -> Result<ConfigState, String> {
        match tokio::fs::read_to_string(self.config_path()).await {
            Ok(mut contents) => {
                let mut value: serde_json::Value = match serde_json::from_str(&contents) {
                    Ok(value) => value,
                    Err(error) => {
                        return Ok(ConfigState::Invalid(format!(
                            "Invalid project configuration: {error}"
                        )));
                    }
                };
                if let Some(inline) = value.get("skills").and_then(serde_json::Value::as_array) {
                    use crate::skills::FilesystemSkillRepository;
                    use baton_core::ports::skills::SkillRepository;
                    let skills: Vec<baton_core::domain::project::Skill> = inline
                        .iter()
                        .map(|value| {
                            let object =
                                value.as_object().ok_or("Inline skill must be an object")?;
                            let id = object
                                .get("id")
                                .and_then(serde_json::Value::as_str)
                                .ok_or("Inline skill has no ID")?
                                .to_owned();
                            let description = object
                                .get("description")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("")
                                .to_owned();
                            let instructions = object
                                .get("instructions")
                                .and_then(serde_json::Value::as_str)
                                .ok_or("Inline skill has no instructions")?
                                .to_owned();
                            Ok::<_, String>(baton_core::domain::project::Skill {
                                id,
                                description,
                                metadata: String::new(),
                                instructions,
                            })
                        })
                        .collect::<Result<_, _>>()?;
                    value.as_object_mut().unwrap().remove("skills");
                    let config = parse_config(&value.to_string())?;
                    let repository = FilesystemSkillRepository::new(self.project_dir.clone());
                    let existing = repository.list()?;
                    let mut ids = std::collections::HashSet::new();
                    for skill in &skills {
                        baton_core::features::skills::validate(skill)?;
                        if !ids.insert(&skill.id) {
                            return Err(format!("Duplicate inline skill ID {:?}", skill.id));
                        }
                        if existing.iter().any(|item| item.id == skill.id) {
                            return Err(format!(
                                "Cannot migrate inline skill {:?}: folder already exists; reconcile it before removing the inline list",
                                skill.id
                            ));
                        }
                    }
                    for skill in &skills {
                        repository.save(skill)?;
                    }
                    self.persist(Some(&contents), &config).await?;
                    contents = tokio::fs::read_to_string(self.config_path())
                        .await
                        .map_err(|error| error.to_string())?;
                }
                match parse_config(&contents) {
                    Ok(config) => Ok(ConfigState::Ready(ConfigSnapshot {
                        config,
                        revision: contents,
                    })),
                    Err(message) => Ok(ConfigState::Invalid(message)),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ConfigState::Missing),
            Err(error) => Err(format!("Could not read project configuration: {error}")),
        }
    }
    async fn create(&self, config: &ProjectConfig) -> Result<(), String> {
        self.persist(None, config).await?;
        self.ensure_runs_gitignore().await
    }
    async fn update(&self, expected_revision: &str, config: &ProjectConfig) -> Result<(), String> {
        self.persist(Some(expected_revision), config).await
    }
}

struct RemoveOnDrop(PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn persist(path: &Path, expected: Option<&str>, contents: &[u8]) -> Result<(), String> {
    let parent = path.parent().expect("configuration path has a parent");
    fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create configuration directory: {e}"))?;
    let lock_path = parent.join("config.lock");
    let mut lock = fs::OpenOptions::new().write(true).create_new(true).open(&lock_path)
        .map_err(|e| format!("Could not lock configuration (another writer or stale .baton/config.lock; retry after checking no writer is active): {e}"))?;
    let _lock_cleanup = RemoveOnDrop(lock_path);
    writeln!(lock, "{}", std::process::id()).map_err(|e| e.to_string())?;
    match expected {
        Some(revision) => {
            let current = fs::read_to_string(path)
                .map_err(|e| format!("Could not read configuration for update: {e}"))?;
            if current != revision {
                return Err("Configuration changed; reload before updating.".into());
            }
        }
        None if path.exists() => return Err("A Baton project configuration already exists.".into()),
        None => {}
    }
    let temporary_path = parent.join(format!("config.{}.tmp", unique_suffix()));
    let mut temporary = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .map_err(|e| format!("Could not create temporary configuration: {e}"))?;
    let _temporary_cleanup = RemoveOnDrop(temporary_path.clone());
    temporary
        .write_all(contents)
        .and_then(|()| temporary.sync_all())
        .map_err(|e| format!("Could not write configuration: {e}"))?;
    drop(temporary);
    // Recheck immediately before publishing; cooperating writers hold config.lock throughout.
    if let Some(revision) = expected {
        if fs::read_to_string(path).map_err(|e| e.to_string())? != revision {
            return Err("Configuration changed; reload before updating.".into());
        }
        fs::rename(&temporary_path, path)
            .map_err(|e| format!("Could not replace configuration: {e}"))?;
    } else {
        fs::hard_link(&temporary_path, path)
            .map_err(|e| format!("Could not create configuration (it may already exist): {e}"))?;
    }
    Ok(())
}

fn unique_suffix() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use baton_core::features::project_config::{
        get_project_config, initialize_project, update_project_config,
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("baton-test-{}", unique_suffix())))
        }
        fn repo(&self) -> FilesystemProjectConfigRepository {
            FilesystemProjectConfigRepository::new(self.0.clone())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    async fn snapshot(repo: &FilesystemProjectConfigRepository) -> ConfigSnapshot {
        match get_project_config(repo).await.unwrap() {
            ConfigState::Ready(snapshot) => snapshot,
            state => panic!("Expected ready, got {state:?}"),
        }
    }

    #[tokio::test]
    async fn lifecycle_and_stale_update() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        assert_eq!(repo.load().await.unwrap(), ConfigState::Missing);
        initialize_project(&repo).await.unwrap();
        assert_eq!(
            fs::read_to_string(fixture.0.join(".baton/.gitignore")).unwrap(),
            "runs/\ntasks/\n"
        );
        assert!(initialize_project(&repo).await.is_err());
        let initial = snapshot(&repo).await;
        let mut changed = initial.config.clone();
        changed.agents.push(sample_agent());
        changed.pipelines.push(sample_pipeline());
        update_project_config(&repo, &initial.revision, &changed)
            .await
            .unwrap();
        assert!(
            update_project_config(&repo, &initial.revision, &initial.config)
                .await
                .unwrap_err()
                .contains("changed")
        );
        assert_eq!(snapshot(&repo).await.config, changed);
        let mut invalid = changed.clone();
        invalid.schema_version = 99;
        assert!(repo.update(&initial.revision, &invalid).await.is_err());
        assert_eq!(snapshot(&repo).await.config, changed);
    }

    #[tokio::test]
    async fn legacy_reads_do_not_write_and_can_be_upgraded() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        fs::create_dir_all(repo.config_path().parent().unwrap()).unwrap();
        let legacy = "{\"schemaVersion\":1}";
        fs::write(repo.config_path(), legacy).unwrap();
        let old = snapshot(&repo).await;
        assert_eq!(fs::read_to_string(repo.config_path()).unwrap(), legacy);
        repo.update(&old.revision, &old.config).await.unwrap();
        assert_eq!(snapshot(&repo).await.config.schema_version, 7);
    }

    #[tokio::test]
    async fn failures_preserve_existing_bytes() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        initialize_project(&repo).await.unwrap();
        let initial = snapshot(&repo).await;
        let lock = repo.config_path().with_file_name("config.lock");
        fs::write(&lock, "held").unwrap();
        assert!(
            repo.update(&initial.revision, &initial.config)
                .await
                .unwrap_err()
                .contains("lock")
        );
        assert_eq!(
            fs::read_to_string(repo.config_path()).unwrap(),
            initial.revision
        );
        assert!(lock.exists());
        fs::remove_file(lock).unwrap();
        fs::write(repo.config_path(), "broken").unwrap();
        assert!(matches!(
            repo.load().await.unwrap(),
            ConfigState::Invalid(_)
        ));
        assert!(
            repo.update(&initial.revision, &initial.config)
                .await
                .is_err()
        );
        fs::remove_file(repo.config_path()).unwrap();
        fs::create_dir(repo.config_path()).unwrap();
        assert!(repo.load().await.is_err());
        assert!(
            repo.update(&initial.revision, &initial.config)
                .await
                .is_err()
        );
        let blocked = Fixture::new();
        fs::write(&blocked.0, "not a directory").unwrap();
        assert!(initialize_project(&blocked.repo()).await.is_err());
        fs::remove_file(&blocked.0).unwrap();
    }

    fn sample_agent() -> baton_core::domain::project::Agent {
        baton_core::domain::project::Agent {
            id: "writer".into(),
            system_prompt: "Write clearly".into(),
            runtime_kind: baton_core::domain::runtime::RuntimeKind::Rig,
            model: Some(baton_core::domain::runtime::RuntimeModel {
                provider: "openai".into(),
                model: "gpt-4.1".into(),
            }),
            credential_id: None,
            rules: vec![],
            skill_ids: vec![],
        }
    }

    fn sample_pipeline() -> baton_core::domain::project::Pipeline {
        use baton_core::domain::project::{Pipeline, PipelineStep};
        Pipeline {
            id: "main".into(),
            name: "Main".into(),
            steps: vec![PipelineStep {
                human: false,
                transitions: vec![],
                id: "plan".into(),
                name: "Plan".into(),
                agent_id: "writer".into(),
            }],
        }
    }

    #[tokio::test]
    async fn initialization_preserves_existing_baton_gitignore() {
        let fixture = Fixture::new();
        let baton = fixture.0.join(".baton");
        fs::create_dir_all(&baton).unwrap();
        fs::write(baton.join(".gitignore"), "custom/\n").unwrap();
        initialize_project(&fixture.repo()).await.unwrap();
        assert_eq!(
            fs::read_to_string(baton.join(".gitignore")).unwrap(),
            "custom/\n"
        );
    }

    #[tokio::test]
    async fn independent_writers_cannot_both_commit() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        initialize_project(&repo).await.unwrap();
        let initial = snapshot(&repo).await;
        let mut changed = initial.config;
        changed.agents.push(sample_agent());
        changed.pipelines.push(sample_pipeline());
        let other = fixture.repo();
        let (a, b) = tokio::join!(
            repo.update(&initial.revision, &changed),
            other.update(&initial.revision, &changed)
        );
        assert_ne!(a.is_ok(), b.is_ok());
        assert_eq!(snapshot(&repo).await.config, changed);
        let names: Vec<_> = fs::read_dir(repo.config_path().parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&std::ffi::OsString::from("config.json")));
        assert!(names.contains(&std::ffi::OsString::from(".gitignore")));
    }
}
