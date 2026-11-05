//! Private state and OS lock for standalone Work invocations; no server/project-identity lock.
use baton_core::{
    domain::pipeline::PipelineRun,
    ports::runs::{RunActionLease, RunRepository},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub struct WorkProcessLock {
    _file: fs::File,
}
impl WorkProcessLock {
    pub fn acquire(workspace: &Path) -> Result<Self, String> {
        let directory = workspace.join(".baton/work");
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(directory.join("process.lock"))
            .map_err(|e| e.to_string())?;
        file.try_lock().map_err(|e| {
            format!(
                "Another work command is using this workspace, or its lock cannot be acquired: {e}"
            )
        })?;
        // Standalone Work does not bootstrap server state, so protect its own local records.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(".gitignore"))
        {
            Ok(mut file) => file.write_all(b"*\n").map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
        Ok(Self { _file: file })
    }
}

pub struct FilesystemWorkRepository(pub PathBuf);
impl FilesystemWorkRepository {
    fn directory(&self) -> PathBuf {
        self.0.join(".baton/work")
    }
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err("Work ID must contain only ASCII letters, digits or hyphens".into());
        }
        Ok(self.directory().join(format!("{id}.json")))
    }
    fn persist(&self, run: &PipelineRun, create: bool) -> Result<(), String> {
        let path = self.path(&run.id)?;
        let directory = self.directory();
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let temporary = directory.join(format!("{}.tmp", uuid::Uuid::now_v7()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(&serde_json::to_vec_pretty(run).map_err(|e| e.to_string())?)
                .and_then(|()| file.sync_all())
                .map_err(|e| e.to_string())?;
            if create {
                fs::hard_link(&temporary, &path)
            } else {
                fs::rename(&temporary, &path)
            }
            .map_err(|e| e.to_string())?;
            fs::File::open(&directory)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())
        })();
        let _ = fs::remove_file(&temporary);
        result
    }
}
struct ActionLease {
    _lock: super::locks::RepositoryLock,
}
impl RunActionLease for ActionLease {}
impl RunRepository for FilesystemWorkRepository {
    fn acquire_action_lease(&self, id: &str) -> Result<Box<dyn RunActionLease>, String> {
        self.path(id)?;
        let lock = super::locks::try_lock(self.directory().join("actions").join(id))
            .map_err(|()| "Work action is already in progress".to_owned())?;
        Ok(Box::new(ActionLease { _lock: lock }))
    }
    fn create(&self, run: &PipelineRun) -> Result<(), String> {
        self.persist(run, true)
    }
    fn save(&self, run: &PipelineRun) -> Result<(), String> {
        self.persist(run, false)
    }
    fn load(&self, id: &str) -> Result<PipelineRun, String> {
        serde_json::from_slice(&fs::read(self.path(id)?).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    fn list(&self) -> Result<Vec<PipelineRun>, String> {
        let entries = match fs::read_dir(self.directory()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.to_string()),
        };
        let mut runs: Vec<PipelineRun> = vec![];
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                runs.push(
                    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?,
                );
            }
        }
        runs.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(runs)
    }
}
