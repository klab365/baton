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
        self.persist(None, config).await
    }
    async fn update(&self, expected_revision: &str, config: &ProjectConfig) -> Result<(), String> {
        self.persist(Some(expected_revision), config).await
    }
}

/// Persist a project identity before opening its vault. Call while holding the project bootstrap lock.
pub fn ensure_project_identity(project_dir: &Path) -> Result<String, String> {
    let baton_dir = project_dir.join(".baton");
    fs::create_dir_all(&baton_dir).map_err(|_| "Could not create project directory")?;
    ensure_gitignore(&baton_dir)?;
    let path = baton_dir.join("config.json");
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("Could not read project configuration".into()),
    };
    let mut value = match contents.as_deref() {
        Some(contents) => serde_json::from_str::<serde_json::Value>(contents)
            .map_err(|_| "Invalid project configuration")?,
        None => serde_json::to_value(ProjectConfig::CURRENT).map_err(|error| error.to_string())?,
    };
    let mut validation_value = value.clone();
    if let Some(object) = validation_value.as_object_mut() {
        object.remove("skills");
    }
    let config = parse_config(&validation_value.to_string())?;
    if let Some(id) = &config.project_id {
        return Ok(id.to_ascii_lowercase());
    }
    let legacy_identity = project_dir.join(".baton/runs/credential-project-id");
    let id = match fs::read_to_string(legacy_identity) {
        Ok(value) => uuid::Uuid::parse_str(value.trim())
            .map_err(|_| "Existing local project identity is invalid")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => uuid::Uuid::now_v7(),
        Err(_) => return Err("Could not read existing local project identity".into()),
    };
    if value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        == Some(1)
    {
        value = serde_json::to_value(config).map_err(|error| error.to_string())?;
    }
    value
        .as_object_mut()
        .ok_or("Invalid project configuration")?
        .insert(
            "projectId".into(),
            serde_json::Value::String(id.to_string()),
        );
    let serialized = serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?;
    persist(&path, contents.as_deref(), &serialized)?;
    Ok(id.to_string())
}

struct RemoveOnDrop(PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn ensure_gitignore(parent: &Path) -> Result<(), String> {
    let path = parent.join(".gitignore");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => file
            .write_all(b"runs/\ntasks/\n")
            .map_err(|error| format!("Could not write {}: {error}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(format!("Could not create {}: {error}", path.display())),
    }
    Ok(())
}

struct ConfigurationLock(fs::File);

impl Drop for ConfigurationLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn persist(path: &Path, expected: Option<&str>, contents: &[u8]) -> Result<(), String> {
    let parent = path.parent().expect("configuration path has a parent");
    fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create configuration directory: {e}"))?;
    let project_root = parent
        .parent()
        .expect("configuration directory has a parent");
    let lock = fs::File::open(project_root)
        .map_err(|e| format!("Could not open configuration lock: {e}"))?;
    lock.try_lock()
        .map_err(|e| format!("Could not lock configuration (another writer may be active): {e}"))?;
    // Explicitly unlock even if a forked process inherited the file descriptor.
    let _lock = ConfigurationLock(lock);
    ensure_gitignore(parent)?;
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
    // Recheck immediately before publishing; cooperating writers hold the project directory lock throughout.
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

    #[test]
    fn bootstrap_migrates_legacy_identity_without_losing_configuration() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.0.join(".baton/runs")).unwrap();
        let id = "018f0000-0000-7000-8000-000000000001";
        fs::write(fixture.0.join(".baton/runs/credential-project-id"), id).unwrap();
        fs::write(
            fixture.0.join(".baton/config.json"),
            r#"{"schemaVersion":7,"agents":[],"pipelines":[]}"#,
        )
        .unwrap();
        assert_eq!(ensure_project_identity(&fixture.0).unwrap(), id);
        let config =
            parse_config(&fs::read_to_string(fixture.0.join(".baton/config.json")).unwrap())
                .unwrap();
        assert_eq!(config.project_id.as_deref(), Some(id));
        assert_eq!(ensure_project_identity(&fixture.0).unwrap(), id);
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
    async fn orphan_lock_file_does_not_block_updates() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        initialize_project(&repo).await.unwrap();
        let initial = snapshot(&repo).await;
        let lock = repo.config_path().with_file_name("config.lock");
        fs::write(&lock, "12345\n").unwrap();
        repo.update(&initial.revision, &initial.config)
            .await
            .unwrap();
        assert!(lock.exists());
    }

    #[test]
    fn configuration_lock_releases_even_with_a_duplicated_handle() {
        let fixture = Fixture::new();
        fs::create_dir_all(&fixture.0).unwrap();
        let file = fs::File::open(&fixture.0).unwrap();
        file.try_lock().unwrap();
        let duplicate = file.try_clone().unwrap();
        drop(ConfigurationLock(file));
        let other = fs::File::open(&fixture.0).unwrap();
        other.try_lock().unwrap();
        drop(duplicate);
    }

    #[tokio::test]
    async fn failures_preserve_existing_bytes() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        initialize_project(&repo).await.unwrap();
        let initial = snapshot(&repo).await;
        let held_lock = fs::File::open(&fixture.0).unwrap();
        held_lock.try_lock().unwrap();
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
        drop(held_lock);
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
            skill_ids: vec![],
            allowed_tools: None,
        }
    }

    fn sample_pipeline() -> baton_core::domain::project::Pipeline {
        use baton_core::domain::project::{Pipeline, PipelineStep};
        Pipeline {
            id: "main".into(),
            steps: vec![PipelineStep {
                command: None,
                timeout_seconds: None,
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
    async fn git_cleanup_cannot_bypass_active_lock() {
        let fixture = Fixture::new();
        fs::create_dir_all(&fixture.0).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&fixture.0)
                .status()
                .unwrap()
                .success()
        );
        let repo = fixture.repo();
        initialize_project(&repo).await.unwrap();
        let initial = snapshot(&repo).await;
        assert!(
            std::process::Command::new("git")
                .args(["add", ".baton/config.json", ".baton/.gitignore"])
                .current_dir(&fixture.0)
                .status()
                .unwrap()
                .success()
        );
        let held = fs::File::open(&fixture.0).unwrap();
        held.try_lock().unwrap();
        fs::write(fixture.0.join(".baton/config.lock"), "orphan").unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["clean", "-fdxq"])
                .current_dir(&fixture.0)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            std::process::Command::new("git")
                .args(["restore", "--worktree", ".baton"])
                .current_dir(&fixture.0)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            repo.update(&initial.revision, &initial.config)
                .await
                .unwrap_err()
                .contains("lock")
        );
        drop(held);
        repo.update(&initial.revision, &initial.config)
            .await
            .unwrap();
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
