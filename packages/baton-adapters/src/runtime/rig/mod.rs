//! Rig-backed implementation of the core agent execution port.
mod copilot_anthropic;
mod plugins;
mod request_trace;
mod tools;
mod wire_trace;

use self::{
    plugins::{Activities, Completed, RegisteredTools, StepPlugin},
    tools::{GetHandoff, ReportOutcome, outcome_name},
};
use super::usage::UsageAccumulator;
use baton_core::{
    domain::pipeline::{RunTraceEntry, StructuredHandoffAttempt},
    domain::project::{Agent as BatonAgent, Outcome, Skill},
    ports::runtime::AgentDecision,
};
use bevy_app::PluginGroup;
use bevy_ecs::prelude::{Entity, With, Without};
use rig::{
    catalog::Catalog,
    catalog::ModelSpec,
    providers::registry::ProviderId,
    serve::{ErasedHandler, adapters::ModelAdapter},
};
use rig_harness::{
    prelude::{
        Agent, App, AppToolsExt, Connection, Deliver, DeliveryMode, Exit, ExitReason,
        HeadlessPlugins, Home, Invoked, Name, ProjectRoot, ProjectWorkspace, RigHarnessPlugins,
        RigHome, SessionPaths, SystemPrompt, ToolAccess, TurnOutcome, WorkDir,
    },
    rig_ecs::{
        agent::{Conversation, SpawnedBy},
        effects::Handler,
        model::ReasoningSetting,
        session_log::{SessionLog, SessionRestored},
    },
    rig_tools::{
        self,
        process::Launcher,
        workspace::{Local, Workspace},
    },
};
use std::{
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

const MAX_RESTORED_CONTEXT_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn push_activity(entries: &mut Vec<String>, content: String) {
    const MAX_ENTRIES: usize = 128;
    const MAX_BYTES: usize = 64 * 1024;
    const MAX_ENTRY_BYTES: usize = 4096;
    if entries.iter().any(|entry| {
        serde_json::from_str::<serde_json::Value>(entry)
            .ok()
            .is_some_and(|value| {
                value.get("captureTruncated").and_then(|v| v.as_bool()) == Some(true)
            })
    }) {
        return;
    }
    let used = entries.iter().map(String::len).sum::<usize>();
    let marker = serde_json::json!({
        "kind": "traceTruncated",
        "occurredAt": baton_core::features::pipeline_engine::utc_now(),
        "captureTruncated": true,
        "content": "Runtime activity capture limits reached; later events omitted",
    })
    .to_string();
    if entries.len() >= MAX_ENTRIES - 1
        || content.len() > MAX_ENTRY_BYTES
        || used
            .saturating_add(content.len())
            .saturating_add(marker.len())
            > MAX_BYTES
    {
        entries.push(marker);
    } else {
        entries.push(content);
    }
}

fn trace_kind(content: &str, fallback: &str) -> String {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| value.get("kind")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| fallback.to_owned())
}

fn sort_trace_entries(entries: &mut [RunTraceEntry]) {
    entries.sort_by(|left, right| left.occurred_at.cmp(&right.occurred_at));
}

fn captured_entry(content: &str, fallback: &str) -> RunTraceEntry {
    RunTraceEntry {
        attempt: None,
        occurred_at: trace_timestamp(content),
        kind: trace_kind(content, fallback),
        content: content.to_owned(),
    }
}

fn trace_timestamp(content: &str) -> String {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| value.get("occurredAt")?.as_str().map(str::to_owned))
        .unwrap_or_else(baton_core::features::pipeline_engine::utc_now)
}

fn validate_context_size(conversation: &Conversation) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(&conversation.0).map_err(|e| format!("Serialize agent context: {e}"))?;
    if bytes.len() > MAX_RESTORED_CONTEXT_BYTES {
        return Err(
            "Agent context exceeds 4 MiB; start a new run after reconciling the workspace".into(),
        );
    }
    Ok(())
}

fn append_outcome_instruction(preamble: &mut String, allowed_outcomes: &[Outcome]) {
    if allowed_outcomes.is_empty() {
        return;
    }
    let allowed = allowed_outcomes
        .iter()
        .map(outcome_name)
        .collect::<Vec<_>>()
        .join(", ");
    preamble.push_str(&format!(
        "\n\nCall report_outcome with one configured outcome: {allowed}."
    ));
}

fn copilot_session_directory<'a>(
    credential: Option<&(baton_core::domain::credentials::Credential, Option<String>)>,
    directory: Option<&'a Path>,
) -> Result<&'a Path, String> {
    use baton_core::domain::credentials::CredentialStatus;
    let (metadata, _) =
        credential.ok_or("GitHub Copilot requires a configured session credential")?;
    if metadata.credential_type.0 != "github_copilot_session" {
        return Err("GitHub Copilot requires a github_copilot_session credential".into());
    }
    if metadata.status != CredentialStatus::Available {
        return Err("GitHub Copilot session is unavailable; delete the credential, connect the account again, and reassign the agent".into());
    }
    let directory = directory.ok_or("GitHub Copilot requires a credential reference")?;
    if !directory.join(".session").is_file() {
        return Err("GitHub Copilot session is missing; delete the credential, connect the account again, and reassign the agent".into());
    }
    Ok(directory)
}

