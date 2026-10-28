use baton_core::{
    domain::{
        project::{Agent, Outcome, ProjectConfig},
        runtime_profiles::{self, RuntimeKind},
    },
    ports::runtime::{AgentDecision, AgentRuntime, AgentRuntimeFactory},
};
use std::path::Path;

/// Dispatch strictly by the agent's snapshotted runtime profile. Never fall back.
pub struct ProfileRuntime {
    config: ProjectConfig,
    fake: super::FakeRuntime,
    decision: Option<AgentDecision>,
}
impl ProfileRuntime {
    pub fn new(config: ProjectConfig) -> Self {
        Self {
            config,
            fake: super::FakeRuntime::default(),
            decision: None,
        }
    }
}
#[derive(Clone, Copy, Default)]
pub struct ProfileRuntimeFactory;

impl AgentRuntimeFactory for ProfileRuntimeFactory {
    fn create(
        &self,
        config: &ProjectConfig,
        pipeline_id: &str,
    ) -> Result<Box<dyn AgentRuntime>, String> {
        config.validate()?;
        if !config
            .pipelines
            .iter()
            .any(|pipeline| pipeline.id == pipeline_id)
        {
            return Err("Selected pipeline does not exist".into());
        }
        Ok(Box::new(ProfileRuntime::new(config.clone())))
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
        self.execute_pipeline_step(directory, agent, input, &[], cancelled)
    }
    fn execute_pipeline_step(
        &mut self,
        directory: &Path,
        agent: &Agent,
        input: &str,
        allowed_outcomes: &[Outcome],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String, String> {
        self.decision = None;
        let profile = runtime_profiles::resolve(&self.config, &agent.runtime_profile_id)?;
        match profile.runtime_kind {
            RuntimeKind::Rig => {
                let (output, decision) = super::rig::RigRuntime::execute(
                    directory,
                    agent,
                    &self.config.skills,
                    input,
                    &profile,
                    allowed_outcomes,
                    cancelled,
                )?;
                self.decision = decision;
                Ok(output)
            }
            RuntimeKind::Fake => self
                .fake
                .execute_cancellable(directory, agent, input, cancelled),
        }
    }
    fn take_decision(&mut self) -> Option<AgentDecision> {
        self.decision.take()
    }
}
