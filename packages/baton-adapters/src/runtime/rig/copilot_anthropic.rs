//! Copilot-authenticated Anthropic Messages and Responses handlers.
use super::wire_trace::WireTrace;
use rig::{
    completion::UnsupportedOption,
    effect::{EffectKind, HandlerDescriptor, family},
    error::{ErrorKind, ErrorReport},
    http_client::{DynHttpClient, ReqwestClient},
    providers::{
        anthropic::wire::{ANTHROPIC, AnthropicConfig, Auth, ToolInputStreaming},
        copilot::auth::Authenticator,
    },
    rig_reqwest::reqwest,
    serve::{Dispatch, Reply, Serve, adapters::ModelAdapter},
};
use std::sync::{Arc, Mutex};

pub(super) struct CopilotAnthropic {
    model: String,
    responses: bool,
    auth: Authenticator,
    auth_http: ReqwestClient,
    http: DynHttpClient,
    unsigned: ModelAdapter<rig::operation::Completion>,
}

fn valid_endpoint(base: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base) else {
        return false;
    };
    let host = url.host_str().unwrap_or("");
    url.scheme() == "https"
        && (host == "githubcopilot.com" || host.ends_with(".githubcopilot.com"))
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && (url.path() == "/" || url.path().is_empty())
}

fn adapter(
    model: &str,
    key: impl Into<rig::wire::Secret>,
    base: &str,
    http: DynHttpClient,
    responses: bool,
) -> ModelAdapter<rig::operation::Completion> {
    if responses {
        let config = rig::providers::openai::wire::OpenAIConfig::with_key(
            &rig::providers::copilot::wire::DIALECT,
            key,
        )
        .with_base_url(base);
        let wire = rig::providers::openai::responses_api::wire::Responses::new(config, model);
        return ModelAdapter::new(
            format!("copilot/{model}"),
            rig::driver::Model::new(wire, http),
        );
    }
    let mut dialect = ANTHROPIC;
    dialect.quirks.auth = Auth::OptionalBearer;
    dialect.quirks.tool_input_streaming = ToolInputStreaming::Off;
    let max_tokens = rig::catalog::Catalog::builtin()
        .resolve(&format!("copilot/{model}"))
        .ok()
        .and_then(|entry| entry.spec.max_output_tokens)
        .map_or(8192, |limit| u64::from(limit).clamp(1, 8192));
    let wire = AnthropicConfig::with_key(&dialect, key)
        .with_base_url(base)
        .completion(model)
        .with_default_max_tokens(max_tokens);
    ModelAdapter::new(
        format!("copilot/{model}"),
        rig::driver::Model::new(wire, http),
    )
}

impl CopilotAnthropic {
    pub(super) fn new(
        model: String,
        responses: bool,
        auth: Authenticator,
        traces: Arc<Mutex<Vec<String>>>,
    ) -> Result<Self, String> {
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in [
            ("user-agent", "GitHubCopilotChat/0.35.0"),
            ("editor-version", "vscode/1.107.0"),
            ("editor-plugin-version", "copilot-chat/0.35.0"),
            ("copilot-integration-id", "vscode-chat"),
        ] {
            headers.insert(
                reqwest::header::HeaderName::from_static(name),
                reqwest::header::HeaderValue::from_static(value),
            );
        }
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Cannot configure Copilot transport")?;
        let http =
            DynHttpClient::new(ReqwestClient::from(client)).with_middleware(WireTrace(traces));
        let unsigned = adapter(
            &model,
            "",
            "https://api.githubcopilot.com",
            http.clone(),
            responses,
        );
        Ok(Self {
            model,
            responses,
            auth,
            auth_http: ReqwestClient::default(),
            http,
            unsigned,
        })
    }
}