fn is_copilot_provider(provider: &str) -> bool {
    matches!(provider, "github-copilot" | "copilot")
}

fn configured_reasoning(
    level: baton_core::domain::project::ThinkingLevel,
) -> rig::completion::Reasoning {
    use baton_core::domain::project::ThinkingLevel as Level;
    match level {
        Level::Off => rig::completion::Reasoning::Off,
        Level::Minimal => rig::completion::Effort::Minimal.into(),
        Level::Low => rig::completion::Effort::Low.into(),
        Level::Medium => rig::completion::Effort::Medium.into(),
        Level::High => rig::completion::Effort::High.into(),
        Level::XHigh => rig::completion::Effort::XHigh.into(),
        Level::Max => rig::completion::Effort::Max.into(),
    }
}

fn apply_configured_reasoning(
    world: &mut bevy_ecs::world::World,
    entity: Entity,
    reasoning: Option<rig::completion::Reasoning>,
) {
    if let Some(reasoning) = reasoning {
        world
            .entity_mut(entity)
            .insert(ReasoningSetting(Some(reasoning)));
    }
}

fn copilot_uses_responses(spec: &ModelSpec) -> bool {
    spec.details.provider.as_ref().is_some_and(|provider| {
        provider.shape.as_deref() == Some("responses")
            || (provider.shape.is_none() && provider.npm.as_deref() == Some("@ai-sdk/openai"))
    })
}

#[cfg(test)]
mod thinking_tests {
    use super::{apply_configured_reasoning, configured_reasoning};
    use baton_core::domain::project::ThinkingLevel;
    use rig::completion::{Effort, Reasoning};
    use rig_harness::rig_ecs::model::ReasoningSetting;

    #[test]
    fn configured_levels_map_to_rig_reasoning_values_including_off() {
        for (level, expected) in [
            (ThinkingLevel::Off, Reasoning::Off),
            (ThinkingLevel::Minimal, Effort::Minimal.into()),
            (ThinkingLevel::Low, Effort::Low.into()),
            (ThinkingLevel::Medium, Effort::Medium.into()),
            (ThinkingLevel::High, Effort::High.into()),
            (ThinkingLevel::XHigh, Effort::XHigh.into()),
            (ThinkingLevel::Max, Effort::Max.into()),
        ] {
            assert_eq!(configured_reasoning(level), expected);
        }
    }

    #[test]
    fn absent_configuration_preserves_a_session_saved_setting() {
        let mut world = bevy_ecs::world::World::new();
        let entity = world.spawn(ReasoningSetting(Some(Reasoning::Off))).id();
        apply_configured_reasoning(&mut world, entity, None);
        assert_eq!(
            world.get::<ReasoningSetting>(entity).unwrap().0,
            Some(Reasoning::Off)
        );
        apply_configured_reasoning(&mut world, entity, Some(Effort::High.into()));
        assert_eq!(
            world.get::<ReasoningSetting>(entity).unwrap().0,
            Some(Effort::High.into())
        );
    }
}

#[cfg(test)]
mod injected_execution_tests {
    use super::{RigAgentContext, RigRuntime};
    use baton_core::domain::{
        pipeline::RunTraceEntry,
        project::{Agent, Outcome},
    };
    use rig::{
        completion::{Effort, Reasoning},
        serve::{ErasedHandler, adapters::ModelAdapter},
        test_utils::{MockCompletionModel, MockStreamEvent},
    };

