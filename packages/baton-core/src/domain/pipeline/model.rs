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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_handoff: Option<super::StructuredHandoff>,
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
    /// Currency code for the estimated cost (for example, USD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Price denominator, such as `perMillionTokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_unit: Option<String>,
    /// Provider-reported or estimated usage coverage; absent on legacy records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<String>,
    /// Rates captured when the estimate was computed, retained for provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_snapshot: Option<serde_json::Value>,
    /// Per-million-token rates used for each independently billable category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_price: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_price: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_price: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_price: Option<String>,
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
pub struct StructuredHandoffAttempt {
    pub attempt: u32,
    pub step_id: String,
    pub report: super::StructuredHandoff,
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
    use super::{ModelUsage, PipelineRun};

    #[test]
    fn legacy_run_without_trace_deserializes_with_an_empty_trace() {
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
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
        run.steps[0].usage = Some(ModelUsage {
            input_tokens: Some(100),
            output_tokens: Some(20),
            cache_read_tokens: Some(10),
            cache_write_tokens: Some(5),
            estimated_cost: Some("0.0003".into()),
            price_source: Some("models.dev".into()),
            price_checked_at: Some("2026-04-01T00:00:00Z".into()),
            currency: Some("USD".into()),
            price_unit: Some("perMillionTokens".into()),
            coverage: Some("complete".into()),
            price_snapshot: Some(serde_json::json!({"input":"2","output":"8"})),
            input_price: Some("2".into()),
            output_price: Some("8".into()),
            cache_read_price: Some("0.2".into()),
            cache_write_price: Some("1".into()),
        });
        let saved = serde_json::to_vec(&run).unwrap();
        let reloaded: PipelineRun = serde_json::from_slice(&saved).unwrap();
        assert_eq!(reloaded.steps[0].usage, run.steps[0].usage);
    }

    #[test]
    fn legacy_usage_keeps_unknown_pricing_unknown_and_new_provenance_roundtrips() {
        let legacy: ModelUsage = serde_json::from_value(serde_json::json!({
            "inputTokens": 12, "outputTokens": 3, "cacheReadTokens": null,
            "cacheWriteTokens": null, "estimatedCost": null,
            "priceSource": null, "priceCheckedAt": null
        }))
        .unwrap();
        assert_eq!(legacy.currency, None);
        assert_eq!(legacy.coverage, None);
        let legacy_json = serde_json::to_value(&legacy).unwrap();
        assert!(legacy_json.get("currency").is_none());
        assert!(legacy_json.get("priceSnapshot").is_none());
        assert!(legacy_json.get("inputPrice").is_none());
        assert!(legacy_json.get("outputPrice").is_none());
        assert!(legacy_json.get("cacheReadPrice").is_none());
        assert!(legacy_json.get("cacheWritePrice").is_none());

        let usage = ModelUsage {
            input_tokens: Some(12),
            output_tokens: Some(3),
            cache_read_tokens: None,
            cache_write_tokens: None,
            estimated_cost: Some("0.0004".into()),
            price_source: Some("models.dev".into()),
            price_checked_at: Some("2026-01-01T00:00:00Z".into()),
            currency: Some("USD".into()),
            price_unit: Some("perMillionTokens".into()),
            coverage: Some("complete".into()),
            price_snapshot: Some(serde_json::json!({"input": 1.0, "output": 2.0})),
            input_price: Some("1.0".into()),
            output_price: Some("2.0".into()),
            cache_read_price: None,
            cache_write_price: None,
        };
        let roundtrip: ModelUsage =
            serde_json::from_value(serde_json::to_value(&usage).unwrap()).unwrap();
        assert_eq!(roundtrip, usage);
    }
}
