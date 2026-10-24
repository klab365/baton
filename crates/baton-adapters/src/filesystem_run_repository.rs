use baton_core::orchestration::{PipelineRun, RunEvent, RunRepository, Status};
use std::{fs, io::Write, path::PathBuf};

pub struct FilesystemRunRepository(pub PathBuf);
impl FilesystemRunRepository {
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err("Run ID must contain only ASCII letters, digits or hyphens".into());
        }
        Ok(self.0.join(".baton/runs").join(format!("{id}.json")))
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
                run.ended_at = Some(baton_core::orchestration::utc_now());
                run.duration_ms = None;
                let event = RunEvent {
                    run_id: run.id.clone(),
                    status: Status::Failed,
                    occurred_at: baton_core::orchestration::utc_now(),
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
impl RunRepository for FilesystemRunRepository {
    fn worktree_manager(&self) -> Result<Box<dyn baton_core::worktrees::WorktreeManager>, String> {
        Ok(Box::new(crate::GitWorktreeManager {
            repository: self.0.clone(),
            base_revision: std::env::var("BATON_BASE_REVISION").unwrap_or_else(|_| "HEAD".into()),
        }))
    }
    fn create(&self, run: &PipelineRun) -> Result<(), String> {
        self.persist(run, true)
    }
    fn save(&self, run: &PipelineRun) -> Result<(), String> {
        self.persist(run, false)
    }
    fn list(&self) -> Result<Vec<PipelineRun>, String> {
        FilesystemRunRepository::list(self)
    }
    fn load(&self, id: &str) -> Result<PipelineRun, String> {
        serde_json::from_slice(&fs::read(self.path(id)?).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    fn publish(&self, event: &RunEvent) -> Result<(), String> {
        self.path(&event.run_id)?;
        let directory = self.0.join(".baton/runs");
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let mut run = self.load(&event.run_id)?;
        if run.events.last() != Some(event) {
            run.events.push(event.clone());
        }
        self.save(&run)
    }
}