    #[test]
    fn catalog_unsupported_level_fails_before_mock_provider_call() {
        let root = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let mock = MockCompletionModel::text("must not run");
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "id":"writer", "instructions":"Write clearly", "provider":"rig",
            "model":"openai/unlisted-model", "skillIds":[], "allowedTools":[],
            "thinkingLevel":"low"
        }))
        .unwrap();
        let mut observer = |_entry: RunTraceEntry| Ok(());
        let result = RigRuntime::execute(
            root.path(),
            &agent,
            &[],
            "input",
            RigAgentContext {
                agent: &agent,
                run_id: "test-run",
                session_root: sessions.path(),
                history_required: false,
                structured_history: &[],
                credential: None,
                auth_store_dir: None,
                observer: &mut observer,
                handler_override: Some(ErasedHandler::new(ModelAdapter::new("mock", mock.clone()))),
            },
            &[],
            &|| false,
        );
        let error = result.unwrap_err();
        assert!(
            error.contains("does not support the requested thinkingLevel"),
            "{error}"
        );
        assert!(mock.requests().is_empty());
    }

    #[test]
    fn configured_off_reaches_model_that_allows_reasoning_to_be_disabled() {
        let root = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let mock = MockCompletionModel::from_stream_turns((1..=3).map(|turn| {
            vec![
                MockStreamEvent::text(format!("response {turn}")),
                MockStreamEvent::final_response_with_default_usage(),
            ]
        }));
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "id":"writer", "instructions":"Write clearly", "provider":"rig",
            "model":"anthropic/claude-haiku-4-5", "skillIds":[], "allowedTools":[],
            "thinkingLevel":"off"
        }))
        .unwrap();
        let mut observer = |_entry: RunTraceEntry| Ok(());
        let result = RigRuntime::execute(
            root.path(),
            &agent,
            &[],
            "input",
            RigAgentContext {
                agent: &agent,
                run_id: "off-run",
                session_root: sessions.path(),
                history_required: false,
                structured_history: &[],
                credential: None,
                auth_store_dir: None,
                observer: &mut observer,
                handler_override: Some(ErasedHandler::new(ModelAdapter::new("mock", mock.clone()))),
            },
            &[],
            &|| false,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(result.0.contains("response"), "{}", result.0);
        let requests = mock.requests();
        assert!(!requests.is_empty());
        assert!(
            requests
                .iter()
                .all(|request| request.options.reasoning == Some(Reasoning::Off))
        );
    }

    #[test]
    fn configured_high_reaches_requests_without_auto_loading_agents_md() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("AGENTS.md"), [0xff]).unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let mock = MockCompletionModel::from_stream_turns((1..=9).map(|turn| {
            vec![
                MockStreamEvent::text(format!("response {turn}")),
                MockStreamEvent::final_response_with_default_usage(),
            ]
        }));
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "id":"writer", "instructions":"Write clearly", "provider":"rig",
            "model":"copilot/gpt-5.3-codex", "skillIds":[], "allowedTools":[],
            "thinkingLevel":"high"
        }))
        .unwrap();
        let legacy_agent: Agent = serde_json::from_value(serde_json::json!({
            "id":"writer", "instructions":"Write clearly", "provider":"rig",
            "model":"copilot/gpt-5.3-codex", "skillIds":[], "allowedTools":[]
        }))
        .unwrap();
        let outcomes = [Outcome::Completed];
        for (history_required, input, current_agent, expected_output, request_floor) in [
            (false, "first", &agent, "response 1", 1),
            (true, "second", &agent, "response 2", 2),
            (true, "legacy", &legacy_agent, "response 3", 3),
        ] {
            let mut observer = |_entry: RunTraceEntry| Ok(());
            let result = RigRuntime::execute(
                root.path(),
                current_agent,
                &[],
                input,
                RigAgentContext {
                    agent: current_agent,
                    run_id: "test-run",
                    session_root: sessions.path(),
                    history_required,
                    structured_history: &[],
                    credential: None,
                    auth_store_dir: None,
                    observer: &mut observer,
                    handler_override: Some(ErasedHandler::new(ModelAdapter::new(
                        "mock",
                        mock.clone(),
                    ))),
                },
                &outcomes,
                &|| false,
            )
            .unwrap();
            assert!(result.0.contains(expected_output));
            let requests = mock.requests();
            assert!(requests.len() >= request_floor);
            assert!(requests.iter().all(|request| {
                request.options.reasoning == Some(Reasoning::Effort(Effort::High))
            }));
        }
    }
}

pub struct RigRuntime;

pub struct RigAgentContext<'a> {
    pub agent: &'a BatonAgent,
    pub run_id: &'a str,
    pub session_root: &'a Path,
    pub history_required: bool,
    pub structured_history: &'a [StructuredHandoffAttempt],
    pub credential: Option<(baton_core::domain::credentials::Credential, Option<String>)>,
    pub auth_store_dir: Option<std::path::PathBuf>,
    pub observer: &'a mut dyn FnMut(RunTraceEntry) -> Result<(), String>,
    pub handler_override: Option<ErasedHandler>,
}
type RigExecutionResult = Result<
    (
        String,
        Option<AgentDecision>,
        Vec<RunTraceEntry>,
        Option<baton_core::domain::pipeline::ModelUsage>,
    ),
    String,
>;

