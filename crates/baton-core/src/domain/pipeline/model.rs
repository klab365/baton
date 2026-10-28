use crate::domain::{project::ProjectConfig, worktrees::RunWorktree};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    WaitingHuman,
    LimitExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StepRun {
    pub step_id: String,
    pub agent_id: String,
    pub status: Status,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub outcome: Option<crate::domain::project::Outcome>,
    #[serde(default)]
    pub selected_transition: Option<crate::domain::project::Transition>,
    #[serde(default)]
    pub attempt: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PipelineRun {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default)]
    pub worktree: Option<RunWorktree>,
    #[serde(default)]
    pub setup_error: Option<String>,
    pub pipeline_id: String,
    pub task: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub events: Vec<RunEvent>,
    pub status: Status,
    pub steps: Vec<StepRun>,
    #[serde(
        default,
        deserialize_with = "crate::domain::project::deserialize_optional_snapshot"
    )]
    pub config_snapshot: Option<ProjectConfig>,
    #[serde(default)]
    pub current_step: Option<String>,
    #[serde(default)]
    pub handoff: Option<String>,
    #[serde(default)]
    pub pending_actions: Vec<crate::domain::project::Outcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub run_id: String,
    pub status: Status,
    pub occurred_at: String,
    pub message: Option<String>,
}

/// Persistence and worktree ports required by the pipeline engine.
#[derive(Debug, Clone, Default)]
pub struct TaskRunContext {
    pub task_id: Option<String>,
    pub worktree: Option<RunWorktree>,
}
