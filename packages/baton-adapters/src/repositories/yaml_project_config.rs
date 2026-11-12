use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use baton_core::{
    domain::project::{Agent, ConfigSnapshot, ConfigState, Pipeline, ProjectConfig},
    ports::project_config::ProjectConfigRepository,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectMetadata {
    schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
}

pub struct YamlProjectConfigRepository {
    project_dir: PathBuf,
}

impl YamlProjectConfigRepository {
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }
}

fn read_files(root: &Path) -> Result<Option<BTreeMap<String, String>>, String> {
    let baton = root.join(".baton");
    let metadata = baton.join("project.yml");
    let metadata_contents = match fs::read_to_string(&metadata) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if baton.join("config.json").exists() {
                return Err(
                    "Legacy .baton/config.json is not supported; migrate it before continuing"
                        .into(),
                );
            }
            for directory in ["agents", "pipelines"] {
                match fs::read_dir(baton.join(directory)) {
                    Ok(entries) => {
                        for entry in entries {
                            let entry = entry.map_err(|e| e.to_string())?;
                            if entry
                                .path()
                                .extension()
                                .is_some_and(|extension| extension == "yml")
                            {
                                return Err(
                                    "Configuration files exist without .baton/project.yml".into()
                                );
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            return Ok(None);
        }
        Err(e) => return Err(format!("Could not read {}: {e}", metadata.display())),
    };
    let mut files = BTreeMap::from([("project.yml".to_owned(), metadata_contents)]);
    for directory in ["agents", "pipelines"] {
        let path = baton.join(directory);
        if fs::symlink_metadata(&path).is_ok_and(|metadata| !metadata.is_dir()) {
            return Err(format!("Not a configuration directory: {}", path.display()));
        }
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Could not read {}: {e}", path.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "yml") {
                if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                    return Err(format!("Not a regular file: {}", path.display()));
                }
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "Invalid configuration filename")?;
                let contents = fs::read_to_string(&path)
                    .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
                files.insert(format!("{directory}/{name}"), contents);
            }
        }
    }
    Ok(Some(files))
}

fn decode(files: &BTreeMap<String, String>) -> Result<ProjectConfig, String> {
    let metadata: ProjectMetadata = serde_yaml_ng::from_str(&files["project.yml"])
        .map_err(|e| format!("Invalid project.yml: {e}"))?;
    let mut config = ProjectConfig {
        schema_version: metadata.schema_version,
        project_id: metadata.project_id,
        agents: vec![],
        pipelines: vec![],
    };
    for (path, contents) in files {
        if let Some(name) = path.strip_prefix("agents/") {
            let agent: Agent =
                serde_yaml_ng::from_str(contents).map_err(|e| format!("Invalid {path}: {e}"))?;
            if name != format!("{}.yml", agent.id) {
                return Err(format!("Agent ID does not match filename: {path}"));
            }
            config.agents.push(agent);
        } else if let Some(name) = path.strip_prefix("pipelines/") {
            let pipeline: Pipeline =
                serde_yaml_ng::from_str(contents).map_err(|e| format!("Invalid {path}: {e}"))?;
            if name != format!("{}.yml", pipeline.id) {
                return Err(format!("Pipeline ID does not match filename: {path}"));
            }
            config.pipelines.push(pipeline);
        }
    }
    config.validate()?;
    Ok(config)
}

fn encode(config: &ProjectConfig) -> Result<BTreeMap<String, String>, String> {
    config.validate()?;
    fn yaml(value: &impl Serialize) -> Result<String, String> {
        serde_yaml_ng::to_string(value).map_err(|e| e.to_string())
    }
    let metadata = ProjectMetadata {
        schema_version: config.schema_version,
        project_id: config.project_id.clone(),
    };
    let mut files = BTreeMap::from([("project.yml".to_owned(), yaml(&metadata)?)]);
    for agent in &config.agents {
        files.insert(format!("agents/{}.yml", agent.id), yaml(agent)?);
    }
    for pipeline in &config.pipelines {
        files.insert(format!("pipelines/{}.yml", pipeline.id), yaml(pipeline)?);
    }
    Ok(files)
}