impl RigRuntime {
    pub fn execute(
        directory: &Path,
        baton_agent: &BatonAgent,
        skills: &[Skill],
        input: &str,
        agent_context: RigAgentContext<'_>,
        allowed_outcomes: &[Outcome],
        cancelled: &dyn Fn() -> bool,
    ) -> RigExecutionResult {
        let runtime_credential = agent_context.credential;
        let model_config = agent_context
            .agent
            .model
            .as_ref()
            .ok_or("Agent needs an explicit provider and model")?;
        if cancelled() {
            return Err("Run cancelled".into());
        }
        let root = directory
            .canonicalize()
            .map_err(|e| format!("Resolve worktree: {e}"))?;
        let provider_name = if is_copilot_provider(&model_config.provider) {
            "copilot"
        } else {
            &model_config.provider
        };
        let provider = ProviderId::catalog(provider_name).ok_or_else(|| {
            format!(
                "Rig does not recognize provider {:?}",
                model_config.provider
            )
        })?;
        let model_reference = format!("{}/{}", provider_name, model_config.model);
        let spec = Catalog::builtin()
            .resolve(&model_reference)
            .map(|resolved| resolved.spec.clone())
            .unwrap_or_else(|_| ModelSpec::new(provider, &model_config.model).with_tools(true));
        let reasoning = baton_agent.thinking_level.map(configured_reasoning);
        if let Some(reasoning) = reasoning {
            if is_copilot_provider(&model_config.provider) && !copilot_uses_responses(&spec) {
                return Err(
                    "Rig catalog does not select the Copilot Responses route for this model".into(),
                );
            }
            spec.validate(&spec.default_options(Some(reasoning)))
                .map_err(|error| {
                    format!("Rig model does not support the requested thinkingLevel: {error}")
                })?;
        }
        let wire_traces = Arc::new(Mutex::new(Vec::new()));
        // Ollama's direct Rig client reads its distinct OLLAMA_API_BASE_URL setting;
        // the generic registry dialect otherwise falls back to localhost.
        let handler = if let Some(handler) = agent_context.handler_override {
            handler
        } else {
            if is_copilot_provider(&model_config.provider) {
                use rig::providers::copilot::auth::{
                    AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore,
                };
                let directory = copilot_session_directory(
                    runtime_credential.as_ref(),
                    agent_context.auth_store_dir.as_deref(),
                )?;
                let spec = Catalog::builtin()
                    .resolve(&format!("copilot/{}", model_config.model))
                    .map_err(|_| "GitHub Copilot model is not available")?
                    .spec
                    .clone();
                let authenticator = Authenticator::new(
                    AuthSource::OAuth,
                    FileTokenStore::new(directory),
                    DeviceCodeHandler::default(),
                    false,
                );
                let responses = copilot_uses_responses(&spec);
                if model_config.model.starts_with("claude-") || responses {
                    ErasedHandler::new(copilot_anthropic::CopilotAnthropic::new(
                        model_config.model.clone(),
                        responses,
                        authenticator,
                        wire_traces.clone(),
                    )?)
                } else {
                    let signed_in = rig::client::signed_in::SignedInModel::new(
                        spec,
                        authenticator,
                        rig::http_client::DynHttpClient::new(
                            rig::http_client::ReqwestClient::default(),
                        )
                        .with_middleware(wire_trace::CopilotSystemString)
                        .with_middleware(wire_trace::WireTrace(wire_traces.clone())),
                    )
                    .map_err(|_| "GitHub Copilot model configuration is invalid")?;
                    ErasedHandler::new(signed_in)
                }
            } else if model_config.provider == "chatgpt" {
                use rig::providers::chatgpt::auth::{
                    AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore,
                };
                let (metadata, _) = runtime_credential
                    .as_ref()
                    .ok_or("ChatGPT runtime requires a configured session credential")?;
                if metadata.credential_type.0 != "chatgpt_session" {
                    return Err("ChatGPT runtime requires a chatgpt_session credential".into());
                }
                let directory = agent_context
                    .auth_store_dir
                    .ok_or("ChatGPT runtime requires a credential reference")?;
                let spec = Catalog::builtin()
                    .resolve(&format!("chatgpt/{}", model_config.model))
                    .map_err(|error| format!("ChatGPT model is not available: {error}"))?
                    .spec
                    .clone();
                let authenticator = Authenticator::new(
                    AuthSource::OAuth,
                    FileTokenStore::new(directory),
                    DeviceCodeHandler::default(),
                    false,
                );
                let signed_in =
                    rig::client::signed_in::SignedInModel::new(
                        spec,
                        authenticator,
                        rig::http_client::DynHttpClient::new(
                            rig::http_client::ReqwestClient::default(),
                        ),
                    )
                    .map_err(|error| format!("ChatGPT model configuration: {error}"))?;
                ErasedHandler::new(signed_in)
            } else if model_config.provider == "ollama" {
                let mut config = rig::providers::ollama::OllamaConfig::new();
                if let Some((credential, secret)) = runtime_credential {
                    if credential.credential_type.0 != "ollama_api_key" {
                        return Err("Ollama runtime requires an ollama_api_key credential".into());
                    }
                    if let Some(base_url) = credential.configuration.get("baseUrl") {
                        config = config.with_base_url(base_url);
                    }
                    if let Some(secret) = secret {
                        config = config.with_api_key(secret);
                    }
                }
                let client = config.client();
                ErasedHandler::new(ModelAdapter::new(
                    baton_agent.id.clone(),
                    client.completion(model_config.model.clone()),
                ))
            } else {
                let model_reference = format!("{}/{}", model_config.provider, model_config.model);
                let catalog = Catalog::builtin();
                let model = if model_config.provider == "openai" {
                    let (metadata, secret) = runtime_credential
                        .as_ref()
                        .ok_or("OpenAI runtime requires an API-key credential")?;
                    if metadata.credential_type.0 != "openai_api_key" {
                        return Err("OpenAI runtime requires an openai_api_key credential".into());
                    }
                    let secret = secret
                        .as_deref()
                        .ok_or("OpenAI credential has no stored secret")?;
                    let model_spec = catalog
                        .resolve(&model_reference)
                        .map_err(|error| format!("OpenAI model is not available: {error}"))?
                        .spec
                        .clone();
                    catalog
                        .connect_with(
                            &model_spec,
                            rig::providers::registry::ConnectOptions::new().api_key(secret),
                        )
                        .map_err(|error| format!("Rig provider configuration: {error}"))?
                } else {
                    catalog
                        .connect(model_reference.as_str())
                        .map_err(|error| format!("Rig provider configuration: {error}"))?
                };
                ErasedHandler::new(ModelAdapter::new(baton_agent.id.clone(), model))
            }
        };
        let request_traces = Arc::new(Mutex::new(Vec::new()));
        let connection = Connection {
            spec: Arc::new(spec),
            handler: Handler(handler.layered(request_trace::RequestTrace(request_traces.clone()))),
        };

        if agent_context.run_id.is_empty()
            || !agent_context
                .run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("Run ID contains invalid path characters".into());
        }
        if baton_agent.id.is_empty()
            || !baton_agent
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err("Agent ID contains invalid path characters".into());
        }
        let session = agent_context
            .session_root
            .join("runtime-sessions")
            .join(agent_context.run_id)
            .join(format!("agent-{}", baton_agent.id));
        if agent_context.history_required && !session.join("saved").is_file() {
            return Err("Agent session is missing; manual recovery required".into());
        }
        let mut directories = std::fs::DirBuilder::new();
        directories.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directories.mode(0o700);
        }
        directories
            .create(&session)
            .map_err(|e| format!("Create Rig session: {e}"))?;
        let inflight = session.join("baton-inflight");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut marker = options.open(&inflight).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "Previous agent execution was interrupted or failed; reconcile side effects and remove its baton-inflight marker before resuming".to_owned()
            } else {
                format!("Create Rig execution marker: {e}")
            }
        })?;
        marker
            .write_all(b"Agent execution requires reconciliation if interrupted.\n")
            .and_then(|()| marker.sync_all())
            .map_err(|e| format!("Write Rig execution marker: {e}"))?;
        drop(marker);
        let workspace: Arc<dyn Workspace> = Arc::new(Local);
        let activities: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut app = App::new();
        app.insert_resource(ProjectRoot(root.clone()))
            .insert_resource(ProjectWorkspace(workspace.clone()))
            .insert_resource(RigHome(Home::at(session.join("rig-home"))))
            .insert_resource(SessionPaths(rig_harness::harness_protocol::SessionDir::at(
                &session,
            )))
            .insert_resource(Invoked::default())
            .insert_resource(Activities(activities.clone()))
            .set_error_handler(rig_harness::error::warn)
            .add_plugins((
                HeadlessPlugins
                    .build()
                    .disable::<bevy_log::LogPlugin>()
                    .disable::<rig_harness::host::signals::ExitOnSignalPlugin>(),
                RigHarnessPlugins,
                rig_basics::compaction::CompactionPlugin,
                StepPlugin,
            ));
        let decision = Arc::new(Mutex::new(None));
        let handoff_history = Arc::new(Mutex::new(agent_context.structured_history.to_vec()));
        let mut registered_tools = vec!["get_structured_handoff"];
        app.add_tool(GetHandoff::new(handoff_history));
        if baton_agent.allows_tool("read") {
            registered_tools.push("read");
            app.add_tool(rig_tools::Read {
                workspace: workspace.clone(),
                base: Some(root.clone()),
                confine: true,
                seen: Default::default(),
            });
        }
        if baton_agent.allows_tool("list") {
            registered_tools.push("list");
            app.add_tool(rig_tools::List {
                workspace: workspace.clone(),
                base: Some(root.clone()),
                confine: true,
            });
        }
        if baton_agent.allows_tool("search") {
            registered_tools.push("search");
            app.add_tool(rig_tools::Search {
                workspace: workspace.clone(),
                base: Some(root.clone()),
                confine: true,
            });
        }
        if baton_agent.allows_tool("shell") {
            registered_tools.push("shell");
            app.add_tool(rig_tools::Shell {
                workspace: workspace.clone(),
                launcher: Launcher::default().in_dir(root.clone()),
                confine: true,
                ..Default::default()
            });
        }
        if baton_agent.allows_tool("write") {
            registered_tools.push("write");
            app.add_tool(rig_tools::Write {
                workspace: workspace.clone(),
                base: Some(root.clone()),
                confine: true,
                seen: Default::default(),
            });
        }
        if baton_agent.allows_tool("edit") {
            registered_tools.push("edit");
            app.add_tool(rig_tools::Edit {
                workspace: workspace.clone(),
                base: Some(root.clone()),
                confine: true,
                seen: Default::default(),
            });
        }
        if baton_agent.allows_tool("report_outcome") {
            registered_tools.push("report_outcome");
            app.add_tool(ReportOutcome::new(
                allowed_outcomes.to_vec(),
                decision.clone(),
            ));
        }
        app.insert_resource(RegisteredTools(
            registered_tools
                .iter()
                .map(|tool| (*tool).to_owned())
                .collect(),
        ));
        app.finish();
        app.cleanup();
        app.update();
        if app.should_exit().is_some() {
            return Err("Rig Harness startup failed".into());
        }
        let restored = app
            .world()
            .get_resource::<SessionRestored>()
            .ok_or("Rig session could not be restored; manual recovery required")?;
        if !restored.repairs.is_empty() {
            return Err(
                "Rig session was repaired after an interruption; manual recovery required".into(),
            );
        }
        // Startup may report the interrupted previous turn; it is not the new turn's result.
        app.world_mut().resource_mut::<Completed>().0.take();

        let mut preamble = baton_agent.instructions.clone();
        for skill_id in &baton_agent.skill_ids {
            let skill = skills
                .iter()
                .find(|skill| &skill.id == skill_id)
                .ok_or_else(|| format!("Unknown skill {skill_id:?}"))?;
            preamble.push_str(&format!("\n\nSkill {}:\n{}", skill.id, skill.instructions));
        }
        preamble.push_str("\n\nUse get_structured_handoff(attempt) to retrieve the complete report when the structured-context notice says details were omitted; the attempt number identifies its producer.");
        append_outcome_instruction(&mut preamble, allowed_outcomes);
        let setup_trace = RunTraceEntry {
            attempt: None,
            occurred_at: baton_core::features::pipeline_engine::utc_now(),
            kind: "runtimeSetup".into(),
            content: serde_json::json!({
                "agentId": baton_agent.id,
                "model": model_reference,
                "registeredTools": registered_tools,
                "allowedOutcomes": allowed_outcomes.iter().map(outcome_name).collect::<Vec<_>>(),
            })
            .to_string(),
        };
        (agent_context.observer)(setup_trace.clone())?;
        let world = app.world_mut();
        let agents = world
            .query_filtered::<Entity, (With<Agent>, Without<SpawnedBy>)>()
            .iter(world)
            .collect::<Vec<_>>();
        let [entity] = agents.as_slice() else {
            return Err(
                "Rig session must contain exactly one root agent; manual recovery required".into(),
            );
        };
        let entity = *entity;
        if agent_context.history_required
            && world
                .get::<Conversation>(entity)
                .is_none_or(|c| c.0.messages().is_empty())
        {
            return Err("Rig agent history could not be restored; manual recovery required".into());
        }
        if let Some(conversation) = world.get::<Conversation>(entity) {
            validate_context_size(conversation)?;
        }
        world.entity_mut(entity).insert((
            Name::new(baton_agent.id.clone()),
            SystemPrompt(preamble),
            connection,
            WorkDir(root),
            ToolAccess::All,
        ));
        apply_configured_reasoning(world, entity, reasoning);
        app.world_mut()
            .trigger(Deliver::new(entity, input.to_owned(), DeliveryMode::Queue));

        let mut observed_requests = 0;
        let mut observed_wire = 0;
        let mut observed_activities = 0;
        loop {
            app.update();
            let mut observed = Vec::new();
            let requests = request_traces
                .lock()
                .map_err(|_| "Runtime request trace is unavailable".to_owned())?;
            observed.extend(
                requests
                    .iter()
                    .skip(observed_requests)
                    .map(|content| captured_entry(content, "runtimeRequest")),
            );
            observed_requests = requests.len();
            drop(requests);
            let wire = wire_traces
                .lock()
                .map_err(|_| "Runtime wire trace is unavailable".to_owned())?;
            observed.extend(
                wire.iter()
                    .skip(observed_wire)
                    .map(|content| captured_entry(content, "runtimeWireRequest")),
            );
            observed_wire = wire.len();
            drop(wire);
            let activities = activities
                .lock()
                .map_err(|_| "Runtime activity trace is unavailable".to_owned())?;
            observed.extend(
                activities
                    .iter()
                    .skip(observed_activities)
                    .map(|content| captured_entry(content, "runtimeActivity")),
            );
            observed_activities = activities.len();
            drop(activities);
            sort_trace_entries(&mut observed);
            for entry in observed {
                (agent_context.observer)(entry)?;
            }
            if let Some(outcome) = app.world_mut().resource_mut::<Completed>().0.take() {
                if let Some(failure) = app
                    .world()
                    .get_resource::<SessionLog>()
                    .and_then(|log| log.failure())
                {
                    return Err(format!("Rig session save failed: {failure:?}"));
                }
                let answer = match outcome {
                    TurnOutcome::Answered(rig::message::Message::Assistant(message)) => {
                        message.text()
                    }
                    TurnOutcome::Answered(_) => return Err("Expected assistant message".into()),
                    TurnOutcome::Failed(error) => return Err(format!("Rig agent failed: {error}")),
                    TurnOutcome::Stopped(reason) => {
                        return Err(format!("Rig agent stopped: {reason:?}"));
                    }
                };
                let trace = Vec::new();
                app.world_mut().trigger(Exit(ExitReason::Quit));
                app.update();
                if let Some(failure) = app
                    .world()
                    .get_resource::<SessionLog>()
                    .and_then(|log| log.failure())
                {
                    return Err(format!("Rig session save failed: {failure:?}"));
                }
                let decision = decision
                    .lock()
                    .map_err(|_| "Pipeline outcome state is unavailable".to_owned())?
                    .take();
                std::fs::remove_file(&inflight)
                    .map_err(|e| format!("Clear Rig execution marker: {e}"))?;
                let usage = app.world_mut().resource_mut::<UsageAccumulator>().finish();
                let usage = price_usage(usage, &model_config.provider, &model_config.model);
                return Ok((answer, decision, trace, usage));
            }
            if cancelled() {
                app.world_mut()
                    .trigger(rig_harness::prelude::StopTurn::new(entity));
                app.update();
                return Err("Run cancelled".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn price_usage(
    mut usage: Option<baton_core::domain::pipeline::ModelUsage>,
    provider: &str,
    model: &str,
) -> Option<baton_core::domain::pipeline::ModelUsage> {
    let observed = usage.take()?;
    let prices = std::thread::spawn({
        let provider = provider.to_owned();
        let model = model.to_owned();
        move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?
                .block_on(super::model_pricing::lookup(&provider, &model))
        }
    })
    .join()
    .ok()
    .flatten();
    let Some(prices) = prices else {
        return Some(observed);
    };
    Some(super::model_pricing::apply_prices(
        observed,
        &prices,
        baton_core::features::pipeline_engine::utc_now(),
    ))
}

#[cfg(test)]
mod outcome_prompt_tests {
    use super::{append_outcome_instruction, push_activity, sort_trace_entries};
    use baton_core::domain::{pipeline::RunTraceEntry, project::Outcome};

    #[test]
    fn outcome_prompt_uses_only_configured_choices_without_global_blocked_policy() {
        let mut prompt = "Continue implementation and verification when possible.".to_owned();
        append_outcome_instruction(&mut prompt, &[Outcome::Completed, Outcome::Blocked]);
        assert!(prompt.contains("Continue implementation and verification when possible."));
        assert!(prompt.contains("completed, blocked"));
        assert!(!prompt.contains("Work toward completing"));
        assert!(!prompt.contains("do not stop after inspection"));
        assert!(!prompt.contains("Report actionable results"));
        assert!(!prompt.contains("Use blocked only"));

        let mut other_policy = "Plan only; do not modify files.".to_owned();
        append_outcome_instruction(&mut other_policy, &[Outcome::Approved, Outcome::Rejected]);
        assert!(other_policy.contains("Plan only; do not modify files."));
        assert!(other_policy.contains("approved, rejected"));
        assert!(!other_policy.contains("completed"));
    }

    #[test]
    fn activity_capture_marks_omitted_events_at_entry_and_size_limits() {
        let mut entries = Vec::new();
        for index in 0..200 {
            push_activity(&mut entries, format!("{index}"));
        }
        let marker: serde_json::Value = serde_json::from_str(entries.last().unwrap()).unwrap();
        assert_eq!(entries.len(), 128);
        assert_eq!(marker["kind"], "traceTruncated");
        let prior = entries.len();
        push_activity(&mut entries, "more".into());
        assert_eq!(entries.len(), prior);

        let mut oversized = Vec::new();
        push_activity(&mut oversized, "x".repeat(4097));
        let marker: serde_json::Value = serde_json::from_str(oversized.last().unwrap()).unwrap();
        assert_eq!(marker["captureTruncated"], true);
    }

    #[test]
    fn merged_activity_batch_is_sorted_by_observation_timestamp() {
        let mut entries = vec![
            RunTraceEntry {
                attempt: Some(2),
                occurred_at: "2026-01-01T00:00:02.000Z".into(),
                kind: "toolResult".into(),
                content: "2".into(),
            },
            RunTraceEntry {
                attempt: Some(2),
                occurred_at: "2026-01-01T00:00:01.000Z".into(),
                kind: "modelResponse".into(),
                content: "1".into(),
            },
            RunTraceEntry {
                attempt: Some(2),
                occurred_at: "2026-01-01T00:00:03.000Z".into(),
                kind: "wireRequest".into(),
                content: "3".into(),
            },
        ];
        sort_trace_entries(&mut entries);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.content.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3"]
        );
    }

    #[test]
    fn no_outcome_prompt_is_added_without_choices() {
        let mut prompt = "Agent-specific policy.".to_owned();
        append_outcome_instruction(&mut prompt, &[]);
        assert_eq!(prompt, "Agent-specific policy.");
    }
}

#[cfg(test)]
mod compaction_tests {
    use super::*;
    use bevy_app::PluginGroup;

    #[test]
    fn compaction_starts_with_the_headless_runtime() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new();
        app.insert_resource(ProjectRoot(root.path().to_path_buf()))
            .insert_resource(ProjectWorkspace(Arc::new(Local)))
            .insert_resource(RigHome(Home::at(root.path().join("rig-home"))))
            .insert_resource(SessionPaths(rig_harness::harness_protocol::SessionDir::at(
                root.path(),
            )))
            .insert_resource(Invoked::default())
            .add_plugins((
                HeadlessPlugins
                    .build()
                    .disable::<bevy_log::LogPlugin>()
                    .disable::<rig_harness::host::signals::ExitOnSignalPlugin>(),
                RigHarnessPlugins,
                rig_basics::compaction::CompactionPlugin,
                StepPlugin,
            ));
        app.finish();
        app.cleanup();
        app.update();
        assert!(
            app.world()
                .contains_resource::<rig_basics::compaction::CompactionDefault>()
        );
        assert!(app.should_exit().is_none());
    }
}

