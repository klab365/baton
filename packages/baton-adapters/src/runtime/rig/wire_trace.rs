use std::sync::{Arc, Mutex};

use rig::{
    http_client::{Bytes, Exchange, HeaderMap, HttpMiddleware},
    serve::Serving,
    wasm_compat::WasmBoxedFuture,
};

/// Capture only message roles from model HTTP bodies; message content can be sensitive.
pub(super) struct WireTrace(pub Arc<Mutex<Vec<String>>>);

/// Copilot Chat compatibility: preserve simple system instructions as plain text.
/// Remove once upstream resolves https://github.com/0xPlaygrounds/rig/issues/3004.
/// Responses requests and system blocks with additional metadata are unchanged.
pub(super) struct CopilotSystemString;

fn system_string_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let messages = value.get_mut("messages")?.as_array_mut()?;
    let mut changed = false;
    for message in messages {
        if message.get("role").and_then(|role| role.as_str()) != Some("system") {
            continue;
        }
        let Some(content) = message.get_mut("content") else {
            continue;
        };
        let Some(parts) = content.as_array() else {
            continue;
        };
        if parts.len() != 1
            || parts[0].get("type").and_then(|kind| kind.as_str()) != Some("text")
            || parts[0].as_object().is_none_or(|part| part.len() != 2)
        {
            continue;
        }
        if let Some(text) = parts[0].get("text").and_then(|text| text.as_str()) {
            *content = serde_json::Value::String(text.to_owned());
            changed = true;
        }
    }
    changed.then(|| serde_json::to_vec(&value).ok()).flatten()
}

impl HttpMiddleware for CopilotSystemString {
    fn before_request_body<'a>(
        &'a self,
        exchange: Exchange<'a>,
        _headers: &'a HeaderMap,
        body: Bytes,
    ) -> WasmBoxedFuture<'a, rig::http_client::Result<Bytes>> {
        Box::pin(async move {
            if exchange.extensions.get::<Serving>().is_some()
                && let Some(rewritten) = system_string_body(&body)
            {
                return Ok(Bytes::from(rewritten));
            }
            Ok(body)
        })
    }
}

fn summarize(body: &[u8]) -> Option<String> {
    let body: serde_json::Value = serde_json::from_slice(body).ok()?;
    let messages = body
        .get("messages")
        .or_else(|| body.get("input"))?
        .as_array()?;
    let roles: Vec<_> = messages
        .iter()
        .filter_map(
            |message| match message.get("role").and_then(|role| role.as_str()) {
                Some("system") => Some("system"),
                Some("developer") => Some("developer"),
                Some("user") => Some("user"),
                Some("assistant") => Some("assistant"),
                Some("tool") => Some("tool"),
                _ => None,
            },
        )
        .take(64)
        .collect();
    Some(
        serde_json::json!({
            "kind": "wireRequest",
            "occurredAt": baton_core::features::pipeline_engine::utc_now(),
            "roles": roles,
            "messageCountTruncated": messages.len() > 64,
        })
        .to_string(),
    )
}

impl HttpMiddleware for WireTrace {
    fn before_request_body<'a>(
        &'a self,
        exchange: Exchange<'a>,
        _headers: &'a HeaderMap,
        body: Bytes,
    ) -> WasmBoxedFuture<'a, rig::http_client::Result<Bytes>> {
        Box::pin(async move {
            if exchange.extensions.get::<Serving>().is_some()
                && let Some(summary) = summarize(&body)
                && let Ok(mut traces) = self.0.lock()
            {
                super::push_activity(&mut traces, summary);
            }
            Ok(body)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_only_single_plain_system_text_blocks() {
        let body = br#"{"messages":[{"role":"system","content":[{"type":"text","text":"Plan only"}]},{"role":"user","content":[{"type":"text","text":"task"}]}]}"#;
        let result: serde_json::Value =
            serde_json::from_slice(&system_string_body(body).unwrap()).unwrap();
        assert_eq!(result["messages"][0]["content"], "Plan only");
        assert!(result["messages"][1]["content"].is_array());
        assert!(system_string_body(br#"{"messages":[{"role":"system","content":[{"type":"text","text":"x","extra":true}]}]}"#).is_none());
        assert!(
            system_string_body(
                br#"{"input":[{"role":"system","content":[{"type":"text","text":"x"}]}]}"#
            )
            .is_none()
        );
        assert!(system_string_body(br#"{"messages":[{"role":"system","content":[{"type":"text","text":"x"},{"type":"text","text":"y"}]}]}"#).is_none());
        assert!(
            system_string_body(br#"{"messages":[{"role":"system","content":"already a string"}]}"#)
                .is_none()
        );
        assert!(system_string_body(b"not json").is_none());
    }

    #[test]
    fn omits_anthropic_system_field_and_user_content() {
        let summary = summarize(br#"{"system":[{"type":"text","text":"Plan only"}],"messages":[{"role":"user","content":"private task"}]}"#).unwrap();
        let value: serde_json::Value = serde_json::from_str(&summary).unwrap();
        assert_eq!(value["roles"], serde_json::json!(["user"]));
        assert!(!summary.contains("Plan only"));
        assert!(!summary.contains("private task"));
    }

    #[test]
    fn retains_system_message_and_roles_but_not_other_body_fields() {
        let summary = summarize(br#"{"messages":[{"role":"system","content":"Plan only"},{"role":"user","content":"private task"}],"authorization":"secret"}"#).unwrap();
        let value: serde_json::Value = serde_json::from_str(&summary).unwrap();
        assert_eq!(value["roles"], serde_json::json!(["system", "user"]));
        assert!(!summary.contains("Plan only"));
        assert!(!summary.contains("authorization"));
        assert!(!summary.contains("secret"));
        assert!(!summary.contains("private task"));
    }
}
