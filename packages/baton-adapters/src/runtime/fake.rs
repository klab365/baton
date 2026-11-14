use baton_core::{domain::project::Agent, ports::runtime::AgentRuntime};
/// Deterministic spike adapter: records calls and prefixes each input with the agent ID.
#[derive(Default)]
pub struct FakeRuntime {
    pub calls: Vec<(String, String)>,
    pub directories: Vec<std::path::PathBuf>,
    pub fail_agent: Option<String>,
}
impl AgentRuntime for FakeRuntime {
    fn set_run_id(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn set_history_required(&mut self, _: bool) {}
    fn execute_shell(
        &mut self,
        _: &std::path::Path,
        _: &str,
        _: u32,
        _: &dyn Fn() -> bool,
    ) -> Result<baton_core::domain::work::CheckResult, String> {
        Err("Shell execution is not configured".into())
    }
    fn list_models(&mut self) -> Result<Vec<baton_core::domain::runtime::RuntimeModel>, String> {
        Err("This runtime does not expose model discovery".into())
    }
    fn execute_cancellable(
        &mut self,
        directory: &std::path::Path,
        agent: &Agent,
        input: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String, String> {
        if cancelled() {
            return Err("Run cancelled".into());
        }
        self.execute(directory, agent, input)
    }
    fn execute_pipeline_step(
        &mut self,
        directory: &std::path::Path,
        agent: &Agent,
        input: &str,
        _: &[baton_core::domain::project::Outcome],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String, String> {
        self.execute_cancellable(directory, agent, input, cancelled)
    }
    fn execute_pipeline_step_observed(
        &mut self,
        directory: &std::path::Path,
        agent: &Agent,
        input: &str,
        outcomes: &[baton_core::domain::project::Outcome],
        cancelled: &dyn Fn() -> bool,
        _: &mut dyn FnMut(baton_core::domain::pipeline::RunTraceEntry) -> Result<(), String>,
    ) -> Result<String, String> {
        self.execute_pipeline_step(directory, agent, input, outcomes, cancelled)
    }
    fn take_decision(&mut self) -> Option<baton_core::ports::runtime::AgentDecision> {
        None
    }
    fn take_events(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn take_trace(&mut self) -> Vec<baton_core::domain::pipeline::RunTraceEntry> {
        Vec::new()
    }
    fn supports_session_resumption(&self, _agent: &Agent) -> bool {
        false
    }

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
