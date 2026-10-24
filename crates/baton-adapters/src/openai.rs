use super::rig_support::{agent_instructions, run_async};
use baton_core::{orchestration::AgentRuntime, project::Agent};
use rig_core::{
    completion::CompletionRequest,
    providers::openai::{OpenAI, OpenAIConfig},
};
use std::time::Duration;

/// OpenAI-compatible completion runtime. Credentials and base URL are supplied by the
/// provider infrastructure; neither is copied into project configuration.
#[derive(Default)]
pub struct OpenAiRigRuntime;

impl OpenAiRigRuntime {
    pub fn execute_with_provider(
        &mut self,
        agent: &Agent,
        input: &str,
        base_url: Option<&str>,
        api_key: &str,
    ) -> Result<String, String> {
        if !agent.tool_ids.is_empty() {
            return Err(format!(
                "OpenAI runtime does not support configured tools for agent {}",
                agent.id
            ));
        }
        if !agent.skill_ids.is_empty() {
            return Err(format!(
                "OpenAI runtime does not support configured skills for agent {}",
                agent.id
            ));
        }
        let mut config = OpenAIConfig::new(api_key);
        if let Some(url) = base_url {
            config = config.with_base_url(url.trim_end_matches('/'));
        }
        let client = config.client();
        let request = CompletionRequest::new(input)
            .preamble(agent_instructions(agent))
            .max_tokens(agent.model.max_output_tokens.map(u64::from));
        let model = client.completion(&agent.model.model);
        let response = run_async(async {
            tokio::time::timeout(Duration::from_secs(60), model.call(request)).await
        })
        .map_err(|_| "OpenAI request timed out after 60 seconds".to_owned())?
        .map_err(|_| "OpenAI request failed; check API authentication, model availability, rate limits, and network access".to_owned())?;
        Ok(response.text())
    }
}

impl AgentRuntime for OpenAiRigRuntime {
    fn execute(&mut self, agent: &Agent, input: &str) -> Result<String, String> {
        if agent.model.provider != "openai" {
            return Err(format!(
                "OpenAI runtime requires agent {} to use model.provider \\\"openai\\\"",
                agent.id
            ));
        }
        if !agent.tool_ids.is_empty() {
            return Err(format!(
                "OpenAI runtime does not support configured tools for agent {}",
                agent.id
            ));
        }
        if !agent.skill_ids.is_empty() {
            return Err(format!(
                "OpenAI runtime does not support configured skills for agent {}",
                agent.id
            ));
        }
        let client = OpenAI::from_env().map_err(|_| {
            "OpenAI credentials are unavailable; set OPENAI_API_KEY in your local environment"
                .to_owned()
        })?;
        let request = CompletionRequest::new(input)
            .preamble(agent_instructions(agent))
            .max_tokens(agent.model.max_output_tokens.map(u64::from));
        let model = client.completion(&agent.model.model);
        let response = run_async(async { tokio::time::timeout(Duration::from_secs(60), model.call(request)).await })
            .map_err(|_| "OpenAI request timed out after 60 seconds".to_owned())?
            .map_err(|_| "OpenAI request failed; check API authentication, model availability, rate limits, and network access".to_owned())?;
        Ok(response.text())
    }
}
