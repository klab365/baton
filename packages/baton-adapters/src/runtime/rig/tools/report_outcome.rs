use baton_core::{domain::project::Outcome, ports::runtime::AgentDecision};
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

#[derive(Deserialize, JsonSchema)]
pub(crate) struct ReportOutcomeArgs {
    outcome: String,
    summary: String,
    /// For blocked outcomes, explain why work cannot continue; otherwise omit it or use an empty string.
    #[serde(default)]
    blocker_reason: String,
    /// Optional actionable human request for blocked outcomes; otherwise use an empty string.
    #[serde(default)]
    human_action: String,
    /// Optional versioned structured handoff; summary remains the legacy text handoff.
    #[serde(default)]
    structured_handoff: Option<serde_json::Value>,
}

pub(crate) fn outcome_name(outcome: &Outcome) -> String {
    serde_json::to_value(outcome)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[derive(Clone)]
pub(crate) struct ReportOutcome {
    allowed: Vec<Outcome>,
    decision: Arc<Mutex<Option<AgentDecision>>>,
}
impl ReportOutcome {
    pub(crate) fn new(allowed: Vec<Outcome>, decision: Arc<Mutex<Option<AgentDecision>>>) -> Self {
        Self {
            allowed: allowed
                .into_iter()
                .filter(|outcome| *outcome != Outcome::Progress)
                .collect(),
            decision,
        }
    }
}

impl Tool for ReportOutcome {
    const NAME: &'static str = "report_outcome";
    type Args = ReportOutcomeArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        let allowed = self
            .allowed
            .iter()
            .map(outcome_name)
            .collect::<Vec<_>>()
            .join(", ");
        if self.allowed.is_empty() {
            "Optional report_outcome tool for this sequential step; call it only when providing structured_handoff, and keep the final response as the legacy text handoff. Use outcome `completed`. Structured schema: {version: 1, summary: nonblank string, findings?: [{id: stable unique string, status: open|resolved|accepted, description: nonblank string}], verification?: [{description: nonblank string, result: nonblank string}], nextAction?: nonblank string}. Example: {\"version\":1,\"summary\":\"Ready\",\"findings\":[{\"id\":\"F-1\",\"status\":\"open\",\"description\":\"Add coverage\"}]}. Maximum 128 findings and 16384 JSON bytes. Agent verification is an assertion, not independent verification.".into()
        } else {
            format!(
                "Required control tool for this branching pipeline step. Call once after finishing your work. Allowed outcomes: {allowed}. For blocked outcomes, include a blocker_reason; human_action is optional. Include the complete authoritative handoff for the next step in summary. Optional structured_handoff schema: {{version: 1, summary: nonblank string, findings?: [{{id: stable unique string, status: open|resolved|accepted, description: nonblank string}}], verification?: [{{description: nonblank string, result: nonblank string}}], nextAction?: nonblank string}}. Example: {{\"version\":1,\"summary\":\"Ready\",\"findings\":[{{\"id\":\"F-1\",\"status\":\"open\",\"description\":\"Add coverage\"}}]}}. Maximum 128 findings and 16384 JSON bytes. Agent verification is an assertion, not independent verification."
            )
        }
    }

    fn parameters(&self) -> serde_json::Value {
        let mut schema = rig::__core::tool::args_schema::<ReportOutcomeArgs>();
        let choices = if self.allowed.is_empty() {
            vec![Outcome::Completed]
        } else {
            self.allowed.clone()
        };
        schema["properties"]["outcome"]["enum"] = serde_json::Value::Array(
            choices
                .iter()
                .map(outcome_name)
                .map(serde_json::Value::String)
                .collect(),
        );
        schema
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: ReportOutcomeArgs,
    ) -> Result<String, ToolExecutionError> {
        let value = serde_json::Value::String(args.outcome);
        let outcome: Outcome = serde_json::from_value(value)
            .map_err(|error| ToolExecutionError::invalid_args(error.to_string()))?;
        if !self.allowed.contains(&outcome)
            && !(self.allowed.is_empty() && outcome == Outcome::Completed)
        {
            return Err(ToolExecutionError::invalid_args(
                "Outcome is not configured for this pipeline step",
            ));
        }
        let structured_handoff = args
            .structured_handoff
            .map(|value| {
                baton_core::domain::pipeline::StructuredHandoff::parse(
                    &serde_json::to_vec(&value)
                        .map_err(|error| ToolExecutionError::invalid_args(error.to_string()))?,
                )
                .map_err(ToolExecutionError::invalid_args)
            })
            .transpose()?;
        let mut summary = args.summary.trim().to_owned();
        if summary.is_empty() {
            return Err(ToolExecutionError::invalid_args(
                "Summary must not be empty",
            ));
        }
        if outcome == Outcome::Blocked {
            let reason = args.blocker_reason.trim();
            let action = args.human_action.trim();
            if reason.is_empty() {
                return Err(ToolExecutionError::invalid_args(
                    "Blocked outcomes require a blocker reason",
                ));
            }
            summary.push_str(&format!("\n\nBlocker: {reason}"));
            if !action.is_empty() {
                summary.push_str(&format!("\nHuman action needed: {action}"));
            }
        }
        if summary.len() > baton_core::ports::runtime::MAX_HANDOFF_BYTES {
            return Err(ToolExecutionError::invalid_args(
                "Summary must contain 1 to 16384 bytes",
            ));
        }
        let mut decision = self
            .decision
            .lock()
            .map_err(|_| ToolExecutionError::other("Outcome state is unavailable"))?;
        if decision.is_some() {
            return Err(ToolExecutionError::invalid_args(
                "An outcome has already been reported for this step",
            ));
        }
        *decision = Some(AgentDecision {
            outcome,
            summary,
            structured_handoff,
        });
        Ok("Pipeline outcome recorded".into())
    }
}

