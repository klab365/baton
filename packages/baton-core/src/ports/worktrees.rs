use crate::domain::worktrees::RunWorktree;

/// Execution workspace boundary; adapters may use local Git or another backend.
pub trait WorktreeManager: Send {
    fn allocate(&self, owner: &str) -> Result<RunWorktree, String>;
    fn cleanup(&self, owner: &str, workspace: &mut RunWorktree) -> Result<(), String>;
    fn validate(&self, _owner: &str, _workspace: &RunWorktree) -> Result<(), String> {
        Err("Workspace ownership validation is not supported".into())
    }
}
