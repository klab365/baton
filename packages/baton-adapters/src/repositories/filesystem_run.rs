use baton_core::{
    domain::pipeline::{PipelineRun, RunEvent, Status},
    features::pipeline_engine::utc_now,
    ports::{
        runs::{RunActionLease, RunRepository},
        worktrees::WorktreeManager,
    },
};
use std::{fs, io::Write, path::PathBuf};

// Keep the runtime and HTTP representation as strings while storing JSON transcripts
// structurally. Plain text and legacy string contents remain unchanged.
fn serialize_run(run: &PipelineRun) -> Result<Vec<u8>, serde_json::Error> {
    let mut value = serde_json::to_value(run)?;
    if let Some(trace) = value.get_mut("trace").and_then(|v| v.as_array_mut()) {
        for entry in trace {
            if let Some(content) = entry.get_mut("content")
                && let Some(text) = content.as_str()
                && let Ok(transcript) = serde_json::from_str::<serde_json::Value>(text)
                && (transcript.is_array() || transcript.is_object())
            {
                *content = serde_json::json!({ "json": transcript });
            }
        }
    }
    serde_json::to_vec_pretty(&value)
}

fn deserialize_run(bytes: &[u8]) -> Result<PipelineRun, serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
    if let Some(trace) = value.get_mut("trace").and_then(|v| v.as_array_mut()) {
        for entry in trace {
            if let Some(content) = entry.get_mut("content")
                && let Some(object) = content.as_object()
                && object.len() == 1
                && let Some(transcript) = object.get("json")
                && (transcript.is_array() || transcript.is_object())
            {
                *content = serde_json::Value::String(serde_json::to_string_pretty(transcript)?);
            }
        }
    }
    serde_json::from_value(value)
}

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
        let temporary = parent.join(format!("{}.{}.tmp", run.id, uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(&serialize_run(run).map_err(|e| e.to_string())?)
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
                    deserialize_run(&fs::read(&path).map_err(|e| e.to_string())?)
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
    fn prepare_run_memory(&self, run: &PipelineRun) -> Result<Option<String>, String> {
        run.worktree
            .as_ref()
            .map(|worktree| super::run_memory::prepare(&worktree.path, &run.id, &run.task))
            .transpose()
    }

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
            && let Ok(existing) = deserialize_run(&existing)
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
        deserialize_run(&fs::read(self.path(id)?).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    fn publish(&self, event: &RunEvent) -> Result<(), String> {
        let _lock = self.repository_lock();
        let path = self.path(&event.run_id)?;
        let mut run: PipelineRun =
            deserialize_run(&fs::read(path).map_err(|error| error.to_string())?)
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
    fn trace_files_are_structured_and_legacy_contents_remain_readable() {
        use baton_core::domain::pipeline::RunTraceEntry;

        let directory = tempfile::tempdir().unwrap();
        let repository = FilesystemRunRepository(directory.path().to_path_buf());
        let legacy = serde_json::json!({
            "id": "run-1", "pipelineId": "pipeline", "task": "task",
            "startedAt": "2026-01-01T00:00:00Z", "endedAt": null,
            "durationMs": null, "status": "succeeded", "steps": [],
            "trace": [
                {"occurredAt": "now", "kind": "runtimeTranscript", "content": "[{\"role\":\"assistant\"}]"},
                {"occurredAt": "now", "kind": "text", "content": "not JSON"}
            ]
        });
        let run = deserialize_run(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        repository.create(&run).unwrap();
        let path = directory.path().join(".baton/runs/run-1.json");
        let stored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            stored["trace"][0]["content"]["json"][0]["role"],
            "assistant"
        );
        assert_eq!(stored["trace"][1]["content"], "not JSON");
        let loaded = repository.load("run-1").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&loaded.trace[0].content).unwrap(),
            serde_json::json!([{"role": "assistant"}])
        );
        assert_eq!(
            loaded.trace[1],
            RunTraceEntry {
                attempt: None,
                occurred_at: "now".into(),
                kind: "text".into(),
                content: "not JSON".into()
            }
        );
        assert_eq!(repository.list().unwrap(), vec![loaded.clone()]);
        let mut assigned = loaded;
        assigned.trace[0].attempt = Some(3);
        repository.save(&assigned).unwrap();
        assert_eq!(repository.load("run-1").unwrap(), assigned);
    }

    #[test]
    fn orphaned_temporary_file_does_not_block_writes_or_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let repository = FilesystemRunRepository(directory.path().to_path_buf());
        let runs_dir = directory.path().join(".baton/runs");
        fs::create_dir_all(&runs_dir).unwrap();
        let orphan = runs_dir.join("run-1.tmp");
        fs::write(&orphan, b"orphan").unwrap();
        let value = serde_json::json!({
            "id": "run-1", "pipelineId": "pipeline", "task": "task",
            "startedAt": "2026-01-01T00:00:00Z", "status": "running", "steps": []
        });
        let run = deserialize_run(&serde_json::to_vec(&value).unwrap()).unwrap();

        repository.create(&run).unwrap();
        repository.save(&run).unwrap();
        repository.recover_interrupted().unwrap();

        let recovered = repository.load("run-1").unwrap();
        assert_eq!(recovered.status, Status::Failed);
        assert_eq!(
            recovered.events.last().unwrap().message.as_deref(),
            Some("Run interrupted by server restart")
        );
        assert_eq!(fs::read(orphan).unwrap(), b"orphan");
        assert!(repository.create(&run).is_err());
    }

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
            session: None,
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