#[cfg(test)]
mod context_limit_tests {
    use super::{Conversation, MAX_RESTORED_CONTEXT_BYTES, validate_context_size};

    #[test]
    fn rejects_oversized_restored_context() {
        let small = Conversation(rig::conversation::Conversation::from_messages([
            rig::completion::Message::user("hello"),
        ]));
        assert!(validate_context_size(&small).is_ok());
        let large = Conversation(rig::conversation::Conversation::from_messages([
            rig::completion::Message::user("x".repeat(MAX_RESTORED_CONTEXT_BYTES)),
        ]));
        assert!(
            validate_context_size(&large)
                .unwrap_err()
                .contains("exceeds 4 MiB")
        );
    }
}

#[cfg(test)]
mod copilot_credential_tests {
    use super::{copilot_session_directory, copilot_uses_responses, is_copilot_provider};

    #[test]
    fn copilot_catalog_selects_responses_for_gpt_terra() {
        let catalog = rig::catalog::Catalog::builtin();
        assert!(copilot_uses_responses(
            catalog.resolve("copilot/gpt-6.1-terra").unwrap().spec
        ));
        assert!(!copilot_uses_responses(
            catalog.resolve("copilot/claude-sonnet-5.5").unwrap().spec
        ));
    }

    #[test]
    fn copilot_aliases_use_the_explicit_session_path() {
        assert!(is_copilot_provider("github-copilot"));
        assert!(is_copilot_provider("copilot"));
        assert!(!is_copilot_provider("openai"));
    }
    use baton_core::domain::credentials::{Credential, CredentialStatus, CredentialType};

