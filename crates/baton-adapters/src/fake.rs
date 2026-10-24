use baton_core::{orchestration::AgentRuntime, project::Agent};
/// Deterministic spike adapter: records calls and prefixes each input with the agent ID.
#[derive(Default)]
pub struct FakeRuntime {
    pub calls: Vec<(String, String)>,
    pub directories: Vec<std::path::PathBuf>,
    pub fail_agent: Option<String>,
}
impl AgentRuntime for FakeRuntime {
    fn execute(
        &mut self,
        _directory: &std::path::Path,
        agent: &Agent,
        input: &str,
    ) -> Result<String, String> {
        self.directories.push(_directory.to_path_buf());
        self.calls.push((agent.id.clone(), input.into()));
        if self.fail_agent.as_deref() == Some(&agent.id) {
            return Err("Simulated agent failure".into());
        }
        Ok(format!("{}: {input}", agent.id))
    }
}
