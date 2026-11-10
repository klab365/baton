use baton_core::{domain::project::Outcome, ports::runtime::AgentDecision};
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

#[derive(Deserialize, JsonSchema)]
pub(crate) struct ReportOutcomeArgs {
    outcome: String,
    summary: String,
    /// For blocked outcomes, explain why work cannot continue without human help; otherwise use an empty string.
    blocker_reason: String,
    /// For blocked outcomes, state the specific question or action needed from a human; otherwise use an empty string.
    human_action: String,
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
        Self { allowed, decision }
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
        format!(
            "Required control tool for this branching pipeline step. Call once after finishing your work. Allowed outcomes: {allowed}. Include a concise natural-language summary for the next step."
        )
    }

    fn parameters(&self) -> serde_json::Value {
        let mut schema = rig::__core::tool::args_schema::<ReportOutcomeArgs>();
        schema["properties"]["outcome"]["enum"] = serde_json::Value::Array(
            self.allowed
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
        if !self.allowed.contains(&outcome) {
            return Err(ToolExecutionError::invalid_args(
                "Outcome is not configured for this pipeline step",
            ));
        }
        let mut summary = args.summary.trim().to_owned();
        if summary.is_empty() {
            return Err(ToolExecutionError::invalid_args(
                "Summary must not be empty",
            ));
        }
        if outcome == Outcome::Blocked {
            let reason = args.blocker_reason.trim();
            let action = args.human_action.trim();
            if reason.is_empty() || action.is_empty() {
                return Err(ToolExecutionError::invalid_args(
                    "Blocked outcomes require a blocker reason and a specific human action",
                ));
            }
            summary.push_str(&format!(
                "\n\nBlocker: {reason}\nHuman action needed: {action}"
            ));
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
        *decision = Some(AgentDecision { outcome, summary });
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
    async fn blocked_requires_reason_and_human_action() {
        let decision = Arc::new(Mutex::new(None));
        let tool = ReportOutcome::new(vec![Outcome::Blocked], decision.clone());
        let schema = tool.parameters();
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("blocker_reason"))
        );
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("human_action"))
        );
        let mut context = ToolContext::default();
        for (reason, action) in [("", "Provide credentials"), ("Missing credentials", "")] {
            assert!(
                tool.call(
                    &mut context,
                    super::ReportOutcomeArgs {
                        outcome: "blocked".into(),
                        summary: "Cannot continue".into(),
                        blocker_reason: reason.into(),
                        human_action: action.into(),
                    }
                )
                .await
                .is_err()
            );
        }
        tool.call(
            &mut context,
            super::ReportOutcomeArgs {
                outcome: "blocked".into(),
                summary: "Cannot continue".into(),
                blocker_reason: "Missing credentials".into(),
                human_action: "Provide credentials".into(),
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
                .summary
                .contains("Human action needed: Provide credentials")
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
            },
        )
        .await
        .unwrap();
        assert_eq!(
            *decision.lock().unwrap(),
            Some(AgentDecision {
                outcome: Outcome::Approved,
                summary: "Review passed.".into(),
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
                },
            )
            .await
            .is_err()
        );
    }
}
