use baton_core::tasks::{Task, TaskRepository};
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
impl TaskRepository for FilesystemTaskRepository {
    fn load(&self, id: &str) -> Result<Task, String> {
        serde_json::from_slice(&fs::read(self.path(id)?).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
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
