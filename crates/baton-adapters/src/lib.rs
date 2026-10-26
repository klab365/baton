//! Outbound adapters for Baton's domain ports.
mod profiles;
pub use profiles::ProfileRuntime;
mod fake;
pub mod filesystem_project_config_repository;
pub mod filesystem_run_repository;
pub mod filesystem_task_repository;
pub use fake::FakeRuntime;

mod git_worktree;
pub use git_worktree::GitWorktreeManager;
