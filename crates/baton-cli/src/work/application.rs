//! Standalone Work composition: only the worker's repositories, checks, and mediator.
use baton_adapters::{
    repositories::{
        filesystem_project_config::FilesystemProjectConfigRepository,
        filesystem_work::{FilesystemWorkRepository, WorkProcessLock},
    },
    skills::FilesystemSkillRepository,
    work::ShellWorkChecks,
};
use baton_core::domain::work::{WorkCheck, WorkHumanAction, WorkSubmission};
use baton_core::{features::work::*, work_feature};
use medi_rs::mediator;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

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

pub struct WorkApplication {
    mediator: WorkMediator,
    _lock: WorkProcessLock,
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
            workspace,
        };
        Ok(Self {
            mediator: WorkMediator::builder().work_resources(resources).build(),
            _lock: lock,
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
            WorkCommand::Submit { id, result } => serde_json::to_value(WorkProgress::from(
                self.mediator.send(SubmitWork { id, result }).await?,
            )),
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
