//! Standalone Work composition: only the worker's repositories, checks, and mediator.
use baton_adapters::{
    repositories::{
        filesystem_work::{FilesystemWorkRepository, WorkProcessLock},
        yaml_project_config::YamlProjectConfigRepository as FilesystemProjectConfigRepository,
    },
    skills::FilesystemSkillRepository,
    work::ShellWorkChecks,
};
use baton_core::domain::work::{WorkCheck, WorkHumanAction, WorkSubmission};
use baton_core::ports::{runs::RunRepository, runtime::AgentRuntimeFactory};
use baton_core::{features::work::*, work_feature};
use medi_rs::mediator;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "camelCase", deny_unknown_fields)]
pub enum WorkCommand {
    Start {
        task: String,
        source: Option<String>,
        pipeline: Option<String>,
        checks: Vec<WorkCheck>,
    },
    Refine {
        id: String,
        instruction: String,
    },
    Next {
        id: String,
    },
    Execute {
        id: String,
    },
    Resume {
        id: String,
    },
    Status {
        id: Option<String>,
    },
    Submit {
        id: String,
        result: WorkSubmission,
    },
    Respond {
        id: String,
        action: WorkHumanAction,
    },
    Cancel {
        id: String,
    },
}

mediator! {
    pub struct WorkMediator {
        event_queue_capacity: 1;
        event_workers: 1;
        modules: [work_feature];
    }
}

/// Compact output for mutations; full records are available only through explicit status reads.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkProgress {
    id: String,
    status: baton_core::domain::pipeline::Status,
    revision: u64,
    current_step: Option<String>,
    attempt: Option<u32>,
}
impl From<baton_core::domain::pipeline::PipelineRun> for WorkProgress {
    fn from(run: baton_core::domain::pipeline::PipelineRun) -> Self {
        Self {
            revision: run.session.as_ref().map_or(0, |s| s.revision),
            attempt: run.steps.last().map(|s| s.attempt),
            id: run.id,
            status: run.status,
            current_step: run.current_step,
        }
    }
}

enum StepFailure {
    Runtime(String),
    InvalidOutcome {
        message: String,
        trace: Vec<baton_core::domain::pipeline::RunTraceEntry>,
    },
}
impl From<String> for StepFailure {
    fn from(error: String) -> Self {
        Self::Runtime(error)
    }
}
impl From<&str> for StepFailure {
    fn from(error: &str) -> Self {
        Self::Runtime(error.to_owned())
    }
}

pub struct WorkApplication {
    mediator: WorkMediator,
    workspace: PathBuf,
    lock: Mutex<Option<WorkProcessLock>>,
}
fn append_observation_event(
    events: &mut Vec<baton_core::domain::pipeline::RunEvent>,
    event: baton_core::domain::pipeline::RunEvent,
) -> Option<baton_core::domain::pipeline::RunEvent> {
    if events.len() < 512 {
        events.push(event.clone());
        return Some(event);
    }
    mark_observation_event_limit(events)
}

fn mark_observation_event_limit(
    events: &mut [baton_core::domain::pipeline::RunEvent],
) -> Option<baton_core::domain::pipeline::RunEvent> {
    let event = events
        .iter_mut()
        .find(|event| event.message.as_deref() == Some("Observed runtime activity"))?;
    event.message = Some("Runtime activity limit reached; later observations omitted".into());
    Some(event.clone())
}