fn write_files(root: &Path, expected: Option<&str>, config: &ProjectConfig) -> Result<(), String> {
    let mut files = encode(config)?;
    let baton = root.join(".baton");
    fs::create_dir_all(&baton).map_err(|e| e.to_string())?;
    let lock = fs::File::open(root).map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|e| format!("Could not lock project configuration: {e}"))?;
    let current = read_files(root)?;
    match (expected, &current) {
        (Some(expected), Some(current))
            if serde_json::to_string(current).map_err(|e| e.to_string())? == expected => {}
        (Some(_), _) => return Err("Configuration changed; reload before updating.".into()),
        (None, Some(_)) => return Err("A Baton project configuration already exists.".into()),
        (None, None) => {}
    }
    if let Some(old_files) = &current {
        let old_config = decode(old_files)?;
        if old_config.schema_version == config.schema_version
            && old_config.project_id == config.project_id
        {
            files.insert("project.yml".into(), old_files["project.yml"].clone());
        }
        for agent in &config.agents {
            if old_config.agents.iter().any(|old| old == agent) {
                let path = format!("agents/{}.yml", agent.id);
                files.insert(path.clone(), old_files[&path].clone());
            }
        }
        for pipeline in &config.pipelines {
            if old_config.pipelines.iter().any(|old| old == pipeline) {
                let path = format!("pipelines/{}.yml", pipeline.id);
                files.insert(path.clone(), old_files[&path].clone());
            }
        }
    }
    for directory in ["agents", "pipelines"] {
        let path = baton.join(directory);
        if fs::symlink_metadata(&path).is_ok_and(|metadata| !metadata.is_dir()) {
            return Err(format!("Not a configuration directory: {}", path.display()));
        }
        fs::create_dir_all(path).map_err(|e| e.to_string())?;
    }
    let ignore = baton.join(".gitignore");
    if !ignore.exists() {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&ignore)
        {
            Ok(mut file) => {
                file.write_all(b"runs/\ntasks/\n")
                    .map_err(|e| e.to_string())?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    // Stage each file before publishing. Keep the previous contents for rollback on I/O errors.
    let suffix = uuid::Uuid::now_v7();
    let mut staged = Vec::new();
    for (path, contents) in &files {
        if current.as_ref().and_then(|old| old.get(path)) == Some(contents) {
            continue;
        }
        let target = baton.join(path);
        let temporary = target.with_extension(format!("yml.{suffix}.tmp"));
        let result = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .and_then(|mut file| {
                let result = file
                    .write_all(contents.as_bytes())
                    .and_then(|()| file.sync_all());
                if result.is_err() {
                    let _ = fs::remove_file(&temporary);
                }
                result
            });
        if let Err(error) = result {
            for (_, temporary) in &staged {
                let _ = fs::remove_file(temporary);
            }
            return Err(format!("Could not stage {}: {error}", target.display()));
        }
        staged.push((target, temporary));
    }
    match read_files(root) {
        Ok(latest) if latest == current => {}
        result => {
            for (_, temporary) in &staged {
                let _ = fs::remove_file(temporary);
            }
            return match result {
                Ok(_) => Err("Configuration changed; reload before updating.".into()),
                Err(error) => Err(error),
            };
        }
    }
    let mut published = Vec::new();
    let mut publish = || -> Result<(), String> {
        for (target, temporary) in &staged {
            fs::rename(temporary, target)
                .map_err(|e| format!("Could not update {}: {e}", target.display()))?;
            published.push(target.clone());
        }
        if let Some(current) = &current {
            for path in current.keys().filter(|path| !files.contains_key(*path)) {
                let target = baton.join(path);
                fs::remove_file(&target).map_err(|e| format!("Could not remove {path}: {e}"))?;
                published.push(target);
            }
        }
        Ok(())
    };
    if let Err(error) = publish() {
        for (_, temporary) in &staged {
            let _ = fs::remove_file(temporary);
        }
        let mut rollback_errors = Vec::new();
        for target in published {
            let path = target
                .strip_prefix(&baton)
                .expect("published path is within .baton");
            let path = path.to_string_lossy();
            let result =
                if let Some(contents) = current.as_ref().and_then(|old| old.get(path.as_ref())) {
                    let temporary = target.with_extension(format!("yml.{suffix}.rollback"));
                    fs::write(&temporary, contents).and_then(|()| fs::rename(&temporary, &target))
                } else {
                    fs::remove_file(&target)
                };
            if let Err(error) = result {
                rollback_errors.push(format!("{}: {error}", target.display()));
            }
        }
        if !rollback_errors.is_empty() {
            return Err(format!(
                "{error}; rollback failed: {}",
                rollback_errors.join(", ")
            ));
        }
        return Err(error);
    }
    Ok(())
}

#[async_trait]
impl ProjectConfigRepository for YamlProjectConfigRepository {
    async fn load(&self) -> Result<ConfigState, String> {
        let root = self.project_dir.clone();
        tokio::task::spawn_blocking(move || {
            let lock = fs::File::open(&root).map_err(|e| e.to_string())?;
            lock.try_lock_shared()
                .map_err(|e| format!("Could not lock project configuration: {e}"))?;
            let Some(files) = read_files(&root)? else {
                return Ok(ConfigState::Missing);
            };
            match decode(&files) {
                Ok(config) => Ok(ConfigState::Ready(ConfigSnapshot {
                    config,
                    revision: serde_json::to_string(&files).map_err(|e| e.to_string())?,
                })),
                Err(error) => Ok(ConfigState::Invalid(error)),
            }
        })
        .await
        .map_err(|e| e.to_string())?
    }

    async fn create(&self, config: &ProjectConfig) -> Result<(), String> {
        let root = self.project_dir.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || write_files(&root, None, &config))
            .await
            .map_err(|e| e.to_string())?
    }

    async fn update(&self, expected_revision: &str, config: &ProjectConfig) -> Result<(), String> {
        let root = self.project_dir.clone();
        let expected = expected_revision.to_owned();
        let config = config.clone();
        tokio::task::spawn_blocking(move || write_files(&root, Some(&expected), &config))
            .await
            .map_err(|e| e.to_string())?
    }
}

