use baton_core::worktrees::{RunWorktree, WorktreeManager};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub struct GitWorktreeManager {
    pub repository: PathBuf,
    pub base_revision: String,
}
impl GitWorktreeManager {
    fn git(&self, args: &[&std::ffi::OsStr]) -> Result<String, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repository)
            .args(args)
            .output()
            .map_err(|e| format!("Cannot launch Git: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "Git worktree operation failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    }
    fn location(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err("Invalid worktree owner ID".into());
        }
        let common = self.git(&[
            "rev-parse".as_ref(),
            "--path-format=absolute".as_ref(),
            "--git-common-dir".as_ref(),
        ])?;
        Ok(Path::new(&common).join("baton-worktrees").join(id))
    }
}
impl WorktreeManager for GitWorktreeManager {
    fn allocate(&self, id: &str) -> Result<RunWorktree, String> {
        let path = self.location(id).map_err(|e| {
            format!("Cannot allocate isolated workspace. Configure a Git project repository: {e}")
        })?;
        let base = self.git(&[
            "rev-parse".as_ref(),
            "--verify".as_ref(),
            "--end-of-options".as_ref(),
            format!("{}^{{commit}}", self.base_revision).as_ref(),
        ])?;
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // Exclusive reservation prevents concurrent allocation or reuse, including stale runs.
        let reservation = parent.join(format!("{id}.reservation"));
        use std::io::Write;
        let mut owner = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&reservation)
            .map_err(|e| format!("Workspace ownership collision for {id}: {e}"))?;
        owner
            .write_all(id.as_bytes())
            .and_then(|()| owner.sync_all())
            .map_err(|e| e.to_string())?;
        let branch = format!("baton/run-{id}");
        self.git(&[
            "worktree".as_ref(),
            "add".as_ref(),
            "-b".as_ref(),
            branch.as_ref(),
            path.as_os_str(),
            base.as_ref(),
        ]).map_err(|e| format!("Cannot allocate {} on {branch}: {e}. Reservation retained; inspect before retrying with a new run ID.", path.display()))?;
        // A reservation alone does not establish ownership: Git must have created
        // the worktree successfully before a cleanup receipt is issued.
        let mut receipt = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(parent.join(format!("{id}.owner")))
            .map_err(|e| e.to_string())?;
        receipt
            .write_all(id.as_bytes())
            .and_then(|()| receipt.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(RunWorktree {
            owner: id.into(),
            path,
            branch,
            base_revision: base,
            lifecycle: "retained".into(),
            cleanup_outcome: "Not requested; all runs are retained for inspection".into(),
        })
    }
    fn cleanup(&self, id: &str, workspace: &mut RunWorktree) -> Result<(), String> {
        let result = (|| {
            let expected = self.location(id)?;
            let marker = expected.parent().unwrap().join(format!("{id}.owner"));
            if workspace.owner != id
                || workspace.path != expected
                || workspace.branch != format!("baton/run-{id}")
                || fs::read_to_string(&marker).map_err(|e| e.to_string())? != id
            {
                return Err("Refusing cleanup: workspace is not owned by this run".into());
            }
            let actual = Command::new("git")
                .arg("-C")
                .arg(&expected)
                .args(["symbolic-ref", "--short", "HEAD"])
                .output()
                .map_err(|e| e.to_string())?;
            if !actual.status.success()
                || String::from_utf8_lossy(&actual.stdout).trim() != workspace.branch
            {
                return Err("Refusing cleanup: worktree branch identity changed".into());
            }
            self.git(&["worktree".as_ref(), "remove".as_ref(), expected.as_os_str()])?;
            fs::remove_file(marker).map_err(|e| e.to_string())?;
            workspace.lifecycle = "removed".into();
            Ok(())
        })();
        workspace.cleanup_outcome = match &result {
            Ok(()) => "Removed explicitly".into(),
            Err(e) => format!("Cleanup refused or failed: {e}"),
        };
        result
    }
}
