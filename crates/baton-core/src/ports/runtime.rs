use crate::domain::{
    project::{Agent, Outcome, ProjectConfig},
    runtime::RuntimeModel,
};

/// Maximum size for a runtime prompt, response, or pipeline handoff.
pub const MAX_HANDOFF_BYTES: usize = 16 * 1024;

/// A branching pipeline decision reported separately from an agent's prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDecision {
    pub outcome: Outcome,
    pub summary: String,
}

/// Executes one pipeline agent step. Implementations may expose the optional
/// pipeline outcome control tool when `allowed_outcomes` is non-empty.
pub trait AgentRuntime: Send {
    /// Lists models exposed by this explicitly selected runtime without reading credentials.
    fn list_models(&mut self) -> Result<Vec<RuntimeModel>, String> {
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

    /// Executes a pipeline step with the outcomes its transitions allow.
    fn execute_pipeline_step(
        &mut self,
        directory: &std::path::Path,
        agent: &Agent,
        input: &str,
        _allowed_outcomes: &[Outcome],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String, String> {
        self.execute_cancellable(directory, agent, input, cancelled)
    }

    /// Takes the structured decision reported by the last pipeline step, if any.
    fn take_decision(&mut self) -> Option<AgentDecision> {
        None
    }

    /// Sanitized metadata only; never raw runtime diagnostics or authentication data.
    fn take_events(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Executes in the isolated task worktree. Implementations must not change
    /// global process state such as the current working directory.
    fn execute(
        &mut self,
        directory: &std::path::Path,
        agent: &Agent,
        input: &str,
    ) -> Result<String, String>;
}

/// Constructs an agent runtime for a snapshotted pipeline configuration.
pub trait AgentRuntimeFactory: Send + Sync {
    fn create(
        &self,
        config: &ProjectConfig,
        pipeline_id: &str,
    ) -> Result<Box<dyn AgentRuntime>, String>;
}
