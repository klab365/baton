use crate::{
    domain::pipeline::{PipelineRun, RunEvent},
    ports::worktrees::WorktreeManager,
};

pub trait RunActionLease: Send {}
struct UnlockedRunActionLease;
impl RunActionLease for UnlockedRunActionLease {}

/// Persistence boundary required by pipeline execution and run lifecycle features.
pub trait RunRepository: Send + Sync {
    /// Reserve a human action for a run. Cancellation intentionally does not take this lease.
    fn acquire_action_lease(&self, _id: &str) -> Result<Box<dyn RunActionLease>, String> {
        Ok(Box::new(UnlockedRunActionLease))
    }
    fn worktree_manager(&self) -> Result<Box<dyn WorktreeManager>, String> {
        Err("Project worktree manager is not configured".into())
    }
    fn create(&self, run: &PipelineRun) -> Result<(), String>;
    fn save(&self, run: &PipelineRun) -> Result<(), String>;
    fn load(&self, id: &str) -> Result<PipelineRun, String>;
    fn list(&self) -> Result<Vec<PipelineRun>, String>;
    /// Durable publication point for future live-update consumers.
    fn publish(&self, _event: &RunEvent) -> Result<(), String> {
        Ok(())
    }
}
