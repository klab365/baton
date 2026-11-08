use baton_core::{
    domain::tasks::Task,
    ports::tasks::{TaskOperationLease, TaskRepository},
};
use std::{fs, path::PathBuf};

pub struct FilesystemTaskRepository(pub PathBuf);
impl FilesystemTaskRepository {
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err("Invalid task ID".into());
        }
        Ok(self.0.join(".baton/tasks").join(format!("{id}.json")))
    }
}
struct FileTaskOperationLease {
    _lock: super::locks::RepositoryLock,
}
impl TaskOperationLease for FileTaskOperationLease {}

impl TaskRepository for FilesystemTaskRepository {
    fn acquire_operation_lease(&self, id: &str) -> Result<Box<dyn TaskOperationLease>, String> {
        self.path(id)?;
        let lock = super::locks::try_lock(self.0.join(".baton/locks/tasks").join(id))
            .map_err(|()| "Task already has an active run or mutation".to_owned())?;
        Ok(Box::new(FileTaskOperationLease { _lock: lock }))
    }

    fn load(&self, id: &str) -> Result<Task, String> {
        serde_json::from_slice(&fs::read(self.path(id)?).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    fn list(&self) -> Result<Vec<Task>, String> {
        let directory = self.0.join(".baton/tasks");
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        let mut tasks = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("json")
            {
                let path = entry.path();
                let id = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .ok_or("Invalid task filename")?;
                tasks.push(self.load(id)?);
            }
        }
        Ok(tasks)
    }
    fn save(&self, task: &Task) -> Result<(), String> {
        let path = self.path(&task.id)?;
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temporary = path.with_extension("tmp");
        let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec_pretty(task).map_err(|e| e.to_string())?)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(temporary, path).map_err(|e| e.to_string())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        fs::remove_file(self.path(id)?).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_run_lease_excludes_concurrent_operations_and_releases_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let repository = FilesystemTaskRepository(directory.path().to_path_buf());
        let lease = repository.acquire_operation_lease("task-1").unwrap();

        assert!(repository.acquire_operation_lease("task-1").is_err());
        drop(lease);
        assert!(repository.acquire_operation_lease("task-1").is_ok());
    }
}
