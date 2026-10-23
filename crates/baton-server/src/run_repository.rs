use baton_core::orchestration::{PipelineRun, RunRepository};
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
        // Exclusive temporary file also prevents concurrent writers for the same run.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let bytes = serde_json::to_vec_pretty(run).map_err(|e| e.to_string())?;
            file.write_all(&bytes)
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
}
impl RunRepository for FilesystemRunRepository {
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
}
