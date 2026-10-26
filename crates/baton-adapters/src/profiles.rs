use baton_core::{
    orchestration::AgentRuntime,
    project::{Agent, ProjectConfig},
    runtime_profiles::{self, RuntimeKind},
};
use std::path::Path;

/// Dispatch strictly by the agent's snapshotted runtime profile. Never fall back.
pub struct ProfileRuntime {
    config: ProjectConfig,
    fake: crate::FakeRuntime,
}
impl ProfileRuntime {
    pub fn new(config: ProjectConfig) -> Self {
        Self {
            config,
            fake: crate::FakeRuntime::default(),
        }
    }
}
impl AgentRuntime for ProfileRuntime {
    fn execute(&mut self, directory: &Path, agent: &Agent, input: &str) -> Result<String, String> {
        self.execute_cancellable(directory, agent, input, &|| false)
    }
    fn execute_cancellable(
        &mut self,
        directory: &Path,
        agent: &Agent,
        input: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String, String> {
        let profile = runtime_profiles::resolve(&self.config, &agent.runtime_profile_id)?;
        match profile.runtime_kind {
            RuntimeKind::Rig => Err("Rig runtime integration is not installed".into()),
            RuntimeKind::Fake => self
                .fake
                .execute_cancellable(directory, agent, input, cancelled),
        }
    }
}
