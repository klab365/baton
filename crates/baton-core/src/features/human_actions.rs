use crate::{
    domain::{
        pipeline::{PipelineRun, Status, transitions::route},
        project::Outcome,
    },
    features::pipeline_engine::{execute_graph, publish},
    ports::{
        runs::RunRepository,
        runtime::{AgentRuntime, MAX_HANDOFF_BYTES},
    },
};

/// Explicit user action only. Invalid actions do not consume or mutate the wait.
pub fn resume_human(
    id: &str,
    outcome: Outcome,
    response: &str,
    runtime: &mut dyn AgentRuntime,
    repository: &dyn RunRepository,
) -> Result<PipelineRun, String> {
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
    if !step.human {
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
    execute_graph(&config, run, runtime, repository)
}
