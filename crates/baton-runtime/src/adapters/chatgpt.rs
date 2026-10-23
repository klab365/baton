use super::rig_support::{agent_instructions, run_async};
use baton_core::{
    orchestration::AgentRuntime,
    project::{Agent, ExecutionConfig},
};
use rig_core::{
    completion::CompletionRequest,
    providers::{
        chatgpt::{
            self,
            auth::{AuthSource, Authenticator, DeviceCodeHandler, FileTokenStore},
        },
        openai::OpenAIConfig,
    },
};
use std::{path::PathBuf, time::Duration};
/// ChatGPT subscription implementation using Rig's OAuth device-code flow.
/// Tokens are kept only in the caller-selected local token directory.
pub struct ChatGptRigRuntime {
    token_dir: PathBuf,
}

impl ChatGptRigRuntime {
    pub fn new(token_dir: PathBuf) -> Self {
        Self { token_dir }
    }
}

impl AgentRuntime for ChatGptRigRuntime {
    fn execute(
        &mut self,
        agent: &Agent,
        _: &ExecutionConfig,
        input: &str,
    ) -> Result<String, String> {
        if agent.model.provider != "chatgpt" {
            return Err(format!(
                "ChatGPT runtime requires agent {} to use model.provider \\\"chatgpt\\\"",
                agent.id
            ));
        }
        if !agent.tool_ids.is_empty() || !agent.skill_ids.is_empty() {
            return Err(format!(
                "ChatGPT runtime does not support configured tools or skills for agent {}",
                agent.id
            ));
        }
        let auth = Authenticator::new(
            AuthSource::OAuth,
            FileTokenStore::new(&self.token_dir),
            DeviceCodeHandler::new(|prompt| {
                eprintln!(
                    "Open {} and enter code {}",
                    prompt.verification_uri, prompt.user_code
                );
            }),
            true,
        );
        let context = run_async(auth.auth_context(&rig_reqwest::shared())).map_err(|_| {
            "ChatGPT sign-in failed; complete the device-code flow and try again".to_owned()
        })?;
        let mut config = OpenAIConfig::with_key(&chatgpt::DIALECT, context.api_key);
        if let Some(account_id) = context.account_id {
            config = config.with_account_id(account_id);
        }
        let model = config.client().completion(&agent.model.model);
        let request = CompletionRequest::new(input)
            .preamble(agent_instructions(agent))
            .max_tokens(agent.model.max_output_tokens.map(u64::from));
        let response = run_async(async {
            tokio::time::timeout(Duration::from_secs(60), model.call(request)).await
        })
        .map_err(|_| "ChatGPT request timed out after 60 seconds".to_owned())?
        .map_err(|_| {
            "ChatGPT request failed; check your subscription and selected model".to_owned()
        })?;
        Ok(response.text())
    }
}
