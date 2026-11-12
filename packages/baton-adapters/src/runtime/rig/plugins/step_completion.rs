use bevy_ecs::prelude::{Added, Query, Res, ResMut, Resource, With};
use rig_harness::prelude::{
    AgentLoop, App, AppCliExt, Front, IntoScheduleConfigs, Outcome, Settled, ToolRequest, Turn,
    TurnOutcome, Update,
};

#[derive(Default)]
pub(crate) struct StepPlugin;

#[derive(Resource, Default)]
pub(crate) struct Activities(pub(crate) std::sync::Arc<std::sync::Mutex<Vec<String>>>);

#[derive(Resource, Default)]
pub(crate) struct Completed(pub(crate) Option<TurnOutcome>);

#[derive(Resource, Default)]
pub(crate) struct RegisteredTools(pub(crate) Vec<String>);

impl rig_harness::prelude::Plugin for StepPlugin {
    fn build(&self, app: &mut App) {
        app.add_front(Front::new("baton").on_terminal().headless())
            .init_resource::<Completed>()
            .init_resource::<Activities>()
            .init_resource::<RegisteredTools>()
            .add_systems(Update, complete.after(AgentLoop))
            .add_systems(Update, record_tool_calls.after(AgentLoop))
            .add_systems(Update, record_tool_results.after(AgentLoop));
    }
}

fn record_tool_calls(
    calls: Query<&ToolRequest, Added<ToolRequest>>,
    activities: Res<Activities>,
    registered: Res<RegisteredTools>,
) {
    for call in &calls {
        let record = tool_call_activity(call.call.function.name.as_str(), &registered.0);
        if let Ok(mut activities) = activities.0.lock() {
            super::super::push_activity(&mut activities, record);
        }
    }
}

#[allow(clippy::type_complexity)]
fn record_tool_results(
    results: Query<(&ToolRequest, &Outcome), (With<Settled>, Added<Settled>)>,
    activities: Res<Activities>,
    registered: Res<RegisteredTools>,
) {
    for (call, outcome) in &results {
        let name = call.call.function.name.as_str();
        let allowed_name = safe_tool_name(name, &registered.0);
        let status = match (&outcome.0, allowed_name) {
            (Ok(rig::effect::Answered::ToolResult { result }), Some(_)) => result.status_name(),
            (Err(_), Some(_)) => "failed",
            (_, Some(_)) => "unavailable",
            _ => "rejected",
        };
        let record = tool_result_activity(name, status, &registered.0);
        if let Ok(mut activities) = activities.0.lock() {
            super::super::push_activity(&mut activities, record);
        }
    }
}

fn safe_tool_name<'a>(name: &str, registered: &'a [String]) -> Option<&'a str> {
    registered
        .iter()
        .find(|known| known.as_str() == name)
        .map(String::as_str)
}

fn tool_call_activity(name: &str, registered: &[String]) -> String {
    safe_tool_name(name, registered).map_or_else(
        || tool_activity("toolCallRejected", "unrecognized", Some("rejected")),
        |name| tool_activity("toolCall", name, None),
    )
}

fn tool_result_activity(name: &str, status: &str, registered: &[String]) -> String {
    safe_tool_name(name, registered).map_or_else(
        || tool_activity("toolResultRejected", "unrecognized", Some("rejected")),
        |name| tool_activity("toolResult", name, Some(status)),
    )
}

fn tool_activity(kind: &str, tool: &str, status: Option<&str>) -> String {
    serde_json::json!({
        "kind": kind,
        "occurredAt": baton_core::features::pipeline_engine::utc_now(),
        "tool": tool,
        "status": status,
    })
    .to_string()
}

fn complete(
    turns: Query<&TurnOutcome, (With<Turn>, Added<Settled>)>,
    mut result: ResMut<Completed>,
) {
    for outcome in &turns {
        result.0 = Some(outcome.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::{tool_call_activity, tool_result_activity};

    #[test]
    fn tool_trace_contains_registered_names_and_status_but_redacts_untrusted_names() {
        let registered = vec!["read".to_owned(), "write".to_owned()];
        let call = tool_call_activity("read", &registered);
        let result = tool_result_activity("read", "success", &registered);
        let malicious_name = "password-token-secret";
        let rejected = tool_call_activity(malicious_name, &registered);
        let rejected_result = tool_result_activity(malicious_name, "success", &registered);
        let call: serde_json::Value = serde_json::from_str(&call).unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        let rejected: serde_json::Value = serde_json::from_str(&rejected).unwrap();
        let rejected_result: serde_json::Value = serde_json::from_str(&rejected_result).unwrap();
        assert_eq!(call["tool"], "read");
        assert!(call["status"].is_null());
        assert_eq!(result["status"], "success");
        assert_eq!(rejected["tool"], "unrecognized");
        assert!(!rejected.to_string().contains(malicious_name));
        assert_eq!(rejected_result["tool"], "unrecognized");
        assert!(!rejected_result.to_string().contains(malicious_name));
        assert!(!call.to_string().contains("arguments"));
        assert!(!result.to_string().contains("private tool output"));
    }
}
