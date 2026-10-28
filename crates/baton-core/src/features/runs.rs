use crate::{
    domain::{
        pipeline::{PipelineRun, RunEvent, Status},
        project::Outcome,
    },
    features::{human_actions::resume_human, pipeline_engine::utc_now},
    ports::{runs::RunRepository, runtime::AgentRuntimeFactory},
};
use medi_rs::{MediCommand, medi_handler, medi_module};
use std::sync::Arc;

#[derive(Clone)]
pub struct RunFeatureResources {
    pub runs: Arc<dyn RunRepository>,
    pub runtime_factory: Arc<dyn AgentRuntimeFactory>,
}

#[derive(MediCommand)]
#[medi_command(return_type = Vec<PipelineRun>, error_type = String)]
pub struct ListRuns;

#[derive(MediCommand)]
#[medi_command(return_type = Vec<PipelineRun>, error_type = String)]
pub struct ListTaskRuns {
    pub task_id: String,
}

#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct GetRun {
    pub id: String,
}

#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct CancelRun {
    pub id: String,
}

#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct RespondToRun {
    pub id: String,
    pub outcome: Outcome,
    pub response: String,
}

#[medi_handler]
async fn list_runs_handler(
    resources: RunFeatureResources,
    _command: ListRuns,
) -> Result<Vec<PipelineRun>, String> {
    resources.runs.list()
}

#[medi_handler]
async fn list_task_runs_handler(
    resources: RunFeatureResources,
    command: ListTaskRuns,
) -> Result<Vec<PipelineRun>, String> {
    Ok(resources
        .runs
        .list()?
        .into_iter()
        .filter(|run| run.task_id.as_deref() == Some(&command.task_id))
        .collect())
}

#[medi_handler]
async fn get_run_handler(
    resources: RunFeatureResources,
    command: GetRun,
) -> Result<PipelineRun, String> {
    resources.runs.load(&command.id)
}

#[medi_handler]
async fn cancel_run_handler(
    resources: RunFeatureResources,
    command: CancelRun,
) -> Result<PipelineRun, String> {
    let mut run = resources.runs.load(&command.id)?;
    if !matches!(run.status, Status::Running | Status::WaitingHuman) {
        return Err("Only running or waiting runs can be cancelled".into());
    }
    run.status = Status::Cancelled;
    run.pending_actions.clear();
    for step in &mut run.steps {
        if matches!(step.status, Status::Running | Status::WaitingHuman) {
            step.status = Status::Cancelled;
        }
    }
    let now = utc_now();
    run.ended_at = Some(now.clone());
    run.duration_ms = None;
    let event = RunEvent {
        run_id: run.id.clone(),
        status: Status::Cancelled,
        occurred_at: now,
        message: Some("Cancellation requested".into()),
    };
    run.events.push(event.clone());
    resources.runs.save(&run)?;
    resources.runs.publish(&event)?;
    Ok(run)
}

#[medi_handler]
async fn respond_to_run_handler(
    resources: RunFeatureResources,
    command: RespondToRun,
) -> Result<PipelineRun, String> {
    let lease = resources.runs.acquire_action_lease(&command.id)?;
    let run = resources.runs.load(&command.id)?;
    let config = run
        .config_snapshot
        .clone()
        .ok_or("Run has no configuration snapshot")?;
    let mut runtime = resources
        .runtime_factory
        .create(&config, &run.pipeline_id)?;
    let runs = resources.runs;
    tokio::task::spawn_blocking(move || {
        let _lease = lease;
        resume_human(
            &command.id,
            command.outcome,
            &command.response,
            runtime.as_mut(),
            runs.as_ref(),
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

medi_module! {
    pub manifest runs_feature;
    resources { run_resources: crate::features::runs::RunFeatureResources; }
    commands {
        crate::features::runs::ListRuns => crate::features::runs::list_runs_handler;
        crate::features::runs::ListTaskRuns => crate::features::runs::list_task_runs_handler;
        crate::features::runs::GetRun => crate::features::runs::get_run_handler;
        crate::features::runs::CancelRun => crate::features::runs::cancel_run_handler;
        crate::features::runs::RespondToRun => crate::features::runs::respond_to_run_handler;
    }
}