    #[test]
    fn requires_selected_available_copilot_session_and_its_marker() {
        let dir = tempfile::tempdir().unwrap();
        let mut credential = Credential {
            id: "selected".into(),
            name: "Copilot".into(),
            credential_type: CredentialType::new("github_copilot_session").unwrap(),
            configuration: Default::default(),
            created_at: 0,
            updated_at: 0,
            status: CredentialStatus::Available,
        };
        assert!(copilot_session_directory(None, Some(dir.path())).is_err());
        assert!(copilot_session_directory(Some(&(credential.clone(), None)), None).is_err());
        assert!(
            copilot_session_directory(Some(&(credential.clone(), None)), Some(dir.path())).is_err()
        );
        std::fs::write(dir.path().join(".session"), "").unwrap();
        assert!(
            copilot_session_directory(Some(&(credential.clone(), None)), Some(dir.path())).is_ok()
        );
        credential.status = CredentialStatus::Unavailable;
        assert!(
            copilot_session_directory(Some(&(credential.clone(), None)), Some(dir.path())).is_err()
        );
        credential.status = CredentialStatus::Available;
        credential.credential_type = CredentialType::new("openai_api_key").unwrap();
        assert!(copilot_session_directory(Some(&(credential, None)), Some(dir.path())).is_err());
    }
}
