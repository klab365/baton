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
        #[serde(rename = "maxAttempts")]
        max_attempts: u32,
        checks: Vec<WorkCheck>,
    },
    Next {
        id: String,
    },
    Execute {
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
        message: &'static str,
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
                max_attempts,
                checks,
            } => serde_json::to_value(WorkProgress::from(
                self.mediator
                    .send(StartWork {
                        id: uuid::Uuid::now_v7().to_string(),
                        task,
                        source,
                        pipeline_id: pipeline,
                        max_attempts,
                        checks,
                    })
                    .await?,
            )),
            WorkCommand::Next { id } => {
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
                let input = baton_core::features::pipeline_engine::step_input(&run, pipeline);
                let recorded_input = input.clone();
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
                runtime.set_run_id(&run.id)?;
                if let Some(agent) = &agent {
                    runtime.set_history_required(
                        run.steps.iter().any(|attempt| {
                            attempt.agent_id == agent.id && attempt.output.is_some()
                        }),
                    );
                }
                let workspace = self.workspace.clone();
                let allowed = if step.transitions.is_empty() {
                    vec![]
                } else {
                    item.allowed_outcomes
                };
                let command = step.command().map(str::to_owned);
                let timeout = step.timeout_seconds().unwrap_or(300);
                let execution_lock = WorkProcessLock::acquire_execution(&self.workspace)?;
                self.lock.lock().map_err(|e| e.to_string())?.take();
                let repository = FilesystemWorkRepository(self.workspace.clone());
                let cancellation_id = id.clone();
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
                        let mut summary = format!(
                            "Exit status: {}\n{}",
                            result
                                .exit_code
                                .map_or_else(|| "none".into(), |code| code.to_string()),
                            result.output
                        );
                        let mut end = summary
                            .len()
                            .min(baton_core::ports::runtime::MAX_HANDOFF_BYTES);
                        while !summary.is_char_boundary(end) {
                            end -= 1;
                        }
                        summary.truncate(end);
                        return Ok((outcome, summary, vec![]));
                    }
                    let agent = agent.ok_or("Current step has no configured agent")?;
                    let output = runtime
                        .execute_pipeline_step(&workspace, &agent, &input, &allowed, &cancelled)?;
                    let trace = runtime.take_trace();
                    if allowed.is_empty() {
                        Ok((
                            baton_core::domain::project::Outcome::Completed,
                            output,
                            trace,
                        ))
                    } else {
                        let Some(decision) = runtime.take_decision() else {
                            return Err(StepFailure::InvalidOutcome {
                                message: "Agent completed without reporting a pipeline outcome",
                                trace,
                            });
                        };
                        if !allowed.contains(&decision.outcome) {
                            return Err(StepFailure::InvalidOutcome {
                                message: "Agent reported an invalid outcome",
                                trace,
                            });
                        }
                        Ok((decision.outcome, decision.summary, trace))
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
                let (outcome, summary, trace) = match execution.and_then(|result| result) {
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
                            })
                            .await?;
                        return Err(message.into());
                    }
                    Err(StepFailure::Runtime(error)) => return Err(error),
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
                            },
                            input: Some(recorded_input),
                            trace,
                        })
                        .await?,
                ));
                drop(execution_lock);
                result
            }
            WorkCommand::Status { id: None } => serde_json::to_value(
                self.mediator
                    .send(ListWork)
                    .await?
                    .into_iter()
                    .map(WorkProgress::from)
                    .collect::<Vec<_>>(),
            ),
            WorkCommand::Status { id: Some(id) } => {
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
