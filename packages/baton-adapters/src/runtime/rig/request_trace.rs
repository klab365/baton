use std::sync::{Arc, Mutex};

use rig::{
    effect::{Answered, EffectId, EffectKind},
    error::ErrorReport,
    serve::{AfterServe, BeforeServe, Intercept},
};

/// Captures only model messages and tool names at the handler boundary, never HTTP headers.
pub(super) struct RequestTrace(pub Arc<Mutex<Vec<String>>>);

impl Intercept for RequestTrace {
    fn name(&self) -> String {
        "baton-request-trace".into()
    }

    async fn before(&self, _id: EffectId, kind: &EffectKind) -> BeforeServe {
        if let EffectKind::Completion { request, .. } = kind {
            let messages: Vec<_> = request
                .chat_history
                .iter()
                .map(|message| message.as_ref())
                .collect();
            let tools: Vec<_> = request
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect();
            let content = serde_json::json!({ "messages": messages, "tools": tools }).to_string();
            if let Ok(mut trace) = self.0.lock() {
                trace.push(content);
            }
        }
        BeforeServe::Proceed
    }

    async fn after(
        &self,
        _id: EffectId,
        _kind: &EffectKind,
        _outcome: &Result<Answered, ErrorReport>,
    ) -> AfterServe {
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
    async fn captures_final_messages_and_tool_names_without_extra_parameters() {
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
        assert_eq!(entry["messages"][0]["role"], "system");
        assert!(entry["messages"][0].to_string().contains("Plan only"));
        assert_eq!(entry["tools"], serde_json::json!(["read"]));
        assert!(!entries[0].contains("never record"));
    }
}
