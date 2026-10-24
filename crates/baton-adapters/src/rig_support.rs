use baton_core::project::Agent;
pub(super) fn agent_instructions(agent: &Agent) -> String {
    if agent.rules.is_empty() {
        return agent.system_prompt.clone();
    }
    format!(
        "{}\n\nRules:\n{}",
        agent.system_prompt,
        agent
            .rules
            .iter()
            .map(|rule| format!("- {rule}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

pub(super) fn run_async<T>(future: impl std::future::Future<Output = T>) -> T {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(future))
    } else {
        tokio::runtime::Runtime::new()
            .expect("Tokio runtime can be created for OpenAI execution")
            .block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use super::super::OpenAiRigRuntime;
    use super::*;
    use baton_core::orchestration::AgentRuntime;
    use baton_core::project::{Agent, ModelSettings};

    fn agent(rules: Vec<&str>) -> Agent {
        Agent {
            id: "writer".into(),
            system_prompt: "Be concise.".into(),
            model: ModelSettings {
                provider: "openai".into(),
                model: "gpt-4.1-mini".into(),
                max_output_tokens: Some(42),
            },
            rules: rules.into_iter().map(str::to_owned).collect(),
            tool_ids: Vec::new(),
            skill_ids: Vec::new(),
        }
    }

    #[test]
    fn openai_prompt_includes_system_prompt_and_rules() {
        assert_eq!(
            agent_instructions(&agent(vec!["No markdown.", "Use facts."])),
            "Be concise.\n\nRules:\n- No markdown.\n- Use facts."
        );
    }

    #[test]
    fn openai_rejects_unsupported_tools_without_credentials_or_network() {
        let mut agent = agent(Vec::new());
        agent.tool_ids.push("shell".into());
        assert_eq!(
            OpenAiRigRuntime.execute(std::path::Path::new("/unused"), &agent, "input"),
            Err("OpenAI runtime does not support configured tools for agent writer".into())
        );
    }
}