impl WorkApplication {
    pub fn new(workspace: PathBuf) -> Result<Self, String> {
        let workspace = workspace.canonicalize().map_err(|e| e.to_string())?;
        let lock = WorkProcessLock::acquire(&workspace)?;
        let resources = WorkResources {
            config: Arc::new(FilesystemProjectConfigRepository::new(workspace.clone())),
            runs: Arc::new(FilesystemWorkRepository(workspace.clone())),
            skills: Arc::new(FilesystemSkillRepository::new(workspace.clone())),
            checks: Arc::new(ShellWorkChecks),
            workspace: workspace.clone(),
        };
        Ok(Self {
            mediator: WorkMediator::builder().work_resources(resources).build(),
            workspace,
            lock: Mutex::new(Some(lock)),
        })
    }
    pub async fn execute(&self, command: WorkCommand) -> Result<serde_json::Value, String> {
        let value = match command {
            WorkCommand::Start {
                task,
                source,
                pipeline,
                checks,
            } => serde_json::to_value(WorkProgress::from(
                self.mediator
                    .send(StartWork {
                        id: uuid::Uuid::now_v7().to_string(),
                        task,
                        source,
                        pipeline_id: pipeline,
                        checks,
                    })
                    .await?,
            )),
            WorkCommand::Refine { id, instruction } => {
                let new_id = uuid::Uuid::now_v7().to_string();
                serde_json::to_value(WorkProgress::from(
                    self.mediator
                        .send(RefineWork {
                            id,
                            new_id,
                            instruction,
                        })
                        .await?,
                ))
            }
            WorkCommand::Next { id } => {
                if let Ok(_execution_lock) = WorkProcessLock::acquire_execution(&self.workspace) {
                    self.mediator.send(RecoverWork { id: id.clone() }).await?;
                }
                serde_json::to_value(self.mediator.send(NextWork { id }).await?)
            }
            WorkCommand::Execute { id } => {
                let item = self.mediator.send(NextWork { id: id.clone() }).await?;
                let run = self.mediator.send(GetWork { id: id.clone() }).await?;
                let step = item.step.ok_or("No active step")?;
                if step.is_human() {
                    return Err("Human steps require an explicit response".into());
                }
                let agent = item.agent;
                let attempt = item.attempt.ok_or("Missing attempt")?;
                let config = run
                    .config_snapshot
                    .as_ref()
                    .ok_or("Missing configuration snapshot")?;
                let pipeline = config
                    .pipelines
                    .iter()
                    .find(|p| p.id == run.pipeline_id)
                    .ok_or("Missing pipeline")?;
                if baton_core::features::pipeline_engine::validate_handoff(&run).is_err() {
                    self.mediator
                        .send(FailWork {
                            id: id.clone(),
                            revision: item.revision,
                            step_id: step.id.clone(),
                            attempt,
                            input: String::new(),
                            trace: vec![],
                            failure: WorkFailure::InvalidHandoff,
                            failure_detail: None,
                        })
                        .await?;
                    return Err(WorkFailure::InvalidHandoff.message().into());
                }
                let input = baton_core::features::pipeline_engine::with_step_prompt(
                    baton_core::features::pipeline_engine::step_input(&run, pipeline)?,
                    &step,
                );
                let recorded_input = input.clone();
                let setup = (|| {
                    let store =
                        baton_adapters::credentials::FilesystemCredentialRepository::for_project(
                            &self.workspace,
                        )?;
                    let auth_root = store.root_path().to_path_buf();
                    let factory = baton_adapters::runtime::ConfiguredRuntimeFactory {
                        project_dir: self.workspace.clone(),
                        credentials: Arc::new(store),
                        auth_root,
                    };
                    let mut runtime = factory.create(config, &run.pipeline_id)?;
                    let conversation_id = run
                        .session
                        .as_ref()
                        .and_then(|session| session.conversation_id.as_deref())
                        .unwrap_or(&run.id);
                    runtime.set_run_id(conversation_id)?;
                    Ok::<_, String>(runtime)
                })();
                let mut runtime = match setup {
                    Ok(runtime) => runtime,
                    Err(_) => {
                        self.mediator
                            .send(FailWork {
                                id: id.clone(),
                                revision: item.revision,
                                step_id: step.id.clone(),
                                attempt,
                                input: recorded_input,
                                trace: vec![],
                                failure: WorkFailure::Runtime,
                                failure_detail: None,
                            })
                            .await?;
                        return Err(WorkFailure::Runtime.message().into());
                    }
                };
                let history_unavailable = if let Some(agent) = &agent {
                    let required = run
                        .session
                        .as_ref()
                        .is_some_and(|session| session.conversation_agents.contains(&agent.id))
                        || run.steps.iter().any(|attempt| {
                            attempt.agent_id == agent.id && attempt.output.is_some()
                        });
                    runtime.set_history_required(required);
                    required && !runtime.supports_session_resumption(agent)
                } else {
                    false
                };
                let workspace = self.workspace.clone();
                let allowed = if step.transitions.is_empty() {
                    vec![]
                } else {
                    item.allowed_outcomes
                };
                let command = step.command().map(str::to_owned);
                let timeout = step.timeout_seconds().unwrap_or(300);
                let execution_lock = WorkProcessLock::acquire_execution(&self.workspace)?;
                let recovered = self.mediator.send(RecoverWork { id: id.clone() }).await?;
                if recovered.status != baton_core::domain::pipeline::Status::Running {
                    return Err("Work execution was interrupted; inspect side effects and resume explicitly".into());
                }
                self.mediator
                    .send(BeginWork {
                        id: id.clone(),
                        revision: item.revision,
                        attempt,
                    })
                    .await?;
                self.lock.lock().map_err(|e| e.to_string())?.take();
                let repository = FilesystemWorkRepository(self.workspace.clone());
                let cancellation_id = id.clone();
                let observation_repository = FilesystemWorkRepository(self.workspace.clone());
                let observation_step = step.id.clone();
                let execution_step = step.clone();
                let execution = tokio::task::spawn_blocking(move || -> Result<_, StepFailure> {
                    let cancelled = || {
                        repository
                            .load(&cancellation_id)
                            .map(|run| {
                                run.status == baton_core::domain::pipeline::Status::Cancelled
                            })
                            .unwrap_or(true)
                    };
                    if let Some(command) = command {
                        let result =
                            runtime.execute_shell(&workspace, &command, timeout, &cancelled)?;
                        let outcome = if result.exit_code == Some(0) {
                            baton_core::domain::project::Outcome::Completed
                        } else {
                            baton_core::domain::project::Outcome::Failed
                        };
                        let summary = format!(
                            "Exit status: {}\n{}",
                            result
                                .exit_code
                                .map_or_else(|| "none".into(), |code| code.to_string()),
                            result.output
                        );
                        if summary.len() > baton_core::ports::runtime::MAX_HANDOFF_BYTES {
                            return Err(StepFailure::Runtime(
                                "Shell handoff exceeds 16384 bytes".into(),
                            ));
                        }
                        return Ok((outcome, summary, None, vec![]));
                    }
                    let agent = agent.ok_or("Current step has no configured agent")?;
                    if history_unavailable {
                        return Err(StepFailure::Runtime(
                            "Runtime does not support resuming this agent's session".into(),
                        ));
                    }
                    let mut observer = |mut entry: baton_core::domain::pipeline::RunTraceEntry| {
                        let _lease =
                            baton_core::features::pipeline_engine::acquire_action_lease_retry(
                                &observation_repository,
                                &cancellation_id,
                                &cancelled,
                            )?;
                        let mut run = observation_repository.load(&cancellation_id)?;
                        if run.status != baton_core::domain::pipeline::Status::Running
                            || run.current_step.as_deref() != Some(&observation_step)
                            || run.steps.last().is_none_or(|step| {
                                step.attempt != attempt
                                    || step.status != baton_core::domain::pipeline::Status::Running
                            })
                        {
                            return Err("Work attempt changed while recording activity".into());
                        }
                        entry.attempt = Some(attempt);
                        let occurred_at = entry.occurred_at.clone();
                        baton_core::features::pipeline_engine::append_bounded_trace(
                            &mut run,
                            [entry],
                        );
                        let event = baton_core::domain::pipeline::RunEvent {
                            run_id: run.id.clone(),
                            status: run.status.clone(),
                            occurred_at,
                            message: Some("Observed runtime activity".into()),
                        };
                        let published = append_observation_event(&mut run.events, event);
                        observation_repository.save(&run)?;
                        if let Some(event) = published {
                            observation_repository.publish(&event)
                        } else {
                            Ok(())
                        }
                    };
                    runtime.set_structured_handoff_required(true);
                    let mut runtime_input = input.clone();
                    let mut report_retries = 0;
                    let mut trace = Vec::new();
                    loop {
                        let output = runtime.execute_pipeline_step_observed(
                            &workspace,
                            &agent,
                            &runtime_input,
                            &allowed,
                            &cancelled,
                            &mut observer,
                        )?;
                        trace.extend(runtime.take_trace());
                        match baton_core::features::pipeline_engine::validate_agent_response(
                            &execution_step,
                            &allowed,
                            output,
                            runtime.take_decision(),
                        ) {
                            Ok((outcome, summary, structured_handoff)) => {
                                break Ok((outcome, summary, structured_handoff, trace));
                            }
                            Err(error) if report_retries < 2 => {
                                report_retries += 1;
                                runtime_input =
                                    baton_core::features::pipeline_engine::report_repair_prompt(
                                        &error, &allowed,
                                    );
                            }
                            Err(error) => {
                                break Err(StepFailure::InvalidOutcome {
                                    message: error,
                                    trace,
                                });
                            }
                        }
                    }
                })
                .await
                .map_err(|e| StepFailure::Runtime(e.to_string()));
                let workspace = self.workspace.clone();
                let lock =
                    tokio::task::spawn_blocking(move || WorkProcessLock::acquire_wait(&workspace))
                        .await
                        .map_err(|e| e.to_string())??;
                *self.lock.lock().map_err(|e| e.to_string())? = Some(lock);
                let current = self.mediator.send(GetWork { id: id.clone() }).await?;
                if current.status == baton_core::domain::pipeline::Status::Cancelled {
                    drop(execution_lock);
                    return serde_json::to_value(WorkProgress::from(current))
                        .map_err(|e| e.to_string());
                }
                let (outcome, summary, structured_handoff, trace) = match execution
                    .and_then(|result| result)
                {
                    Ok(result) => result,
                    Err(StepFailure::InvalidOutcome { message, trace }) => {
                        self.mediator
                            .send(FailWork {
                                id: id.clone(),
                                revision: item.revision,
                                step_id: step.id.clone(),
                                attempt,
                                input: recorded_input,
                                trace,
                                failure: WorkFailure::InvalidOutcome,
                                failure_detail: None,
                            })
                            .await?;
                        return Err(message);
                    }
                    Err(StepFailure::Runtime(error)) => {
                        let failure = if error.contains("reconcile side effects")
                            && error.contains("baton-inflight")
                        {
                            WorkFailure::Interrupted
                        } else if error.contains("session is missing") {
                            WorkFailure::MissingSession
                        } else if error
                            == "Pi CLI could not start after 2 retries; system resources are temporarily unavailable"
                        {
                            WorkFailure::ResourceUnavailable
                        } else if error.starts_with("Pi provider is temporarily unavailable") {
                            WorkFailure::ProviderTransient
                        } else if error == WorkFailure::ProviderPolicy.message() {
                            WorkFailure::ProviderPolicy
                        } else if error == WorkFailure::ProviderAuthentication.message()
                            || (error.starts_with("Pi CLI failed with status ")
                                && (error.contains(": authentication failed")
                                    || error.contains(": provider API key is not configured")))
                        {
                            WorkFailure::ProviderAuthentication
                        } else if error.starts_with("Pi CLI failed with status ")
                            && (error.contains(": unsupported CLI option")
                                || error.contains(": configured model was not found"))
                        {
                            WorkFailure::ProviderConfiguration
                        } else {
                            WorkFailure::Runtime
                        };
                        self.mediator
                            .send(FailWork {
                                id: id.clone(),
                                revision: item.revision,
                                step_id: step.id.clone(),
                                attempt,
                                input: recorded_input,
                                trace: vec![],
                                failure,
                                failure_detail: step
                                    .command()
                                    .map(|_| format!("Shell step '{}' failed: {error}", step.id)),
                            })
                            .await?;
                        return Err(step.command().map_or_else(
                            || failure.message().to_owned(),
                            |_| format!("Shell step '{}' failed: {error}", step.id),
                        ));
                    }
                };
                let result = serde_json::to_value(WorkProgress::from(
                    self.mediator
                        .send(SubmitWork {
                            id,
                            result: WorkSubmission {
                                revision: item.revision,
                                step_id: step.id,
                                attempt,
                                outcome,
                                summary,
                                structured_handoff,
                            },
                            input: Some(recorded_input),
                            trace,
                        })
                        .await?,
                ));
                drop(execution_lock);
                result
            }
            WorkCommand::Resume { id } => {
                let _execution_lock = WorkProcessLock::acquire_execution(&self.workspace)?;
                serde_json::to_value(WorkProgress::from(
                    self.mediator.send(ResumeWork { id }).await?,
                ))
            }
            WorkCommand::Status { id: None } => {
                let mut runs = self.mediator.send(ListWork).await?;
                if let Ok(_execution_lock) = WorkProcessLock::acquire_execution(&self.workspace) {
                    for run in &mut runs {
                        *run = self
                            .mediator
                            .send(RecoverWork { id: run.id.clone() })
                            .await?;
                    }
                }
                serde_json::to_value(runs.into_iter().map(WorkProgress::from).collect::<Vec<_>>())
            }
            WorkCommand::Status { id: Some(id) } => {
                if let Ok(_execution_lock) = WorkProcessLock::acquire_execution(&self.workspace) {
                    self.mediator.send(RecoverWork { id: id.clone() }).await?;
                }
                serde_json::to_value(self.mediator.send(GetWork { id }).await?)
            }
            WorkCommand::Submit { id, result } => {
                let _execution_lock = WorkProcessLock::acquire_execution(&self.workspace)?;
                serde_json::to_value(WorkProgress::from(
                    self.mediator
                        .send(SubmitWork {
                            id,
                            result,
                            input: None,
                            trace: vec![],
                        })
                        .await?,
                ))
            }
            WorkCommand::Respond { id, action } => serde_json::to_value(WorkProgress::from(
                self.mediator.send(RespondWork { id, action }).await?,
            )),
            WorkCommand::Cancel { id } => serde_json::to_value(WorkProgress::from(
                self.mediator.send(CancelWork { id }).await?,
            )),
        };
        value.map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::append_observation_event;
    use baton_core::domain::pipeline::{RunEvent, Status};

    #[test]
    fn observation_overflow_marks_only_observation_and_preserves_event_count() {
        let mut events = (0..511)
            .map(|index| RunEvent {
                run_id: "r".into(),
                status: Status::Running,
                occurred_at: "now".into(),
                message: Some(format!("Lifecycle event {index}")),
            })
            .collect::<Vec<_>>();
        events.push(RunEvent {
            run_id: "r".into(),
            status: Status::Running,
            occurred_at: "now".into(),
            message: Some("Observed runtime activity".into()),
        });
        let count = events.len();
        let marked = append_observation_event(
            &mut events,
            RunEvent {
                run_id: "r".into(),
                status: Status::Running,
                occurred_at: "later".into(),
                message: Some("Observed runtime activity".into()),
            },
        )
        .unwrap();
        assert_eq!(events.len(), count);
        assert_eq!(events[0].message.as_deref(), Some("Lifecycle event 0"));
        assert_eq!(events.len(), 512);
        assert_eq!(
            marked.message.as_deref(),
            Some("Runtime activity limit reached; later observations omitted")
        );
        assert!(
            append_observation_event(
                &mut events,
                RunEvent {
                    run_id: "r".into(),
                    status: Status::Running,
                    occurred_at: "later".into(),
                    message: Some("Observed runtime activity".into()),
                },
            )
            .is_none()
        );
    }
}
