use baton_core::{
    domain::pipeline::RunTraceEntry,
    domain::{
        project::{Agent, Outcome, ProjectConfig},
        runtime::RuntimeKind,
    },
    ports::runtime::{AgentDecision, AgentRuntime, AgentRuntimeFactory},
};
use std::{path::Path, sync::Arc};

/// Dispatch strictly by the agent's configured runtime. Never fall back.
pub struct ConfiguredRuntime {
    config: ProjectConfig,
    fake: super::FakeRuntime,
    decision: Option<AgentDecision>,
    trace: Vec<RunTraceEntry>,
    credentials: Arc<dyn baton_core::ports::credentials::CredentialRepository>,
    auth_root: std::path::PathBuf,
}
impl ConfiguredRuntime {
    pub fn new(
        config: ProjectConfig,
        credentials: Arc<dyn baton_core::ports::credentials::CredentialRepository>,
        auth_root: std::path::PathBuf,
    ) -> Self {
        Self {
            config,
            fake: super::FakeRuntime::default(),
            decision: None,
            trace: Vec::new(),
            credentials,
            auth_root,
        }
    }
}
#[derive(Clone)]
pub struct ConfiguredRuntimeFactory {
    pub credentials: Arc<dyn baton_core::ports::credentials::CredentialRepository>,
    pub auth_root: std::path::PathBuf,
}

impl AgentRuntimeFactory for ConfiguredRuntimeFactory {
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
        Ok(Box::new(ConfiguredRuntime::new(
            config.clone(),
            self.credentials.clone(),
            self.auth_root.clone(),
        )))
    }
}

impl AgentRuntime for ConfiguredRuntime {
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
        self.trace.clear();
        match agent.runtime_kind {
            RuntimeKind::Rig => {
                let runtime_credential = if let Some(id) = agent.credential_id.as_deref() {
                    self.credentials
                        .get_sync(id)?
                        .map(|metadata| {
                            let secret = self.credentials.secret_sync(id)?;
                            Ok::<_, String>((metadata, secret))
                        })
                        .transpose()?
                } else {
                    None
                };
                let (output, decision, trace) = super::rig::RigRuntime::execute(
                    directory,
                    agent,
                    &self.config.skills,
                    input,
                    super::rig::RigAgentContext {
                        agent,
                        credential: runtime_credential,
                        auth_store_dir: agent
                            .credential_id
                            .as_ref()
                            .map(|id| self.auth_root.join("auth").join(id)),
                    },
                    allowed_outcomes,
                    cancelled,
                )?;
                self.decision = decision;
                self.trace = trace;
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
    fn take_trace(&mut self) -> Vec<RunTraceEntry> {
        std::mem::take(&mut self.trace)
    }
}
