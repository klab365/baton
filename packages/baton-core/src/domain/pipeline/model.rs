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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<crate::domain::runtime::RuntimeModel>,
    /// Absent means provider usage was unavailable or not collected, never zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ModelUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub estimated_cost: Option<String>,
    pub price_source: Option<String>,
    pub price_checked_at: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<crate::domain::work::WorkSession>,
    pub pipeline_id: String,
    pub task: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub events: Vec<RunEvent>,
    /// Complete, persisted runtime transcript. Empty for runs recorded before tracing.
    #[serde(default)]
    pub trace: Vec<RunTraceEntry>,
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

/// A sanitized item emitted by an agent runtime while executing a run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunTraceEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    pub occurred_at: String,
    pub kind: String,
    pub content: String,
}

/// Persistence and worktree ports required by the pipeline engine.
#[derive(Debug, Clone, Default)]
pub struct TaskRunContext {
    pub task_id: Option<String>,
    pub worktree: Option<RunWorktree>,
}

#[cfg(test)]
mod tests {
    use super::PipelineRun;

    #[test]
    fn legacy_run_without_trace_deserializes_with_an_empty_trace() {
        let run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id": "run-1", "pipelineId": "pipeline", "task": "task",
            "startedAt": "2026-01-01T00:00:00Z", "endedAt": null,
            "durationMs": null, "events": [], "status": "running",
            "configSnapshot": null,
            "steps": [{ "stepId": "build", "agentId": "builder", "status": "succeeded", "input": null, "output": null, "error": null }]
        }))
        .unwrap();
        assert!(run.trace.is_empty());
        assert_eq!(run.steps[0].started_at, None);
        assert_eq!(run.steps[0].ended_at, None);
        assert_eq!(run.steps[0].duration_ms, None);
        assert_eq!(run.steps[0].model, None);
        assert_eq!(run.steps[0].usage, None);
    }
}
