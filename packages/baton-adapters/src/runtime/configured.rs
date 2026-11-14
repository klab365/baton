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
    pub(super) run_id: String,
    pub(super) history_required: bool,
    fake: super::FakeRuntime,
    pub(super) decision: Option<AgentDecision>,
    pub(super) trace: Vec<RunTraceEntry>,
    credentials: Arc<dyn baton_core::ports::credentials::CredentialRepository>,
    pub(super) auth_root: std::path::PathBuf,
    _credential_leases: Option<Box<dyn baton_core::ports::credentials::CredentialLease>>,
    pub(super) pi_command: std::ffi::OsString,
}
impl ConfiguredRuntime {
    pub fn new(
        credentials: Arc<dyn baton_core::ports::credentials::CredentialRepository>,
        auth_root: std::path::PathBuf,
        skills: Vec<baton_core::domain::project::Skill>,
    ) -> Self {
        Self {
            skills,
            run_id: uuid::Uuid::now_v7().to_string(),
            history_required: false,
            fake: super::FakeRuntime::default(),
            decision: None,
            trace: Vec::new(),
            credentials,
            auth_root,
            _credential_leases: None,
            pi_command: "pi".into(),
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
            .filter(|step| !step.is_human())
            .filter_map(|step| {
                config
                    .agents
                    .iter()
                    .find(|agent| agent.id == step.agent_id())
                    .and_then(|agent| {
                        (agent.runtime_kind == RuntimeKind::Rig)
                            .then(|| agent.credential_id.clone())
                            .flatten()
                    })
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
    fn list_models(&mut self) -> Result<Vec<baton_core::domain::runtime::RuntimeModel>, String> {
        Err("This runtime does not expose model discovery".into())
    }
    fn take_events(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn supports_session_resumption(&self, agent: &Agent) -> bool {
        matches!(agent.runtime_kind, RuntimeKind::Rig | RuntimeKind::Pi)
    }

    fn set_run_id(&mut self, run_id: &str) -> Result<(), String> {
        if run_id.is_empty()
            || !run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("Run ID must contain only ASCII letters, digits or hyphens".into());
        }
        self.run_id = run_id.to_owned();
        Ok(())
    }

    fn set_history_required(&mut self, required: bool) {
        self.history_required = required;
    }

    fn execute_shell(
        &mut self,
        directory: &Path,
        command: &str,
        timeout_seconds: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<baton_core::domain::work::CheckResult, String> {
        crate::work::run_pipeline_shell(directory, command, timeout_seconds, cancelled)
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
        self.execute_pipeline_step_observed(
            directory,
            agent,
            input,
            allowed_outcomes,
            cancelled,
            &mut |_| Ok(()),
        )
    }
    fn execute_pipeline_step_observed(
        &mut self,
        directory: &Path,
        agent: &Agent,
        input: &str,
        allowed_outcomes: &[Outcome],
        cancelled: &dyn Fn() -> bool,
        observer: &mut dyn FnMut(RunTraceEntry) -> Result<(), String>,
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
                        run_id: &self.run_id,
                        session_root: &self.auth_root,
                        history_required: self.history_required,
                        credential: runtime_credential,
                        auth_store_dir: agent
                            .credential_id
                            .as_ref()
                            .map(|id| self.auth_root.join("auth").join(id)),
                        observer,
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
            RuntimeKind::Pi => super::pi::execute(
                self,
                directory,
                agent,
                input,
                allowed_outcomes,
                cancelled,
                observer,
            ),
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
    use baton_core::{
        domain::runtime::RuntimeKind,
        features::pipeline_engine::start_pipeline,
        ports::{credentials::CredentialRepository, runs::RunRepository},
    };
    use std::time::Duration;

    #[cfg(unix)]
    #[test]
    fn pi_detached_pipe_holder() {
        let Ok(pid_file) = std::env::var("BATON_PI_DESCENDANT_FILE") else {
            return;
        };
        #[cfg(unix)]
        unsafe {
            libc::setsid();
        }
        std::fs::write(pid_file, std::process::id().to_string()).unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    #[cfg(unix)]
    #[test]
    fn pi_cli_receives_worktree_prompt_model_and_restricted_tools() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let args = root.path().join("args");
        let prompt = root.path().join("prompt");
        let pi = bin.join("pi");
        std::fs::write(
            &pi,
            format!(
                r#"#!/bin/sh
printf '%s\n' "$@" > '{}'
while [ "$#" -gt 0 ]; do
  if [ "$1" = '--session-dir' ]; then shift; session="$1"; fi
  if [ "$1" = '--session-id' ]; then shift; id="$1"; fi
  shift
done
printf '{{"type":"session","id":"%s","cwd":"%s"}}\n{{"type":"message","message":{{"role":"user"}}}}\n{{"type":"message","message":{{"role":"assistant"}}}}\n' "$id" "$PWD" > "$session/test_$id.jsonl"
cat > '{}'
printf 'done'
"#,
                args.display(),
                prompt.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let mut runtime =
            ConfiguredRuntime::new(repository.clone(), root.path().join("vault"), vec![]);
        runtime.pi_command = pi.clone().into();
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"pi", "instructions":"role", "provider":"pi", "model":"openai/gpt-4.1", "credentialId":null, "skillIds":[], "allowedTools":["read", "shell", "report_outcome"]})).unwrap();
        let large_prompt = format!("- {}", "x".repeat(256 * 1024));
        let result = runtime.execute(root.path(), &agent, &large_prompt);
        assert_eq!(result.unwrap(), "done");
        assert_eq!(std::fs::read_to_string(prompt).unwrap(), large_prompt);
        let args = std::fs::read_to_string(args).unwrap();
        assert!(args.contains("--no-extensions") && args.contains("--no-mcp"));
        assert!(args.contains("--model\nopenai/gpt-4.1"));
        assert!(args.contains("--tools\nread,bash"));
        assert!(args.ends_with("--\n"));
        std::fs::write(
            &pi,
            r#"#!/bin/sh
cat > /dev/null
printf '{"outcome":"completed","summary":"handoff"}' > "$BATON_PI_OUTCOME_FILE"
printf 'done'
"#,
        )
        .unwrap();
        let allowed = [Outcome::Completed, Outcome::Blocked];
        assert_eq!(
            runtime
                .execute_pipeline_step(root.path(), &agent, "task", &allowed, &|| false)
                .unwrap(),
            "done"
        );
        let decision = runtime.take_decision().unwrap();
        assert_eq!(decision.outcome, Outcome::Completed);
        assert_eq!(decision.summary, "handoff");
        let calls = root.path().join("followup-calls");
        std::fs::write(
            &pi,
            format!(
                "#!/bin/sh\ncat > /dev/null\nprintf x >> '{}'\nprintf 'no tool call'\n",
                calls.display()
            ),
        )
        .unwrap();
        assert_eq!(
            runtime
                .execute_pipeline_step(root.path(), &agent, "task", &allowed, &|| false)
                .unwrap(),
            "no tool call"
        );
        assert!(runtime.take_decision().is_none());
        assert_eq!(std::fs::read_to_string(&calls).unwrap(), "xxx");
        std::fs::write(&pi, format!("#!/bin/sh\ncat > /dev/null\nprintf x >> '{}'\nprintf '{{\"outcome\":\"approved\",\"summary\":\"wrong\"}}' > \"$BATON_PI_OUTCOME_FILE\"\n", calls.display())).unwrap();
        runtime
            .execute_pipeline_step(root.path(), &agent, "task", &allowed, &|| false)
            .unwrap();
        assert!(runtime.take_decision().is_none());
        assert_eq!(std::fs::read_to_string(&calls).unwrap(), "xxxxxx");
        std::fs::write(
            &pi,
            "#!/bin/sh\necho 'Error: Unknown option: secret-token' >&2\nexit 7\n",
        )
        .unwrap();
        let error = runtime.execute(root.path(), &agent, "task").unwrap_err();
        assert!(error.contains("unsupported CLI option"));
        assert!(!error.contains("secret-token"));
        let root = tempfile::tempdir().unwrap();
        let mut runtime =
            ConfiguredRuntime::new(repository.clone(), root.path().join("vault"), vec![]);
        runtime.pi_command = pi.clone().into();
        std::fs::write(
            &pi,
            "#!/bin/sh\necho 'Error: Model \"fake/nonexistent-secret\" not found. Use --list-models to see available models.' >&2\nexit 7\n",
        )
        .unwrap();
        let error = runtime.execute(root.path(), &agent, "task").unwrap_err();
        assert!(error.contains("configured model was not found"), "{error}");
        assert!(!error.contains("nonexistent-secret"));
        let root = tempfile::tempdir().unwrap();
        let mut runtime =
            ConfiguredRuntime::new(repository.clone(), root.path().join("vault"), vec![]);
        runtime.pi_command = pi.clone().into();
        std::fs::write(
            &pi,
            "#!/bin/sh\necho 'No API key found for private-provider.' >&2\nexit 7\n",
        )
        .unwrap();
        let error = runtime.execute(root.path(), &agent, "task").unwrap_err();
        assert!(
            error.contains("provider API key is not configured"),
            "{error}"
        );
        assert!(!error.contains("private-provider"));
        let root = tempfile::tempdir().unwrap();
        let mut runtime =
            ConfiguredRuntime::new(repository.clone(), root.path().join("vault"), vec![]);
        runtime.pi_command = pi.clone().into();
        std::fs::write(&pi, "#!/bin/sh\necho 'opaque-secret' >&2\nexit 7\n").unwrap();
        let error = runtime.execute(root.path(), &agent, "task").unwrap_err();
        assert!(error.contains("no safe diagnostic available"), "{error}");
        assert!(!error.contains("opaque-secret") && !error.contains("credentials"));
        let root = tempfile::tempdir().unwrap();
        let mut runtime =
            ConfiguredRuntime::new(repository.clone(), root.path().join("vault"), vec![]);
        runtime.pi_command = pi.clone().into();
        std::fs::write(&pi, "#!/bin/sh\ncat >/dev/null\nexit 7\n").unwrap();
        let error = runtime.execute(root.path(), &agent, "task").unwrap_err();
        assert!(error.contains("no safe diagnostic available"));
        let root = tempfile::tempdir().unwrap();
        let mut runtime =
            ConfiguredRuntime::new(repository.clone(), root.path().join("vault"), vec![]);
        runtime.pi_command = pi.clone().into();
        std::fs::write(
            &pi,
            "#!/bin/sh\nprintf 'unauthorized %0100000d' 0 >&2\nexit 7\n",
        )
        .unwrap();
        let error = runtime.execute(root.path(), &agent, "task").unwrap_err();
        assert!(error.contains("authentication failed"));
        assert!(error.contains("diagnostics truncated"));
        assert!(!error.contains("000000"));
    }

    #[cfg(unix)]
    #[test]
    fn pi_failure_diagnostic_survives_pipeline_repository_reload() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(root.path())
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ]);
        let pi = root.path().join("pi");
        std::fs::write(&pi, "#!/bin/sh\necho 'Error: Model \"fake/private-model\" not found. Use --list-models to see available models.' >&2\nexit 7\n").unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut config: baton_core::domain::project::ProjectConfig = serde_json::from_str(
            include_str!("../../../baton-core/tests/fixture_config.json"),
        )
        .unwrap();
        config.agents[0].runtime_kind = RuntimeKind::Pi;
        config.pipelines[0].steps.truncate(1);
        config.pipelines[0].steps[0].transitions.clear();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let mut runtime = ConfiguredRuntime::new(repository, root.path().join("vault"), vec![]);
        runtime.pi_command = pi.into();
        let runs = crate::repositories::filesystem_run::FilesystemRunRepository(root.path().into());
        let run =
            start_pipeline(&config, "draft", "task", "pi-failure", &mut runtime, &runs).unwrap();
        assert_eq!(run.status, baton_core::domain::pipeline::Status::Failed);
        assert!(
            run.steps[0]
                .error
                .as_deref()
                .unwrap()
                .contains("configured model was not found")
        );
        assert!(
            !serde_json::to_string(&run)
                .unwrap()
                .contains("private-model")
        );
        let reopened = runs.load("pi-failure").unwrap();
        assert_eq!(reopened.steps[0].error, run.steps[0].error);
        assert_eq!(reopened.steps[0].agent_id, "writer");
        assert_eq!(reopened.steps[0].attempt, 1);
    }

    #[cfg(unix)]
    #[test]
    fn pi_resumes_per_agent_and_run_and_retries_missing_outcomes() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let pi = root.path().join("pi");
        std::fs::write(&pi, r##"#!/bin/sh
args="$*"
while [ "$#" -gt 0 ]; do
  if [ "$1" = '--session-dir' ]; then shift; session="$1"; fi
  if [ "$1" = '--session-id' ]; then shift; id="$1"; fi
  if [ "$1" = '--session' ]; then shift; file="$1"; fi
  shift
done
prompt=$(cat)
printf '%s\n' "$args" >> "$session/args"
if [ -z "$file" ]; then file="$session/test_$id.jsonl"; fi
if [ -f "$file" ]; then
  grep -q 'prior-conversation' "$file" || exit 9
else
  printf '{"type":"session","id":"%s","cwd":"%s"}\n{"type":"message","message":{"role":"user","content":"prior-conversation"}}\n{"type":"message","message":{"role":"assistant"}}\n' "$id" "$PWD" > "$file"
fi
case "$prompt" in
  *'You did not call report_outcome'*)
    printf '{"outcome":"completed","summary":"handoff"}' > "$BATON_PI_OUTCOME_FILE" ;;
esac
printf '%s\n' "$prompt" >> "$session/prompts"
printf 'handoff'
"##).unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let vault = root.path().join("vault");
        let make_runtime = || {
            let repository = Arc::new(FilesystemCredentialRepository::new(vault.clone()));
            let mut runtime = ConfiguredRuntime::new(repository, vault.clone(), vec![]);
            runtime.pi_command = pi.clone().into();
            runtime
        };
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"first", "instructions":"role", "provider":"pi", "skillIds":[], "allowedTools":["report_outcome"]})).unwrap();
        let mut runtime = make_runtime();
        runtime.set_run_id("run-one").unwrap();
        assert_eq!(
            runtime
                .execute_pipeline_step(
                    root.path(),
                    &agent,
                    "original task",
                    &[Outcome::Completed],
                    &|| false
                )
                .unwrap(),
            "handoff"
        );
        assert_eq!(runtime.take_decision().unwrap().outcome, Outcome::Completed);
        let first = vault.join("runtime-sessions/run-one/agent-first/pi");
        let prompts = std::fs::read_to_string(first.join("prompts")).unwrap();
        assert!(prompts.contains("original task"));
        assert!(prompts.contains("You did not call report_outcome"));
        let session_path = std::fs::read_dir(&first)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .unwrap();
        let args = std::fs::read_to_string(first.join("args")).unwrap();
        assert!(args.contains("--session-id "));
        assert!(args.contains(&format!("--session {}", session_path.display())));
        let mut restarted = make_runtime();
        restarted.set_run_id("run-one").unwrap();
        restarted.set_history_required(true);
        restarted
            .execute(root.path(), &agent, "new handoff")
            .unwrap();
        assert!(
            std::fs::read_to_string(first.join("prompts"))
                .unwrap()
                .contains("new handoff")
        );
        let mut other = agent.clone();
        other.id = "second".into();
        restarted.set_history_required(false);
        restarted
            .execute(root.path(), &other, "other agent")
            .unwrap();
        let second_dir = vault.join("runtime-sessions/run-one/agent-second/pi");
        let second_path = std::fs::read_dir(second_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .unwrap();
        assert_ne!(session_path.file_name(), second_path.file_name());
        restarted.set_run_id("run-two").unwrap();
        restarted.execute(root.path(), &agent, "other run").unwrap();
        let other_run_dir = vault.join("runtime-sessions/run-two/agent-first/pi");
        let other_run_path = std::fs::read_dir(other_run_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .unwrap();
        assert_ne!(session_path.file_name(), other_run_path.file_name());
        restarted.set_run_id("run-one").unwrap();
        restarted.set_history_required(true);
        let saved_session = std::fs::read(&session_path).unwrap();
        std::fs::remove_file(&session_path).unwrap();
        assert!(
            restarted
                .execute(root.path(), &agent, "resume")
                .unwrap_err()
                .contains("session is missing")
        );
        std::fs::write(&session_path, "corrupt\n").unwrap();
        assert!(
            restarted
                .execute(root.path(), &agent, "resume")
                .unwrap_err()
                .contains("session is invalid")
        );
        std::fs::write(&session_path, saved_session).unwrap();
        std::fs::write(first.join("baton-inflight"), "interrupted").unwrap();
        assert!(
            restarted
                .execute(root.path(), &agent, "resume")
                .unwrap_err()
                .contains("reconcile side effects")
        );
    }

    #[cfg(unix)]
    #[test]
    fn pi_process_is_killed_when_cancelled() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let pi = root.path().join("pi");
        std::fs::write(&pi, "#!/bin/sh\nsleep 30 &\nwait\n").unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let mut runtime = ConfiguredRuntime::new(repository, root.path().join("vault"), vec![]);
        runtime.pi_command = pi.into();
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"pi", "instructions":"role", "provider":"pi", "skillIds":[], "allowedTools":[]})).unwrap();
        let started = std::time::Instant::now();
        let result = runtime.execute_cancellable(root.path(), &agent, "task", &|| {
            started.elapsed() > std::time::Duration::from_millis(100)
        });
        assert_eq!(result.unwrap_err(), "Agent execution cancelled");
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn pi_reports_auth_failure_even_if_large_prompt_pipe_closes_early() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let pi = root.path().join("pi");
        std::fs::write(&pi, "#!/bin/sh\necho private-detail >&2\nexit 7\n").unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let mut runtime = ConfiguredRuntime::new(repository, root.path().join("vault"), vec![]);
        runtime.pi_command = pi.into();
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"pi", "instructions":"role", "provider":"pi", "skillIds":[], "allowedTools":[]})).unwrap();
        let error = runtime
            .execute(root.path(), &agent, &"x".repeat(1024 * 1024))
            .unwrap_err();
        assert!(
            error.contains("no safe diagnostic available") && !error.contains("private-detail"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pi_reports_early_stdin_pipe_close() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let pi = root.path().join("pi");
        std::fs::write(&pi, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let mut runtime = ConfiguredRuntime::new(repository, root.path().join("vault"), vec![]);
        runtime.pi_command = pi.into();
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"pi", "instructions":"role", "provider":"pi", "skillIds":[], "allowedTools":[]})).unwrap();
        let error = runtime
            .execute(root.path(), &agent, &"x".repeat(1024 * 1024))
            .unwrap_err();
        assert!(error.contains("Could not send Pi prompt"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn pi_bounds_collection_if_a_descendant_escapes_its_process_group() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let pi = root.path().join("pi");
        let escaped_pid = root.path().join("escaped.pid");
        std::fs::write(
            &pi,
            format!(
                r#"#!/bin/sh
BATON_PI_DESCENDANT_FILE='{}' '{}' --exact runtime::configured::tests::pi_detached_pipe_holder --nocapture &
for i in $(seq 1 100); do [ -s '{}' ] && break; sleep 0.01; done
exit 0
"#,
                escaped_pid.display(), std::env::current_exe().unwrap().display(), escaped_pid.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let mut runtime = ConfiguredRuntime::new(repository, root.path().join("vault"), vec![]);
        runtime.pi_command = pi.into();
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"pi", "instructions":"role", "provider":"pi", "skillIds":[], "allowedTools":[]})).unwrap();
        let started = std::time::Instant::now();
        let error = runtime.execute(root.path(), &agent, "task").unwrap_err();
        assert!(error.contains("kept its output pipes open"), "{error}");
        let pid: i32 = std::fs::read_to_string(escaped_pid)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
    }

    #[cfg(unix)]
    #[test]
    fn pi_checks_cancellation_while_collecting_descendant_pipes() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let pi = root.path().join("pi");
        let pid_file = root.path().join("escaped.pid");
        std::fs::write(
            &pi,
            format!(
                r#"#!/bin/sh
BATON_PI_DESCENDANT_FILE='{}' '{}' --exact runtime::configured::tests::pi_detached_pipe_holder --nocapture &
while [ ! -f '{}' ]; do sleep 0.01; done
exit 0
"#,
                pid_file.display(), std::env::current_exe().unwrap().display(), pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let mut runtime = ConfiguredRuntime::new(repository, root.path().join("vault"), vec![]);
        runtime.pi_command = pi.into();
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"pi", "instructions":"role", "provider":"pi", "skillIds":[], "allowedTools":[]})).unwrap();
        let started = std::time::Instant::now();
        let error = runtime
            .execute_cancellable(root.path(), &agent, "task", &|| {
                pid_file.exists() && started.elapsed() > Duration::from_secs(1)
            })
            .unwrap_err();
        assert_eq!(error, "Agent execution cancelled");
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }

    #[test]
    fn pi_rejects_unsupported_tools_without_running_cli() {
        let root = tempfile::tempdir().unwrap();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let mut runtime = ConfiguredRuntime::new(repository, root.path().join("vault"), vec![]);
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"pi", "instructions":"role", "provider":"pi", "skillIds":[], "allowedTools":["unsupported"]})).unwrap();
        let error = runtime
            .execute_pipeline_step(root.path(), &agent, "task", &[], &|| false)
            .unwrap_err();
        assert!(error.contains("does not support"));
    }

    #[tokio::test]
    async fn missing_and_incompatible_credentials_report_clear_errors() {
        use baton_core::domain::credentials::{Credential, CredentialStatus, CredentialType};
        let root = tempfile::tempdir().unwrap();
        let repository = Arc::new(FilesystemCredentialRepository::new(
            root.path().join("vault"),
        ));
        let mut runtime =
            ConfiguredRuntime::new(repository.clone(), root.path().join("vault"), vec![]);
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"agent", "systemPrompt":"test", "provider":"rig", "model":"openai/gpt-4.1", "credentialId":"shared", "skillIds":[]})).unwrap();
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
            "pipelines": [{"id":"pipeline","steps":[{"id":"step","name":"Step","type":"agent","agentId":"agent"}]}]
        }"#).unwrap();
        let runtime = factory.create(&config, "pipeline").unwrap();
        assert!(repository.acquire_delete_lease("selected").is_err());
        assert!(repository.acquire_delete_lease("other").is_ok());
        drop(runtime);
        assert!(repository.acquire_delete_lease("selected").is_ok());
    }
}
