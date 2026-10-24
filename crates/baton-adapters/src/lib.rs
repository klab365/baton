//! Outbound adapters for Baton's domain ports.
mod chatgpt;
mod chatgpt_provider;
mod fake;
pub mod filesystem_project_config_repository;
pub mod filesystem_run_repository;
pub mod filesystem_task_repository;
mod openai;
mod rig_support;

pub use chatgpt::ChatGptRigRuntime;
pub use chatgpt_provider::{ChatGptSession, DeviceCode};
pub use fake::FakeRuntime;
pub use openai::OpenAiRigRuntime;

mod git_worktree;
pub use git_worktree::GitWorktreeManager;
