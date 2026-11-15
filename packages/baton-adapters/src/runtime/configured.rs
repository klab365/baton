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

fn supports_thinking_level(agent: &Agent) -> bool {
    match agent.runtime_kind {
        RuntimeKind::Pi => true,
        RuntimeKind::Rig => agent.model.is_some(),
        RuntimeKind::Fake => false,
    }
}

/// Dispatch strictly by the agent's configured runtime. Never fall back.
pub struct ConfiguredRuntime {
    skills: Vec<baton_core::domain::project::Skill>,
    pub(super) run_id: String,
    pub(super) history_required: bool,
    pub(super) structured_history: Vec<baton_core::domain::pipeline::StructuredHandoffAttempt>,
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
            structured_history: Vec::new(),
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
    fn set_structured_handoff_history(
        &mut self,
        history: Vec<baton_core::domain::pipeline::StructuredHandoffAttempt>,
    ) {
        self.structured_history = history;
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
        if agent.thinking_level.is_some() && !supports_thinking_level(agent) {
            return Err("Agent thinkingLevel is not supported for this runtime/model/level combination; remove it to run this agent".into());
        }
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
                        structured_history: &self.structured_history,
                        observer,
                        handler_override: None,
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

    #[test]
    fn rig_thinking_levels_are_deferred_to_model_catalog_validation() {
        for provider in ["copilot", "github-copilot"] {
            let agent: Agent = serde_json::from_value(serde_json::json!({
                "id":"agent", "instructions":"role", "provider":"rig",
                "model":format!("{provider}/gpt-5.3-codex"), "skillIds":[], "thinkingLevel":"high"
            }))
            .unwrap();
            assert!(super::supports_thinking_level(&agent));
        }
        for (provider, model, level) in [
            ("copilot", "gpt-5.3-codex", "low"),
            ("openai", "gpt-5.3-codex", "high"),
            ("copilot", "other", "high"),
        ] {
            let agent: Agent = serde_json::from_value(serde_json::json!({
                "id":"agent", "instructions":"role", "provider":"rig",
                "model":format!("{provider}/{model}"), "skillIds":[], "thinkingLevel":level
            }))
            .unwrap();
            assert!(super::supports_thinking_level(&agent));
        }
    }

    #[cfg(unix)]
    #[test]
    fn configured_thinking_level_is_rejected_before_runtime_dispatch() {
        use std::os::unix::fs::PermissionsExt;
        for runtime_kind in [RuntimeKind::Rig, RuntimeKind::Fake] {
            let root = tempfile::tempdir().unwrap();
            let repository = Arc::new(FilesystemCredentialRepository::new(root.path().into()));
            let mut runtime = ConfiguredRuntime::new(repository, root.path().into(), vec![]);
            let marker = root.path().join("was-run");
            let pi_command = root.path().join("must-not-run");
            std::fs::write(
                &pi_command,
                format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
            )
            .unwrap();
            std::fs::set_permissions(&pi_command, std::fs::Permissions::from_mode(0o755)).unwrap();
            runtime.pi_command = pi_command.clone().into();
            let agent: Agent = serde_json::from_value(serde_json::json!({
                "id":"agent", "instructions":"role", "provider": runtime_kind,
                "skillIds":[], "thinkingLevel":"high"
            }))
            .unwrap();
            let error = runtime
                .execute_pipeline_step(root.path(), &agent, "prompt", &[], &|| false)
                .unwrap_err();
            assert!(error.contains("thinkingLevel is not supported"), "{error}");
            assert!(!marker.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn pi_provider_failures_retry_only_without_tools() {
        use std::os::unix::fs::PermissionsExt;
        for (diagnostic, tools, failures, expected_calls, expected_error) in [
            ("rate_limit_error", vec![], 1, 2, None),
            ("rate_limit_error", vec![], 5, 3, Some("after 2 retries")),
            (
                "rate_limit_error",
                vec!["shell"],
                5,
                1,
                Some("reconcile side effects"),
            ),
            (
                "content_policy_violation",
                vec![],
                5,
                1,
                Some("content policy"),
            ),
            (
                "authentication failed",
                vec![],
                5,
                1,
                Some("authentication failed"),
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let script = root.path().join("pi");
            let calls = root.path().join("calls");
            std::fs::write(&script, format!(r##"#!/bin/sh
while [ "$#" -gt 0 ]; do
  case "$1" in
    --session-dir) shift; dir="$1";;
    --session-id) shift; id="$1";;
    --session) shift; file="$1";;
  esac
  shift
done
cat >/dev/null
if [ -z "$file" ]; then
  file="$dir/test_$id.jsonl"
  printf '{{"type":"session","id":"%s","cwd":"%s"}}\n{{"type":"message","message":{{"role":"user"}}}}\n' "$id" "$PWD" > "$file"
fi
printf x >> '{}'
n=$(wc -c < '{}')
if [ "$n" -le {} ]; then
  printf '{{"type":"message","message":{{"role":"assistant","stopReason":"error","errorMessage":"{} private-secret"}}}}\n' >> "$file"
  exit 1
fi
printf '{{"type":"message","message":{{"role":"assistant","stopReason":"stop"}}}}\n' >> "$file"
printf done
"##, calls.display(), calls.display(), failures, diagnostic)).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            let vault = root.path().join("vault");
            let repository = Arc::new(FilesystemCredentialRepository::new(vault.clone()));
            let mut runtime = ConfiguredRuntime::new(repository, vault, vec![]);
            runtime.pi_command = script.into();
            let agent: Agent = serde_json::from_value(serde_json::json!({"id":"pi", "instructions":"role", "provider":"pi", "skillIds":[], "allowedTools":tools})).unwrap();
            let mut trace = vec![];
            let result = runtime.execute_pipeline_step_observed(
                root.path(),
                &agent,
                "private prompt",
                &[],
                &|| false,
                &mut |entry| {
                    trace.push(entry);
                    Ok(())
                },
            );
            assert_eq!(
                trace
                    .iter()
                    .filter(|entry| entry.kind == "runtimeRetry")
                    .count(),
                expected_calls - 1
            );
            match expected_error {
                Some(expected) => {
                    let error = result.unwrap_err();
                    assert!(error.contains(expected), "{error}");
                    assert!(!error.contains("private"));
                }
                None => assert_eq!(result.unwrap(), "done"),
            }
            assert_eq!(
                std::fs::read_to_string(calls).unwrap().len(),
                expected_calls
            );
        }
    }

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
        let args_path = root.path().join("args");
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
                args_path.display(),
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
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"pi", "instructions":"role", "provider":"pi", "model":"openai/gpt-4.1", "credentialId":null, "skillIds":[], "allowedTools":["read", "shell", "report_outcome"], "thinkingLevel":"high"})).unwrap();
        let large_prompt = format!("- {}", "x".repeat(256 * 1024));
        let result = runtime.execute(root.path(), &agent, &large_prompt);
        assert_eq!(result.unwrap(), "done");
        assert_eq!(std::fs::read_to_string(prompt).unwrap(), large_prompt);
        let args = std::fs::read_to_string(&args_path).unwrap();
        assert!(args.contains("--no-extensions") && args.contains("--no-mcp"));
        assert!(args.contains("--no-skills"));
        assert!(args.contains("--model\nopenai/gpt-4.1"));
        assert!(args.contains("--thinking\nhigh"));
        assert!(args.contains("--tools\nread,bash"));
        for level in ["off", "minimal", "low", "medium", "xhigh", "max"] {
            let mut agent = agent.clone();
            agent.thinking_level = Some(serde_json::from_value(serde_json::json!(level)).unwrap());
            let vault = root.path().join(format!("vault-{level}"));
            let mut level_runtime = ConfiguredRuntime::new(repository.clone(), vault, vec![]);
            level_runtime.pi_command = pi.clone().into();
            assert_eq!(
                level_runtime
                    .execute(root.path(), &agent, "prompt")
                    .unwrap(),
                "done"
            );
            assert!(
                std::fs::read_to_string(&args_path)
                    .unwrap()
                    .contains(&format!("--thinking\n{level}"))
            );
        }
        let mut unset_agent = agent.clone();
        unset_agent.thinking_level = None;
        let vault = root.path().join("vault-unset");
        let mut unset_runtime = ConfiguredRuntime::new(repository.clone(), vault, vec![]);
        unset_runtime.pi_command = pi.clone().into();
        assert_eq!(
            unset_runtime
                .execute(root.path(), &unset_agent, "prompt")
                .unwrap(),
            "done"
        );
        assert!(
            !std::fs::read_to_string(&args_path)
                .unwrap()
                .contains("--thinking")
        );
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
            "#!/bin/sh\nprintf 'No API key found for private-provider.\\n\\nLogin with pi auth login\\nSee /private/path/config.json\\n' >&2\nexit 7\n",
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
    fn pi_sequential_structured_report_is_optional_and_preserves_final_text() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let pi = root.path().join("pi");
        std::fs::write(&pi, r##"#!/bin/sh
args="$*"
session=""
id=""
file=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = '--session-dir' ]; then shift; session="$1"; fi
  if [ "$1" = '--session-id' ]; then shift; id="$1"; fi
  if [ "$1" = '--session' ]; then shift; file="$1"; fi
  shift
done
prompt=$(cat)
mkdir -p "$session"
printf '%s\\n' "$args" > "$session/args"
cp "$BATON_PI_HANDOFF_HISTORY_PATH" "$session/history-capture.json"
if [ -z "$file" ]; then file="$session/test_$id.jsonl"; fi
if [ -f "$file" ]; then
  printf '{"type":"message","message":{"role":"user","content":"%s"}}\n{"type":"message","message":{"role":"assistant"}}\n' "$prompt" >> "$file"
else
  printf '{"type":"session","id":"%s","cwd":"%s"}\n{"type":"message","message":{"role":"user","content":"%s"}}\n{"type":"message","message":{"role":"assistant"}}\n' "$id" "$PWD" "$prompt" > "$file"
fi
case "$prompt" in
  *structured-request*) printf '{"outcome":"completed","summary":"report summary","structured_handoff":{"version":1,"summary":"Structured","findings":[]}}' > "$BATON_PI_OUTCOME_FILE" ;;
esac
printf 'legacy final answer'
"##).unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let vault = root.path().join("vault");
        let repository = Arc::new(FilesystemCredentialRepository::new(vault.clone()));
        let mut runtime = ConfiguredRuntime::new(repository, vault, vec![]);
        runtime.pi_command = pi.into();
        runtime.set_run_id("sequential-test").unwrap();
        runtime.structured_history = vec![baton_core::domain::pipeline::StructuredHandoffAttempt {
            attempt: 7,
            step_id: "prior".into(),
            report: baton_core::domain::pipeline::StructuredHandoff {
                version: 1,
                summary: "Prior".into(),
                findings: vec![],
                verification: vec![],
                next_action: None,
            },
        }];
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "id":"writer", "instructions":"role", "provider":"pi", "model":"openai/test", "skillIds":[], "allowedTools":["report_outcome"]
        })).unwrap();
        let output = runtime
            .execute_pipeline_step(root.path(), &agent, "structured-request", &[], &|| false)
            .unwrap();
        assert_eq!(output, "legacy final answer");
        let captured =
            std::fs::read_to_string(root.path().join(
                "vault/runtime-sessions/sequential-test/agent-writer/pi/history-capture.json",
            ))
            .unwrap();
        assert!(captured.contains("Prior"));
        assert_eq!(
            runtime
                .take_decision()
                .unwrap()
                .structured_handoff
                .unwrap()
                .summary,
            "Structured"
        );
        let output = runtime
            .execute_pipeline_step(root.path(), &agent, "ordinary", &[], &|| false)
            .unwrap();
        assert_eq!(output, "legacy final answer");
        assert!(runtime.take_decision().is_none());

        runtime.set_run_id("history-only").unwrap();
        let restricted_agent: Agent = serde_json::from_value(serde_json::json!({
            "id":"writer", "instructions":"role", "provider":"pi", "model":"openai/test", "skillIds":[], "allowedTools":["read"]
        })).unwrap();
        runtime
            .execute_pipeline_step(root.path(), &restricted_agent, "ordinary", &[], &|| false)
            .unwrap();
        let args = std::fs::read_to_string(
            root.path()
                .join("vault/runtime-sessions/history-only/agent-writer/pi/args"),
        )
        .unwrap();
        assert!(args.contains("get_structured_handoff"));
        assert!(!args.contains("report_outcome"));
    }

    #[cfg(unix)]
    #[test]
    fn pi_resumes_per_agent_and_run_and_retries_missing_outcomes() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let pi = root.path().join("pi");
        std::fs::write(&pi, r##"#!/bin/sh
args="$*"
system_prompt=""
skills_flag="no"
while [ "$#" -gt 0 ]; do
  if [ "$1" = '--no-skills' ]; then skills_flag="yes"; fi
  if [ "$1" = '--system-prompt' ]; then shift; system_prompt="$1"; fi
  if [ "$1" = '--session-dir' ]; then shift; session="$1"; fi
  if [ "$1" = '--session-id' ]; then shift; id="$1"; fi
  if [ "$1" = '--session' ]; then shift; file="$1"; fi
  shift
done
prompt=$(cat)
call=$(find "$session" -name 'system-prompt-*' | wc -l | tr -d ' ')
printf '%s\n' "$args" > "$session/args-$call"
printf '%s\n' "$system_prompt" > "$session/system-prompt-$call"
printf '%s\n' "$skills_flag" > "$session/skills-flag-$call"
printf '%s\n' "$prompt" > "$session/invocation-prompt-$call"
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
        let agent: Agent = serde_json::from_value(serde_json::json!({"id":"first", "instructions":"role", "provider":"pi", "skillIds":["assigned"], "allowedTools":["report_outcome"], "thinkingLevel":"high"})).unwrap();
        let mut runtime = make_runtime();
        runtime.skills = vec![
            baton_core::domain::project::Skill {
                id: "assigned".into(),
                description: String::new(),
                metadata: String::new(),
                instructions: "assigned-only-instruction".into(),
            },
            baton_core::domain::project::Skill {
                id: "unassigned".into(),
                description: String::new(),
                metadata: String::new(),
                instructions: "unassigned-must-not-appear".into(),
            },
        ];
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
        let args = std::fs::read_to_string(first.join("args-0")).unwrap();
        assert!(args.contains("--no-skills"));
        let first_system_prompt = std::fs::read_to_string(first.join("system-prompt-0")).unwrap();
        assert!(first_system_prompt.contains("assigned-only-instruction"));
        assert!(!first_system_prompt.contains("unassigned-must-not-appear"));
        assert_eq!(
            std::fs::read_to_string(first.join("skills-flag-0"))
                .unwrap()
                .trim(),
            "yes"
        );
        assert!(prompts.contains("You did not call report_outcome"));
        let retry_prompt = std::fs::read_to_string(first.join("invocation-prompt-1")).unwrap();
        assert!(retry_prompt.contains("You did not call report_outcome"));
        let retry_system_prompt = std::fs::read_to_string(first.join("system-prompt-1")).unwrap();
        assert!(retry_system_prompt.contains("assigned-only-instruction"));
        assert!(!retry_system_prompt.contains("unassigned-must-not-appear"));
        assert_eq!(
            std::fs::read_to_string(first.join("skills-flag-1"))
                .unwrap()
                .trim(),
            "yes"
        );
        let session_path = std::fs::read_dir(&first)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .unwrap();
        let args = std::fs::read_to_string(first.join("args-0")).unwrap();
        assert!(args.contains("--thinking high"));
        assert!(args.contains("--session-id "));
        let retry_args = std::fs::read_to_string(first.join("args-1")).unwrap();
        assert!(retry_args.contains(&format!("--session {}", session_path.display())));
        let mut restarted = make_runtime();
        restarted.skills = runtime.skills.clone();
        restarted.set_run_id("run-one").unwrap();
        restarted.set_history_required(true);
        restarted
            .execute(root.path(), &agent, "new handoff")
            .unwrap();
        let resumed_prompts = std::fs::read_to_string(first.join("prompts")).unwrap();
        assert!(resumed_prompts.contains("new handoff"));
        let resumed_args = std::fs::read_to_string(first.join("args-2")).unwrap();
        assert!(resumed_args.contains("--thinking high"));
        assert!(resumed_args.contains(&format!("--session {}", session_path.display())));
        assert!(
            std::fs::read_to_string(first.join("invocation-prompt-2"))
                .unwrap()
                .contains("new handoff")
        );
        let resumed_system_prompt = std::fs::read_to_string(first.join("system-prompt-2")).unwrap();
        assert!(resumed_system_prompt.contains("assigned-only-instruction"));
        assert!(!resumed_system_prompt.contains("unassigned-must-not-appear"));
        assert_eq!(
            std::fs::read_to_string(first.join("skills-flag-2"))
                .unwrap()
                .trim(),
            "yes"
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