impl Serve for CopilotAnthropic {
    type Family = family::Completion;
    fn descriptor(&self) -> HandlerDescriptor {
        self.unsigned.descriptor()
    }
    fn check(&self, kind: &EffectKind) -> Vec<UnsupportedOption> {
        self.unsigned.check(kind)
    }
    async fn serve(&self, kind: EffectKind, dispatch: Dispatch) -> Reply {
        let context = match self.auth.auth_context(&self.auth_http).await {
            Ok(context) => context,
            Err(_) => {
                return Reply::Outcome(Err(ErrorReport::new(
                    ErrorKind::Provider,
                    "GitHub Copilot authentication failed; reconnect the credential",
                )));
            }
        };
        let Some(base) = context
            .api_base
            .as_deref()
            .filter(|base| valid_endpoint(base))
        else {
            return Reply::Outcome(Err(ErrorReport::new(
                ErrorKind::Provider,
                "GitHub Copilot returned an invalid endpoint",
            )));
        };
        adapter(
            &self.model,
            context.api_key,
            base,
            self.http.clone(),
            self.responses,
        )
        .serve(kind, dispatch)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{CopilotAnthropic, valid_endpoint};

    #[tokio::test]
    #[ignore = "requires a real Copilot session and consumes quota"]
    async fn live_responses_tool_call() {
        use rig::{
            completion::{CompletionRequest, ToolDefinition},
            effect::{Answered, EffectId, EffectKind},
            providers::copilot::auth::{
                AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore,
            },
            serve::{Dispatch, ErasedHandler},
        };
        use std::sync::{Arc, Mutex};
        let directory = std::env::var_os("BATON_PROBE_SESSION").expect("BATON_PROBE_SESSION");
        let auth = Authenticator::new(
            AuthSource::OAuth,
            FileTokenStore::new(directory),
            DeviceCodeHandler::default(),
            false,
        );
        let handler = ErasedHandler::new(
            CopilotAnthropic::new(
                "gpt-6.1-terra".into(),
                true,
                auth,
                Arc::new(Mutex::new(Vec::new())),
            )
            .unwrap(),
        );
        let tool = ToolDefinition::new(
            "ping".try_into().unwrap(),
            "Return a greeting",
            serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
        );
        let request = CompletionRequest::new("Call the ping tool before responding.").tool(tool);
        let response = handler
            .handle(
                EffectKind::Completion {
                    request: request.into(),
                    stream: true,
                },
                Dispatch::new(EffectId::from_raw(1), true),
            )
            .await
            .into_outcome()
            .await
            .expect("Copilot Responses request");
        let Answered::Completion(response) = response else {
            panic!("unexpected response")
        };
        assert!(
            response
                .tool_calls()
                .any(|call| call.function.name.as_str() == "ping")
        );
    }
    #[tokio::test]
    #[ignore = "requires a real Copilot session and consumes quota"]
    async fn live_claude_without_max_tokens() {
        use rig::{
            completion::{CompletionRequest, ToolDefinition},
            effect::{Answered, EffectId, EffectKind},
            providers::copilot::auth::{
                AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore,
            },
            serve::{Dispatch, ErasedHandler},
        };
        use std::sync::{Arc, Mutex};
        let directory = std::env::var_os("BATON_PROBE_SESSION").expect("BATON_PROBE_SESSION");
        let auth = Authenticator::new(
            AuthSource::OAuth,
            FileTokenStore::new(directory),
            DeviceCodeHandler::default(),
            false,
        );
        let handler = ErasedHandler::new(
            CopilotAnthropic::new(
                "claude-haiku-5.5".into(),
                false,
                auth,
                Arc::new(Mutex::new(Vec::new())),
            )
            .unwrap(),
        );
        let tool = ToolDefinition::new(
            "ping".try_into().unwrap(),
            "Return a greeting",
            serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
        );
        let request = CompletionRequest::new("Call the ping tool before responding.").tool(tool);
        let response = handler
            .handle(
                EffectKind::Completion {
                    request: request.into(),
                    stream: true,
                },
                Dispatch::new(EffectId::from_raw(1), true),
            )
            .await
            .into_outcome()
            .await
            .expect("Copilot Claude request without max_tokens");
        let Answered::Completion(response) = response else {
            panic!("unexpected response")
        };
        assert!(
            response
                .tool_calls()
                .any(|call| call.function.name.as_str() == "ping")
        );
    }

    #[test]
    fn accepts_only_copilot_https_roots() {
        assert!(valid_endpoint("https://api.individual.githubcopilot.com"));
        for bad in [
            "http://api.githubcopilot.com",
            "https://evilgithubcopilot.com",
            "https://api.githubcopilot.com:444",
            "https://api.githubcopilot.com/path",
            "https://api.githubcopilot.com?q=x",
            "https://user@api.githubcopilot.com",
        ] {
            assert!(!valid_endpoint(bad), "{bad}");
        }
    }
}