pub fn ensure_project_identity(root: &Path) -> Result<String, String> {
    let files = read_files(root)?;
    let mut config = match &files {
        Some(files) => decode(files)?,
        None => ProjectConfig::CURRENT,
    };
    if let Some(id) = &config.project_id {
        return Ok(id.to_ascii_lowercase());
    }
    let id = uuid::Uuid::now_v7().to_string();
    config.project_id = Some(id.clone());
    let revision = files
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| e.to_string())?;
    write_files(root, revision.as_deref(), &config)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_stable_and_does_not_overwrite_orphaned_files() {
        let root = tempfile::tempdir().unwrap();
        let first = ensure_project_identity(root.path()).unwrap();
        assert_eq!(ensure_project_identity(root.path()).unwrap(), first);
        let credentials = crate::credentials::FilesystemCredentialRepository::for_project_in(
            root.path(),
            root.path(),
        )
        .unwrap();
        assert_eq!(
            credentials.root_path(),
            root.path().join("projects").join(first)
        );

        let other = tempfile::tempdir().unwrap();
        let directory = other.path().join(".baton/agents");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("keep.yml"), "important").unwrap();
        assert!(ensure_project_identity(other.path()).is_err());
        assert_eq!(
            fs::read_to_string(directory.join("keep.yml")).unwrap(),
            "important"
        );
    }

    #[tokio::test]
    async fn repository_and_examples_are_valid() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for root in [
            workspace.clone(),
            workspace.join("examples/demo"),
            workspace.join("examples/session-factory"),
        ] {
            let repo = YamlProjectConfigRepository::new(root.clone());
            assert!(
                matches!(repo.load().await.unwrap(), ConfigState::Ready(_)),
                "{}",
                root.display()
            );
        }
    }

    #[tokio::test]
    async fn saves_preserve_unchanged_files_and_remove_deleted_definitions() {
        let root = tempfile::tempdir().unwrap();
        let repo = YamlProjectConfigRepository::new(root.path().to_owned());
        let mut config = ProjectConfig::CURRENT;
        config.agents.push(Agent {
            id: "test".into(),
            instructions: "hello".into(),
            runtime_kind: Default::default(),
            model: None,
            credential_id: None,
            skill_ids: vec![],
            allowed_tools: None,
        });
        repo.create(&config).await.unwrap();
        let agent_path = root.path().join(".baton/agents/test.yml");
        let original = fs::read_to_string(&agent_path).unwrap();
        fs::write(&agent_path, format!("# Keep this comment\n{original}")).unwrap();
        let ConfigState::Ready(snapshot) = repo.load().await.unwrap() else {
            panic!("expected configuration")
        };
        repo.update(&snapshot.revision, &snapshot.config)
            .await
            .unwrap();
        assert!(
            fs::read_to_string(&agent_path)
                .unwrap()
                .starts_with("# Keep this comment")
        );
        let ConfigState::Ready(snapshot) = repo.load().await.unwrap() else {
            panic!("expected configuration")
        };
        config.agents.clear();
        repo.update(&snapshot.revision, &config).await.unwrap();
        assert!(!agent_path.exists());
    }

    #[tokio::test]
    async fn yaml_files_round_trip_and_detect_external_changes() {
        let root = tempfile::tempdir().unwrap();
        let repo = YamlProjectConfigRepository::new(root.path().to_owned());
        let mut config = ProjectConfig::CURRENT;
        config.agents.push(Agent {
            id: "test".into(),
            instructions: "hello".into(),
            runtime_kind: Default::default(),
            model: None,
            credential_id: None,
            skill_ids: vec![],
            allowed_tools: None,
        });
        repo.create(&config).await.unwrap();
        assert!(root.path().join(".baton/agents/test.yml").exists());
        let ConfigState::Ready(snapshot) = repo.load().await.unwrap() else {
            panic!("expected configuration")
        };
        assert_eq!(snapshot.config, config);
        fs::write(
            root.path().join(".baton/agents/test.yml"),
            "invalid: true\n",
        )
        .unwrap();
        assert!(repo.update(&snapshot.revision, &config).await.is_err());
        assert!(matches!(
            repo.load().await.unwrap(),
            ConfigState::Invalid(_)
        ));
    }
}
