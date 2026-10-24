use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunWorktree {
    pub owner: String,
    pub path: PathBuf,
    pub branch: String,
    pub base_revision: String,
    pub lifecycle: String,
    pub cleanup_outcome: String,
}

/// Execution workspace boundary; future execution targets need not use local Git.
pub trait WorktreeManager {
    fn allocate(&self, run_id: &str) -> Result<RunWorktree, String>;
    fn cleanup(&self, run_id: &str, workspace: &mut RunWorktree) -> Result<(), String>;
}
