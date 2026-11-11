use crate::{
    domain::{
        pipeline::{PipelineRun, RunEvent, Status},
        project::Outcome,
    },
    features::{
        human_actions::resume_human,
        pipeline_engine::{execute_graph, publish, utc_now},
    },
    ports::{runs::RunRepository, runtime::AgentRuntimeFactory, tasks::TaskRepository},
};
use medi_rs::{MediCommand, medi_handler, medi_module};
use std::sync::Arc;

#[derive(Clone)]
pub struct RunFeatureResources {
    pub runs: Arc<dyn RunRepository>,
    pub tasks: Arc<dyn TaskRepository>,
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
    cancel_run(resources.runs.as_ref(), &command.id)
}

/// Shared cancellation policy for managed runs and externally executed work.
pub(crate) fn cancel_run(repository: &dyn RunRepository, id: &str) -> Result<PipelineRun, String> {
    let initial = repository.load(id)?;
    let _lease = if initial.session.is_some() {
        Some(repository.acquire_action_lease(id)?)
    } else {
        None
    };
    let mut run = repository.load(id)?;
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
    repository.save(&run)?;
    repository.publish(&event)?;
    Ok(run)
}

/// Resume a server-interrupted agent step in its existing worktree.
pub fn resume_interrupted_run(
    id: &str,
    runs: &dyn RunRepository,
    tasks: &dyn TaskRepository,
    factory: &dyn AgentRuntimeFactory,
) -> Result<PipelineRun, String> {
    let _lease = runs.acquire_action_lease(id)?;
    let mut run = runs.load(id)?;
    if run.status != Status::Failed
        || !run.events.last().is_some_and(|event| {
            event.message.as_deref() == Some("Run interrupted by server restart")
        })
    {
        return Err("Run was not interrupted by server restart".into());
    }
    let _task_lease = run
        .task_id
        .as_deref()
        .map(|id| tasks.acquire_operation_lease(id))
        .transpose()?;
    let config = run
        .config_snapshot
        .clone()
        .ok_or("Run has no configuration snapshot")?;
    config.validate()?;
    let pipeline = config
        .pipelines
        .iter()
        .find(|pipeline| pipeline.id == run.pipeline_id)
        .ok_or("Unknown pipeline")?;
    let step = pipeline
        .steps
        .iter()
        .find(|step| Some(&step.id) == run.current_step.as_ref())
        .ok_or("Unknown current step")?;
    if step.is_human() || step.command().is_some() {
        return Err("Interrupted human or shell step requires manual reconciliation".into());
    }
    let workspace = run.worktree.as_ref().ok_or("Missing isolated worktree")?;
    runs.worktree_manager()?
        .validate(&workspace.owner, workspace)?;
    let mut runtime = factory.create(&config, &run.pipeline_id)?;
    let note = "Previous attempt was interrupted by server restart. Inspect the existing worktree and reconcile any potentially completed external actions (including commits, pushes and pull requests) before continuing; do not repeat side effects blindly.";
    if let Some(attempt) = run
        .steps
        .iter_mut()
        .rev()
        .find(|attempt| attempt.status == Status::Running)
    {
        attempt.status = Status::Failed;
        attempt.error =
            Some("Interrupted by server restart; outcome and side effects are unknown".into());
        attempt.output = Some(note.into());
    }
    run.handoff = Some(note.into());
    run.status = Status::Running;
    run.ended_at = None;
    publish(
        runs,
        &mut run,
        Some("Resuming interrupted agent step in existing worktree".into()),
    )?;
    let run = execute_graph(&config, run, runtime.as_mut(), runs)?;
    if run.status == Status::Succeeded
        && let Some(id) = &run.task_id
    {
        let mut task = tasks.load(id)?;
        task.status = crate::domain::tasks::TaskStatus::Done;
        task.updated_at = utc_now().max(task.updated_at.clone());
        tasks.save(&task)?;
    }
    Ok(run)
}

#[medi_handler]
async fn respond_to_run_handler(
    resources: RunFeatureResources,
    command: RespondToRun,
) -> Result<PipelineRun, String> {
    let lease = resources.runs.acquire_action_lease(&command.id)?;
    let run = resources.runs.load(&command.id)?;
    let task_lease = run
        .task_id
        .as_deref()
        .map(|id| resources.tasks.acquire_operation_lease(id))
        .transpose()?;
    let config = run
        .config_snapshot
        .clone()
        .ok_or("Run has no configuration snapshot")?;
    let mut runtime = resources
        .runtime_factory
        .create(&config, &run.pipeline_id)?;
    let runs = resources.runs;
    let tasks = resources.tasks;
    tokio::task::spawn_blocking(move || {
        let _lease = lease;
        let _task_lease = task_lease;
        let run = resume_human(
            &command.id,
            command.outcome,
            &command.response,
            runtime.as_mut(),
            runs.as_ref(),
        )?;
        if run.status == Status::Succeeded
            && let Some(id) = &run.task_id
        {
            let mut task = tasks.load(id)?;
            task.status = crate::domain::tasks::TaskStatus::Done;
            task.updated_at = utc_now().max(task.updated_at.clone());
            tasks.save(&task)?;
        }
        Ok(run)
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
