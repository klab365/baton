use crate::domain::work::CheckResult;
use std::path::Path;

/// Executes explicitly configured process gates, never model-provided evidence.
pub trait WorkCheckRunner: Send + Sync {
    fn run(&self, workspace: &Path, command: &str) -> Result<CheckResult, String>;
}
