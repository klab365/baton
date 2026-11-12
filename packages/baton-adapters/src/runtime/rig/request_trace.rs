use std::sync::{Arc, Mutex};

use rig::{
    effect::{Answered, EffectId, EffectKind},
    error::ErrorReport,
    serve::{AfterServe, BeforeServe, Intercept},
};

/// Captures structural request metadata only; prompts and tool arguments may be sensitive.
pub(super) struct RequestTrace(pub Arc<Mutex<Vec<String>>>);

impl Intercept for RequestTrace {
    fn name(&self) -> String {
        "baton-request-trace".into()
    }

    async fn before(&self, _id: EffectId, kind: &EffectKind) -> BeforeServe {
        if let EffectKind::Completion { request, .. } = kind {
            let roles: Vec<_> = request
                .chat_history
                .iter()
                .take(64)
                .map(|message| match message.as_ref() {
                    rig::message::Message::System { .. } => "system",
                    rig::message::Message::User { .. } => "user",
                    rig::message::Message::Assistant(_) => "assistant",
                })
                .collect();
            let tools: Vec<_> = request
                .tools
                .iter()
                .take(64)
                .map(|tool| tool.name.as_str().chars().take(128).collect::<String>())
                .collect();
            let content = serde_json::json!({
                "kind": "modelRequest",
                "occurredAt": baton_core::features::pipeline_engine::utc_now(),
                "messageRoles": roles,
                "messageCountTruncated": request.chat_history.len() > 64,
                "toolNames": tools,
                "toolListTruncated": request.tools.len() > 64,
                "toolNameTruncated": request.tools.iter().any(|tool| tool.name.as_str().chars().count() > 128),
            }).to_string();
            if let Ok(mut trace) = self.0.lock() {
                super::push_activity(&mut trace, content);
            }
        }
        BeforeServe::Proceed
    }

    async fn after(
        &self,
        _id: EffectId,
        kind: &EffectKind,
        outcome: &Result<Answered, ErrorReport>,
    ) -> AfterServe {
        if matches!(kind, EffectKind::Completion { .. }) {
            let status = match outcome {
                Ok(Answered::Completion(_)) => "completed",
                Err(_) => "failed",
                Ok(_) => "unavailable",
            };
            let content = serde_json::json!({
                "kind": "modelResponse",
                "occurredAt": baton_core::features::pipeline_engine::utc_now(),
                "status": status,
            })
            .to_string();
            if let Ok(mut trace) = self.0.lock() {
                super::push_activity(&mut trace, content);
            }
        }
        AfterServe::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::{
        completion::{CompletionRequest, ToolDefinition},
        message::{Message, ToolName},
    };

    #[tokio::test]
    async fn captures_request_structure_without_prompt_or_secret_content() {
        let traces = Arc::new(Mutex::new(Vec::new()));
        let tracer = RequestTrace(traces.clone());
        let mut request = CompletionRequest::from(vec![
            Message::system("Plan only"),
            Message::user("Make a file"),
        ]);
        request.tools.push(Arc::new(ToolDefinition::new(
            ToolName::new("read").unwrap(),
            "read",
            serde_json::json!({}),
        )));
        request.additional_params = Some(serde_json::json!({"secret": "never record"}));
        tracer
            .before(
                EffectId::from_raw(1),
                &EffectKind::Completion {
                    request: Arc::new(request),
                    stream: false,
                },
            )
            .await;
        let entries = traces.lock().unwrap();
        let entry: serde_json::Value = serde_json::from_str(&entries[0]).unwrap();
        assert_eq!(entry["kind"], "modelRequest");
        assert_eq!(entry["messageRoles"], serde_json::json!(["system", "user"]));
        assert_eq!(entry["toolListTruncated"], false);
        assert_eq!(entry["toolNames"], serde_json::json!(["read"]));
        assert!(!entries[0].contains("Plan only"));
        assert!(!entries[0].contains("Make a file"));
        assert!(!entries[0].contains("never record"));
    }
}