#[cfg(test)]
mod tests {
    use super::ReportOutcome;
    use baton_core::{domain::project::Outcome, ports::runtime::AgentDecision};
    use rig::tool::{Tool, ToolContext};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn blocked_requires_reason_and_accepts_optional_human_action() {
        let decision = Arc::new(Mutex::new(None));
        let tool = ReportOutcome::new(vec![Outcome::Blocked], decision.clone());
        let schema = tool.parameters();
        let description = tool.description();
        assert!(description.contains("status: open|resolved|accepted"));
        assert!(description.contains("verification is an assertion"));
        assert!(
            !schema["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("blocker_reason"))
        );
        assert!(
            !schema["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("human_action"))
        );
        let mut context = ToolContext::default();
        assert!(
            tool.call(
                &mut context,
                super::ReportOutcomeArgs {
                    outcome: "blocked".into(),
                    summary: "Cannot continue".into(),
                    blocker_reason: "  ".into(),
                    human_action: String::new(),
                    structured_handoff: None,
                }
            )
            .await
            .is_err()
        );
        tool.call(
            &mut context,
            super::ReportOutcomeArgs {
                outcome: "blocked".into(),
                summary: "Cannot continue".into(),
                blocker_reason: "Missing credentials".into(),
                human_action: String::new(),
                structured_handoff: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            decision.lock().unwrap().as_ref().unwrap().summary,
            "Cannot continue\n\nBlocker: Missing credentials"
        );

        let decision = Arc::new(Mutex::new(None));
        let tool = ReportOutcome::new(vec![Outcome::Blocked], decision.clone());
        tool.call(
            &mut context,
            super::ReportOutcomeArgs {
                outcome: "blocked".into(),
                summary: "Cannot continue".into(),
                blocker_reason: "Missing credentials".into(),
                human_action: "Provide credentials".into(),
                structured_handoff: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            decision.lock().unwrap().as_ref().unwrap().summary,
            "Cannot continue\n\nBlocker: Missing credentials\nHuman action needed: Provide credentials"
        );
    }

    #[test]
    fn blocker_reason_can_be_omitted_from_non_blocked_calls() {
        let args: super::ReportOutcomeArgs = serde_json::from_value(serde_json::json!({
            "outcome": "approved",
            "summary": "Looks good"
        }))
        .unwrap();
        assert_eq!(args.blocker_reason, "");
    }

    #[tokio::test]
    async fn report_outcome_rejects_legacy_progress() {
        let decision = Arc::new(Mutex::new(None));
        let tool = ReportOutcome::new(
            vec![Outcome::Progress, Outcome::Completed],
            decision.clone(),
        );
        assert_eq!(
            tool.parameters()["properties"]["outcome"]["enum"],
            serde_json::json!(["completed"])
        );
        assert!(
            tool.call(
                &mut ToolContext::default(),
                super::ReportOutcomeArgs {
                    outcome: "progress".into(),
                    summary: "Partial work".into(),
                    blocker_reason: String::new(),
                    human_action: String::new(),
                    structured_handoff: None,
                },
            )
            .await
            .is_err()
        );
        assert!(decision.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn sequential_report_outcome_is_optional_and_accepts_only_completed() {
        let decision = Arc::new(Mutex::new(None));
        let tool = ReportOutcome::new(vec![], decision.clone());
        assert!(tool.description().contains("Optional report_outcome"));
        assert_eq!(
            tool.parameters()["properties"]["outcome"]["enum"],
            serde_json::json!(["completed"])
        );
        let report = serde_json::json!({"version":1,"summary":"Ready"});
        tool.call(
            &mut ToolContext::default(),
            super::ReportOutcomeArgs {
                outcome: "completed".into(),
                summary: "Ready".into(),
                blocker_reason: String::new(),
                human_action: String::new(),
                structured_handoff: Some(report),
            },
        )
        .await
        .unwrap();
        assert!(
            decision
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .structured_handoff
                .is_some()
        );
        let unused = Arc::new(Mutex::new(None));
        let _optional_tool = ReportOutcome::new(vec![], unused.clone());
        assert!(unused.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn report_outcome_accepts_valid_structured_handoff_and_rejects_unknown_fields() {
        let decision = Arc::new(Mutex::new(None));
        let tool = ReportOutcome::new(vec![Outcome::Approved], decision.clone());
        let valid = serde_json::json!({
            "version": 1, "summary": "Ready", "findings": [], "verification": [],
            "nextAction": "Review"
        });
        tool.call(
            &mut ToolContext::default(),
            super::ReportOutcomeArgs {
                outcome: "approved".into(),
                summary: "Ready".into(),
                blocker_reason: String::new(),
                human_action: String::new(),
                structured_handoff: Some(valid),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            decision
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .structured_handoff
                .as_ref()
                .unwrap()
                .next_action
                .as_deref(),
            Some("Review")
        );

        let tool = ReportOutcome::new(vec![Outcome::Approved], Arc::new(Mutex::new(None)));
        let invalid = serde_json::json!({ "version": 1, "summary": "Ready", "summery": "typo" });
        assert!(
            tool.call(
                &mut ToolContext::default(),
                super::ReportOutcomeArgs {
                    outcome: "approved".into(),
                    summary: "Ready".into(),
                    blocker_reason: String::new(),
                    human_action: String::new(),
                    structured_handoff: Some(invalid),
                },
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("unknown field")
        );
    }

    #[tokio::test]
    async fn report_outcome_records_only_configured_choices() {
        let decision = Arc::new(Mutex::new(None));
        let tool = ReportOutcome::new(vec![Outcome::Approved], decision.clone());
        assert_eq!(
            tool.parameters()["properties"]["outcome"]["enum"],
            serde_json::json!(["approved"])
        );
        let mut context = ToolContext::default();
        tool.call(
            &mut context,
            super::ReportOutcomeArgs {
                outcome: "approved".into(),
                summary: "  Review passed.  ".into(),
                blocker_reason: String::new(),
                human_action: String::new(),
                structured_handoff: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            *decision.lock().unwrap(),
            Some(AgentDecision {
                outcome: Outcome::Approved,
                summary: "Review passed.".into(),
                structured_handoff: None,
            })
        );
        assert!(
            tool.call(
                &mut context,
                super::ReportOutcomeArgs {
                    outcome: "blocked".into(),
                    summary: "Not configured".into(),
                    blocker_reason: String::new(),
                    human_action: String::new(),
                    structured_handoff: None,
                },
            )
            .await
            .is_err()
        );
    }
}
