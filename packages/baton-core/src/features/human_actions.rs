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

pub(crate) fn human_handoff(previous: Option<&str>, response: &str) -> Result<String, String> {
    let mut handoff = match (
        previous.filter(|value| !value.is_empty()),
        response.is_empty(),
    ) {
        (Some(previous), true) => previous.to_owned(),
        (Some(previous), false) => {
            format!("Previous handoff:\n{previous}\n\nHuman response:\n{response}")
        }
        (None, _) => response.to_owned(),
    };
    if handoff.len() > MAX_HANDOFF_BYTES {
        let previous = previous.unwrap_or_default();
        let marker = "[Earlier handoff truncated]\n";
        let prefix = format!("Previous handoff:\n{marker}");
        let suffix = format!("\n\nHuman response:\n{response}");
        let framing = prefix.len() + suffix.len();
        if framing > MAX_HANDOFF_BYTES {
            return Ok(response.to_owned());
        }
        let available = MAX_HANDOFF_BYTES - framing;
        let mut start = previous.len().saturating_sub(available);
        while !previous.is_char_boundary(start) {
            start += 1;
        }
        let context = &previous[start..];
        handoff = format!("{prefix}{context}{suffix}");
    }
    Ok(handoff)
}

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
    let loop_pause = !run
        .steps
        .iter()
        .any(|attempt| attempt.status == Status::WaitingHuman)
        && run.events.iter().rev().any(|event| {
            event
                .message
                .as_deref()
                .is_some_and(|message| message.starts_with("Possible unproductive loop:"))
        });
    if loop_pause {
        if outcome != Outcome::Responded {
            return Err("Expected a response to continue the paused run".into());
        }
        let workspace = run.worktree.as_ref().ok_or("Missing isolated worktree")?;
        repository
            .worktree_manager()?
            .validate(&workspace.owner, workspace)?;
        run.handoff = Some(human_handoff(run.handoff.as_deref(), response)?);
        run.status = Status::Running;
        run.pending_actions.clear();
        publish(
            repository,
            &mut run,
            Some("Human continued after possible unproductive loop".into()),
        )?;
        return Ok((config, run));
    }
    let step = pipeline
        .steps
        .iter()
        .find(|step| Some(&step.id) == run.current_step.as_ref())
        .ok_or("Unknown current step")?;
    if !step.is_human() {
        return Err("Current step is not human".into());
    }
    if !step.transitions.is_empty() {
        let matches = step
            .transitions
            .iter()
            .filter(|edge| edge.outcome == outcome)
            .count();
        if matches == 0 {
            return Err("No configured transition for human action".into());
        }
        if matches > 1 {
            return Err("Ambiguous configured transition for human action".into());
        }
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
    run.handoff = Some(human_handoff(run.handoff.as_deref(), response)?);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_handoff_preserves_full_prior_context_for_empty_response_at_limit() {
        let previous = "x".repeat(MAX_HANDOFF_BYTES);
        assert_eq!(human_handoff(Some(&previous), "").unwrap(), previous);
    }

    #[test]
    fn human_handoff_keeps_response_and_bounds_large_prior_context() {
        let previous = "é".repeat(MAX_HANDOFF_BYTES / 2);
        let handoff = human_handoff(Some(&previous), "please fix it").unwrap();
        assert!(handoff.contains("please fix it"));
        assert!(handoff.contains("[Earlier handoff truncated]"));
        assert!(handoff.len() <= MAX_HANDOFF_BYTES);
    }

    #[test]
    fn large_response_keeps_newest_prior_context_that_fits() {
        let previous = "p".repeat(MAX_HANDOFF_BYTES);
        let response = "r".repeat(10_000);
        let handoff = human_handoff(Some(&previous), &response).unwrap();
        assert!(handoff.starts_with("Previous handoff:\n[Earlier handoff truncated]\n"));
        assert!(handoff.contains("p"));
        assert!(handoff.ends_with(&format!("\n\nHuman response:\n{response}")));
        assert!(handoff.len() <= MAX_HANDOFF_BYTES);
    }

    #[test]
    fn response_is_forwarded_alone_when_framing_cannot_fit() {
        let response = "r".repeat(MAX_HANDOFF_BYTES - 10);
        assert_eq!(human_handoff(Some("prior"), &response).unwrap(), response);
    }

    #[test]
    fn maximum_size_response_is_forwarded_without_added_overhead() {
        let response = "r".repeat(MAX_HANDOFF_BYTES);
        assert_eq!(human_handoff(Some("prior"), &response).unwrap(), response);
    }
}
