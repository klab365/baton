use crate::{
    domain::{
        pipeline::{PipelineRun, Status, transitions::route},
        project::{Outcome, ProjectConfig},
    },
    features::pipeline_engine::{elapsed_ms, execute_graph, publish, utc_now},
    ports::{
        runs::RunRepository,
        runtime::{AgentRuntime, MAX_HANDOFF_BYTES},
    },
};

/// Validate and persist an explicit human decision before releasing the action lease.
pub fn prepare_human_resume(
    id: &str,
    outcome: Outcome,
    response: &str,
    repository: &dyn RunRepository,
) -> Result<(ProjectConfig, PipelineRun), String> {
    let mut run = repository.load(id)?;
    if run.status != Status::WaitingHuman {
        return Err("Run is not waiting for a human".into());
    }
    if !matches!(
        outcome,
        Outcome::Approved | Outcome::Rejected | Outcome::Responded
    ) {
        return Err("Expected approved, rejected, or responded human action".into());
    }
    if response.len() > MAX_HANDOFF_BYTES {
        return Err("Response exceeds 16384 bytes".into());
    }
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
    if !step.is_human() {
        return Err("Current step is not human".into());
    }
    if !step.transitions.is_empty() && !step.transitions.iter().any(|edge| edge.outcome == outcome)
    {
        return Err("No configured transition for human action".into());
    }
    let workspace = run.worktree.as_ref().ok_or("Missing isolated worktree")?;
    repository
        .worktree_manager()?
        .validate(&workspace.owner, workspace)?;
    let index = run
        .steps
        .iter()
        .rposition(|attempt| attempt.status == Status::WaitingHuman)
        .ok_or("Missing human attempt")?;
    let ended_at = utc_now();
    run.steps[index].ended_at = Some(ended_at.clone());
    run.steps[index].duration_ms = run.steps[index]
        .started_at
        .as_deref()
        .and_then(|started| elapsed_ms(started, &ended_at));
    run.handoff = Some(response.into());
    run.steps[index].output = Some(response.into());
    run.steps[index].outcome = Some(outcome.clone());
    run.steps[index].status = Status::Succeeded;
    run.status = Status::Running;
    run.pending_actions.clear();
    route(&mut run, pipeline, index, outcome)?;
    publish(
        repository,
        &mut run,
        Some("Explicit human action received".into()),
    )?;
    Ok((config, run))
}

/// Explicit user action only. Invalid actions do not consume or mutate the wait.
pub fn resume_human(
    id: &str,
    outcome: Outcome,
    response: &str,
    runtime: &mut dyn AgentRuntime,
    repository: &dyn RunRepository,
) -> Result<PipelineRun, String> {
    let (config, run) = prepare_human_resume(id, outcome, response, repository)?;
    execute_graph(&config, run, runtime, repository)
}
