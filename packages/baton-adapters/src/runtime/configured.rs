use baton_core::ports::skills::SkillRepository;
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
    skills: Vec<baton_core::domain::project::Skill>,
    fake: super::FakeRuntime,
    decision: Option<AgentDecision>,
    trace: Vec<RunTraceEntry>,
    credentials: Arc<dyn baton_core::ports::credentials::CredentialRepository>,
    auth_root: std::path::PathBuf,
    _credential_leases: Option<Box<dyn baton_core::ports::credentials::CredentialLease>>,
}
impl ConfiguredRuntime {
    pub fn new(
        credentials: Arc<dyn baton_core::ports::credentials::CredentialRepository>,
        auth_root: std::path::PathBuf,
        skills: Vec<baton_core::domain::project::Skill>,
    ) -> Self {
        Self {
            skills,
            fake: super::FakeRuntime::default(),
            decision: None,
            trace: Vec::new(),
            credentials,
            auth_root,
            _credential_leases: None,
        }
    }
}
#[derive(Clone)]
pub struct ConfiguredRuntimeFactory {
    pub project_dir: std::path::PathBuf,
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
        let pipeline = config
            .pipelines
            .iter()
            .find(|pipeline| pipeline.id == pipeline_id)
            .ok_or("Selected pipeline does not exist")?;
        let ids = pipeline
            .steps
            .iter()
            .filter(|step| !step.human)
            .filter_map(|step| {
                config
                    .agents
                    .iter()
                    .find(|agent| agent.id == step.agent_id)
                    .and_then(|agent| agent.credential_id.clone())
            })
            .collect::<Vec<_>>();
        let leases = self.credentials.acquire_run_leases(&ids)?;
        let skills =
            crate::skills::FilesystemSkillRepository::new(self.project_dir.clone()).list()?;
        let mut runtime =
            ConfiguredRuntime::new(self.credentials.clone(), self.auth_root.clone(), skills);
        runtime._credential_leases = leases;
        Ok(Box::new(runtime))
    }
}

impl AgentRuntime for ConfiguredRuntime {
    fn execute_shell(
        &mut self,
        directory: &Path,
        command: &str,
        timeout_seconds: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<baton_core::domain::work::CheckResult, String> {
        crate::work::run_shell(directory, command, timeout_seconds, cancelled)
    }
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
        let resolved = baton_core::features::skills::resolve(agent, &self.skills)?;
        let agent = &resolved;
        match agent.runtime_kind {
            RuntimeKind::Rig => {
                let runtime_credential = if let Some(id) = agent.credential_id.as_deref() {
                    let metadata = self.credentials.get_sync(id)?.ok_or_else(|| {
                        format!("Credential {id:?} was deleted or is unavailable; select another credential for this agent")
                    })?;
                    let secret = self.credentials.secret_sync(id)?;
                    Some((metadata, secret))
                } else {
                    None
                };
                let (output, decision, trace) = super::rig::RigRuntime::execute(
                    directory,
                    agent,
                    &self.skills,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::FilesystemCredentialRepository;
    use baton_core::ports::credentials::CredentialRepository;

    #[tokio::test]
    async fn missing_and_incompatible_credentials_report_clear_errors() {
        use baton_core::domain::credentials::{Credential, CredentialStatus, CredentialType};
        let root = tempfile::tempdir().unwrap();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let mut runtime =
            ConfiguredRuntime::new(repository.clone(), root.path().join("vault"), vec![]);
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"agent", "systemPrompt":"test", "runtimeKind":"rig", "model":{"provider":"openai", "model":"gpt-4.1"}, "credentialId":"shared", "skillIds":[]})).unwrap();
        let error = runtime.execute(root.path(), &agent, "test").unwrap_err();
        assert!(
            error.contains("shared") && error.contains("unavailable"),
            "{error}"
        );
        let credential = Credential {
            id: "shared".into(),
            name: "shared".into(),
            credential_type: CredentialType::new("ollama_api_key").unwrap(),
            configuration: Default::default(),
            created_at: 0,
            updated_at: 0,
            status: CredentialStatus::Available,
        };
        repository
            .save(&credential, Some("local-secret"))
            .await
            .unwrap();
        let error = runtime.execute(root.path(), &agent, "test").unwrap_err();
        assert!(error.contains("openai_api_key"), "{error}");
    }

    #[test]
    fn pipeline_runtime_holds_and_releases_credential_lease() {
        let root = tempfile::tempdir().unwrap();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let factory = ConfiguredRuntimeFactory {
            project_dir: root.path().into(),
            credentials: repository.clone(),
            auth_root: root.path().join("vault"),
        };
        let config = baton_core::domain::project::parse_config(r#"{
            "schemaVersion": 7,
            "agents": [{"id":"agent","systemPrompt":"test","credentialId":"selected","skillIds":[]}],
            "pipelines": [{"id":"pipeline","steps":[{"id":"step","name":"Step","agentId":"agent"}]}]
        }"#).unwrap();
        let runtime = factory.create(&config, "pipeline").unwrap();
        assert!(repository.acquire_delete_lease("selected").is_err());
        assert!(repository.acquire_delete_lease("other").is_ok());
        drop(runtime);
        assert!(repository.acquire_delete_lease("selected").is_ok());
    }
}
