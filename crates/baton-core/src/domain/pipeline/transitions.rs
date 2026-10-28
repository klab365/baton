use super::model::{PipelineRun, Status};
use crate::domain::project::{Outcome, Pipeline};

pub(crate) fn route(
    run: &mut PipelineRun,
    pipeline: &Pipeline,
    index: usize,
    outcome: Outcome,
) -> Result<(), String> {
    let position = pipeline
        .steps
        .iter()
        .position(|step| step.id == run.steps[index].step_id)
        .ok_or("Unknown step")?;
    let step = &pipeline.steps[position];
    let next = if step.transitions.is_empty() {
        if !matches!(
            outcome,
            Outcome::Completed | Outcome::Approved | Outcome::Responded
        ) {
            run.status = Status::Failed;
            run.steps[index].status = Status::Failed;
            run.steps[index].error =
                Some("Outcome requires an explicit configured transition".into());
            return Ok(());
        }
        pipeline.steps.get(position + 1).map(|step| step.id.clone())
    } else {
        let Some(transition) = step.transitions.iter().find(|edge| edge.outcome == outcome) else {
            run.status = Status::Failed;
            run.steps[index].status = Status::Failed;
            run.steps[index].error = Some("No configured transition for outcome".into());
            return Ok(());
        };
        let key = format!(
            "{}:{}",
            step.id,
            serde_json::to_string(&outcome).map_err(|error| error.to_string())?
        );
        let count = run.transition_counts.entry(key).or_default();
        run.steps[index].selected_transition = Some(transition.clone());
        if *count >= transition.max_traversals {
            run.status = Status::LimitExhausted;
            run.steps[index].traversal_count = *count;
            run.steps[index].error = Some("Transition limit exhausted. Inspect attempts and start a new run after correcting the task or pipeline; no automatic retry.".into());
            return Ok(());
        }
        *count += 1;
        run.steps[index].traversal_count = *count;
        transition.to.clone()
    };
    run.current_step = next;
    if run.current_step.is_none() {
        run.status = if matches!(
            outcome,
            Outcome::Failed | Outcome::Rejected | Outcome::Blocked
        ) {
            Status::Failed
        } else {
            Status::Succeeded
        };
    }
    Ok(())
}
