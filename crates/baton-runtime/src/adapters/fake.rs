use baton_core::{
    orchestration::AgentRuntime,
    project::{Agent, ExecutionConfig},
};
/// Deterministic spike adapter: records calls and prefixes each input with the agent ID.
#[derive(Default)]
pub struct FakeRuntime {
    pub calls: Vec<(String, String)>,
    pub fail_agent: Option<String>,
}
impl AgentRuntime for FakeRuntime {
    fn execute(
        &mut self,
        agent: &Agent,
        _: &ExecutionConfig,
        input: &str,
    ) -> Result<String, String> {
        self.calls.push((agent.id.clone(), input.into()));
        if self.fail_agent.as_deref() == Some(&agent.id) {
            return Err("Simulated agent failure".into());
        }
        Ok(format!("{}: {input}", agent.id))
    }
}
