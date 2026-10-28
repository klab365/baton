use baton_core::{
    domain::pipeline::{PipelineRun, RunEvent, Status},
    features::pipeline_engine::utc_now,
    ports::{
        runs::{RunActionLease, RunRepository},
        worktrees::WorktreeManager,
    },
};
use std::{fs, io::Write, path::PathBuf};

pub struct FilesystemRunRepository(pub PathBuf);
impl FilesystemRunRepository {
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err("Run ID must contain only ASCII letters, digits or hyphens".into());
        }
        Ok(self.0.join(".baton/runs").join(format!("{id}.json")))
    }
    fn repository_lock(&self) -> super::locks::RepositoryLock {
        super::locks::lock(self.0.join(".baton/locks/runs"))
    }
    fn persist(&self, run: &PipelineRun, create: bool) -> Result<(), String> {
        let path = self.path(&run.id)?;
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temporary = parent.join(format!("{}.tmp", run.id));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
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
            fs::File::open(parent)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())
        })();
        let _ = fs::remove_file(temporary);
        result
    }
    pub fn list(&self) -> Result<Vec<PipelineRun>, String> {
        let _lock = self.repository_lock();
        self.list_unlocked()
    }

    fn list_unlocked(&self) -> Result<Vec<PipelineRun>, String> {
        let directory = self.0.join(".baton/runs");
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.to_string()),
        };
        let mut runs: Vec<PipelineRun> = vec![];
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|x| x.to_str()) == Some("json") {
                let mut run: PipelineRun =
                    serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                let legacy = path.with_file_name(format!("{}.events.jsonl", run.id));
                if let Ok(contents) = fs::read_to_string(&legacy) {
                    for line in contents.lines() {
                        let event: RunEvent =
                            serde_json::from_str(line).map_err(|e| e.to_string())?;
                        if !run.events.contains(&event) {
                            run.events.push(event);
                        }
                    }
                    self.persist(&run, false)?;
                    fs::remove_file(legacy).map_err(|e| e.to_string())?;
                }
                runs.push(run);
            }
        }
        runs.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(runs)
    }
    pub fn recover_interrupted(&self) -> Result<(), String> {
        for mut run in self.list()? {
            if run.status == Status::Running {
                run.status = Status::Failed;
                run.ended_at = Some(utc_now());
                run.duration_ms = None;
                let event = RunEvent {
                    run_id: run.id.clone(),
                    status: Status::Failed,
                    occurred_at: utc_now(),
                    message: Some("Run interrupted by server restart".into()),
                };
                run.events.push(event.clone());
                self.save(&run)?;
                self.publish(&event)?;
            }
        }
        Ok(())
    }
}
struct FileRunActionLease {
    _lock: super::locks::RepositoryLock,
}
impl RunActionLease for FileRunActionLease {}

impl RunRepository for FilesystemRunRepository {
    fn acquire_action_lease(&self, id: &str) -> Result<Box<dyn RunActionLease>, String> {
        self.path(id)?;
        let lock = super::locks::try_lock(self.0.join(".baton/locks/run-actions").join(id))
            .map_err(|()| "Run action is already in progress".to_owned())?;
        Ok(Box::new(FileRunActionLease { _lock: lock }))
    }

    fn worktree_manager(&self) -> Result<Box<dyn WorktreeManager>, String> {
        Ok(Box::new(crate::worktrees::GitWorktreeManager {
            repository: self.0.clone(),
            base_revision: std::env::var("BATON_BASE_REVISION").unwrap_or_else(|_| "HEAD".into()),
        }))
    }
    fn create(&self, run: &PipelineRun) -> Result<(), String> {
        let _lock = self.repository_lock();
        self.persist(run, true)
    }
    fn save(&self, run: &PipelineRun) -> Result<(), String> {
        let _lock = self.repository_lock();
        if let Ok(existing) = fs::read(self.path(&run.id)?)
            && let Ok(existing) = serde_json::from_slice::<PipelineRun>(&existing)
            && existing.status == Status::Cancelled
            && run.status != Status::Cancelled
        {
            return Ok(());
        }
        self.persist(run, false)
    }
    fn list(&self) -> Result<Vec<PipelineRun>, String> {
        FilesystemRunRepository::list(self)
    }
    fn load(&self, id: &str) -> Result<PipelineRun, String> {
        let _lock = self.repository_lock();
        serde_json::from_slice(&fs::read(self.path(id)?).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    fn publish(&self, event: &RunEvent) -> Result<(), String> {
        let _lock = self.repository_lock();
        let path = self.path(&event.run_id)?;
        let mut run: PipelineRun =
            serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        if run.events.last() != Some(event) {
            run.events.push(event.clone());
            self.persist(&run, false)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_action_lease_excludes_duplicate_actions_and_releases_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let repository = FilesystemRunRepository(directory.path().to_path_buf());
        let lease = repository.acquire_action_lease("run-1").unwrap();

        assert!(repository.acquire_action_lease("run-1").is_err());
        drop(lease);
        assert!(repository.acquire_action_lease("run-1").is_ok());
    }

    #[test]
    fn stale_run_saves_cannot_overwrite_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let repository = FilesystemRunRepository(directory.path().to_path_buf());
        let cancelled = PipelineRun {
            id: "run-1".into(),
            task_id: None,
            worktree: None,
            setup_error: None,
            pipeline_id: "pipeline".into(),
            task: "task".into(),
            started_at: "2026-01-01T00:00:00Z".into(),
            ended_at: Some("2026-01-01T00:00:01Z".into()),
            duration_ms: Some(1),
            events: vec![],
            trace: vec![],
            status: Status::Cancelled,
            steps: vec![],
            config_snapshot: None,
            current_step: None,
            handoff: None,
            pending_actions: vec![],
        };
        repository.create(&cancelled).unwrap();
        let mut stale = cancelled.clone();
        stale.status = Status::Running;
        stale.ended_at = None;

        repository.save(&stale).unwrap();

        assert_eq!(repository.load("run-1").unwrap().status, Status::Cancelled);
    }
}
