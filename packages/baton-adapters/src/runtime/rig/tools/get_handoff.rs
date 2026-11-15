use baton_core::domain::pipeline::StructuredHandoffAttempt;
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

#[derive(Deserialize, JsonSchema)]
pub(crate) struct GetHandoffArgs {
    /// Attempt number shown in the structured-context omission notice.
    attempt: u32,
}

#[derive(Clone)]
pub(crate) struct GetHandoff {
    history: Arc<Mutex<Vec<StructuredHandoffAttempt>>>,
}

impl GetHandoff {
    pub(crate) fn new(history: Arc<Mutex<Vec<StructuredHandoffAttempt>>>) -> Self {
        Self { history }
    }
}

impl Tool for GetHandoff {
    const NAME: &'static str = "get_structured_handoff";
    type Args = GetHandoffArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Retrieve one complete persisted structured report by producing attempt number. Use attempt numbers from the structured-context omission notice; this returns the report's step ID, findings, verification and next action.".into()
    }

    fn parameters(&self) -> serde_json::Value {
        rig::__core::tool::args_schema::<GetHandoffArgs>()
    }

    fn call(
        &self,
        _context: &mut ToolContext,
        args: GetHandoffArgs,
    ) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + Send {
        let history = self.history.clone();
        async move {
            let history = history.lock().map_err(|_| {
                ToolExecutionError::other("Structured handoff history is unavailable")
            })?;
            let report = history
                .iter()
                .find(|report| report.attempt == args.attempt)
                .ok_or_else(|| {
                    ToolExecutionError::invalid_args(
                        "No structured handoff exists for that attempt",
                    )
                })?;
            serde_json::to_string(report)
                .map_err(|error| ToolExecutionError::other(error.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baton_core::domain::pipeline::{Finding, FindingStatus, StructuredHandoff};
    use rig::tool::{Tool, ToolContext};

    #[tokio::test]
    async fn retrieves_persisted_report_by_attempt() {
        let history = Arc::new(Mutex::new(vec![StructuredHandoffAttempt {
            attempt: 12,
            step_id: "review".into(),
            report: StructuredHandoff {
                version: 1,
                summary: "Review".into(),
                findings: vec![Finding {
                    id: "F-omitted".into(),
                    status: FindingStatus::Open,
                    description: "Recheck generated file".into(),
                }],
                verification: vec![],
                next_action: None,
            },
        }]));
        let tool = GetHandoff::new(history);
        let result = tool
            .call(&mut ToolContext::default(), GetHandoffArgs { attempt: 12 })
            .await
            .unwrap();
        assert!(result.contains("F-omitted"));
        assert!(result.contains("review"));
        assert!(
            tool.call(&mut ToolContext::default(), GetHandoffArgs { attempt: 99 })
                .await
                .is_err()
        );
    }
}
