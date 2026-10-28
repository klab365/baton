//! Control-plane directed execution. No implicit retries or agent-selected destinations.
use crate::{
    domain::pipeline::{
        PipelineRun, RunEvent, Status, StepRun, TaskRunContext, transitions::route,
    },
    domain::project::{Agent, Outcome, PipelineStep, ProjectConfig},
    ports::{
        runs::RunRepository,
        runtime::{AgentDecision, AgentRuntime, MAX_HANDOFF_BYTES},
    },
};
use std::time::Instant;

struct CancellableRuntime<'a, R: ?Sized> {
    runtime: &'a mut R,
    cancelled: &'a (dyn Fn() -> bool + Sync),
}
impl<R: AgentRuntime + ?Sized> AgentRuntime for CancellableRuntime<'_, R> {
    fn execute(
        &mut self,
        directory: &std::path::Path,
        agent: &Agent,
        input: &str,
    ) -> Result<String, String> {
        self.runtime
            .execute_cancellable(directory, agent, input, self.cancelled)
    }
    fn execute_pipeline_step(
        &mut self,
        directory: &std::path::Path,
        agent: &Agent,
        input: &str,
        allowed_outcomes: &[Outcome],
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<String, String> {
        self.runtime.execute_pipeline_step(
            directory,
            agent,
            input,
            allowed_outcomes,
            self.cancelled,
        )
    }
    fn take_decision(&mut self) -> Option<AgentDecision> {
        self.runtime.take_decision()
    }
    fn take_events(&mut self) -> Vec<String> {
        self.runtime.take_events()
    }
}

pub(crate) fn publish(
    repository: &dyn RunRepository,
    run: &mut PipelineRun,
    message: Option<String>,
) -> Result<(), String> {
    let event = RunEvent {
        run_id: run.id.clone(),
        status: run.status.clone(),
        occurred_at: utc_now(),
        message,
    };
    run.events.push(event.clone());
    repository.save(run)?;
    repository.publish(&event)
}
fn cancelled_run(
    repository: &dyn RunRepository,
    run: &PipelineRun,
) -> Result<Option<PipelineRun>, String> {
    let mut current = repository.load(&run.id)?;
    if current.status != Status::Cancelled {
        return Ok(None);
    }
    // Cancellation can arrive while Git is allocating. Never lose the newly
    // allocated identity, and never invoke a runtime after observing cancellation.
    if current.worktree.is_none() && run.worktree.is_some() {
        current.worktree = run.worktree.clone();
    }
    for event in &run.events {
        if !current.events.contains(event) {
            current.events.push(event.clone());
        }
    }
    repository.save(&current)?;
    Ok(Some(current))
}

fn finish_run(run: &mut PipelineRun, started: Instant) {
    run.ended_at = Some(utc_now());
    account_duration(run, started);
}

fn account_duration(run: &mut PipelineRun, started: Instant) {
    run.duration_ms = Some(
        run.duration_ms
            .unwrap_or(0)
            .saturating_add(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
    );
}

pub fn utc_now() -> String {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = elapsed.as_secs();
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        day_seconds / 3600,
        (day_seconds % 3600) / 60,
        day_seconds % 60,
        elapsed.subsec_millis()
    )
}
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

pub fn start_pipeline(
    config: &ProjectConfig,
    pipeline_id: &str,
    task: &str,
    run_id: &str,
    runtime: &mut dyn AgentRuntime,
    repository: &dyn RunRepository,
) -> Result<PipelineRun, String> {
    start_pipeline_for_task(config, pipeline_id, task, run_id, None, runtime, repository)
}

pub fn start_pipeline_for_task(
    config: &ProjectConfig,
    pipeline_id: &str,
    task: &str,
    run_id: &str,
    task_id: Option<String>,
    runtime: &mut dyn AgentRuntime,
    repository: &dyn RunRepository,
) -> Result<PipelineRun, String> {
    start_pipeline_for_task_with_worktree(
        config,
        pipeline_id,
        task,
        run_id,
        TaskRunContext {
            task_id,
            worktree: None,
        },
        runtime,
        repository,
    )
}

pub fn start_pipeline_for_task_with_worktree(
    config: &ProjectConfig,
    pipeline_id: &str,
    task: &str,
    run_id: &str,
    task_context: TaskRunContext,
    runtime: &mut dyn AgentRuntime,
    repository: &dyn RunRepository,
) -> Result<PipelineRun, String> {
    config.validate()?;
    if task.trim().is_empty() || task.len() > MAX_HANDOFF_BYTES {
        return Err("Task must be nonempty and at most 16384 bytes".into());
    }
    let pipeline = config
        .pipelines
        .iter()
        .find(|p| p.id == pipeline_id)
        .ok_or("Unknown pipeline ID")?;
    let started_clock = Instant::now();
    let mut run = PipelineRun {
        id: run_id.into(),
        task_id: task_context.task_id,
        worktree: None,
        setup_error: None,
        pipeline_id: pipeline.id.clone(),
        task: task.into(),
        started_at: utc_now(),
        ended_at: None,
        duration_ms: None,
        events: Vec::new(),
        config_snapshot: Some(config.clone()),
        current_step: Some(pipeline.steps[0].id.clone()),
        transition_counts: Default::default(),
        handoff: Some(task.into()),
        pending_actions: Vec::new(),
        status: Status::Running,
        steps: pipeline
            .steps
            .iter()
            .map(|s| StepRun {
                step_id: s.id.clone(),
                agent_id: s.agent_id.clone(),
                status: Status::Pending,
                input: None,
                output: None,
                error: None,
                outcome: None,
                selected_transition: None,
                traversal_count: 0,
                attempt: 0,
            })
            .collect(),
    };
    repository.create(&run)?;
    publish(repository, &mut run, Some("Run started".into()))?;
    let worktree_owner = run.task_id.as_deref().unwrap_or(run_id).to_owned();
    let allocation = repository.worktree_manager().and_then(|manager| {
        if let Some(worktree) = task_context.worktree {
            // Existing per-run worktrees may be promoted when migrating old tasks.
            manager.validate(&worktree.owner, &worktree)?;
            Ok(worktree)
        } else {
            manager.allocate(&worktree_owner)
        }
    });
    match allocation {
        Ok(worktree) => run.worktree = Some(worktree),
        Err(error) => {
            run.setup_error = Some(error.clone());
            run.status = Status::Failed;
            finish_run(&mut run, started_clock);
            publish(repository, &mut run, Some(error))?;
            return Ok(run);
        }
    }
    if let Some(cancelled) = cancelled_run(repository, &run)? {
        return Ok(cancelled);
    }
    publish(
        repository,
        &mut run,
        Some(
            "Task worktree allocated or reused; retained until task completion or deletion".into(),
        ),
    )?;
    execute_graph(config, run, runtime, repository)
}

fn pending(step: &PipelineStep) -> StepRun {
    StepRun {
        step_id: step.id.clone(),
        agent_id: step.agent_id.clone(),
        status: Status::Pending,
        input: None,
        output: None,
        error: None,
        outcome: None,
        selected_transition: None,
        traversal_count: 0,
        attempt: 0,
    }
}

pub(crate) fn execute_graph(
    config: &ProjectConfig,
    mut run: PipelineRun,
    runtime: &mut dyn AgentRuntime,
    repository: &dyn RunRepository,
) -> Result<PipelineRun, String> {
    let clock = Instant::now();
    let pipeline = config
        .pipelines
        .iter()
        .find(|p| p.id == run.pipeline_id)
        .ok_or("Unknown pipeline")?;
    while run.status == Status::Running {
        if let Some(cancelled) = cancelled_run(repository, &run)? {
            return Ok(cancelled);
        }
        let step = pipeline
            .steps
            .iter()
            .find(|s| Some(&s.id) == run.current_step.as_ref())
            .ok_or("Unknown current step")?;
        let previous_output = run.handoff.clone().unwrap_or_else(|| run.task.clone());
        let input = if run.transition_counts.is_empty() && previous_output == run.task {
            run.task.clone()
        } else {
            format!(
                "Original task:\n{}\n\nPrevious step output and handoff:\n{}",
                run.task, previous_output
            )
        };
        let index = if let Some(i) = run
            .steps
            .iter()
            .position(|s| s.step_id == step.id && s.status == Status::Pending)
        {
            i
        } else {
            run.steps.push(pending(step));
            run.steps.len() - 1
        };
        run.steps[index].attempt = run.steps.iter().map(|s| s.attempt).max().unwrap_or(0) + 1;
        run.steps[index].input = Some(input.clone());
        if step.human {
            run.steps[index].status = Status::WaitingHuman;
            run.status = Status::WaitingHuman;
            run.pending_actions = if step.transitions.is_empty() {
                vec![Outcome::Approved, Outcome::Rejected, Outcome::Responded]
            } else {
                step.transitions.iter().map(|t| t.outcome.clone()).collect()
            };
            account_duration(&mut run, clock);
            publish(
                repository,
                &mut run,
                Some(
                    "Waiting for explicit human approval, rejection, or response; cancel to abort"
                        .into(),
                ),
            )?;
            return Ok(run);
        }
        run.steps[index].status = Status::Running;
        publish(
            repository,
            &mut run,
            Some(format!("Step {} started", step.id)),
        )?;
        let agent = config
            .agents
            .iter()
            .find(|a| a.id == step.agent_id)
            .ok_or("Unknown agent")?;
        let allowed_outcomes = step
            .transitions
            .iter()
            .map(|transition| transition.outcome.clone())
            .collect::<Vec<_>>();
        let mut assigned = agent.clone();
        for id in &agent.skill_ids {
            let skill = config
                .skills
                .iter()
                .find(|skill| &skill.id == id)
                .ok_or("Unknown assigned skill")?;
            assigned.system_prompt.push_str(&format!(
                "\n\nAssigned skill {}:\n{}",
                skill.name, skill.instructions
            ));
        }
        assigned.skill_ids.clear();
        let cancellation_id = run.id.clone();
        let cancelled = || {
            repository
                .load(&cancellation_id)
                .map(|r| r.status == Status::Cancelled)
                .unwrap_or(true)
        };
        let mut guarded = CancellableRuntime {
            runtime,
            cancelled: &cancelled,
        };
        let result = if input.len() > MAX_HANDOFF_BYTES {
            Err("Agent input exceeds 16384 bytes".into())
        } else {
            guarded.execute_pipeline_step(
                &run.worktree
                    .as_ref()
                    .ok_or("Missing isolated worktree")?
                    .path,
                &assigned,
                &input,
                &allowed_outcomes,
                &cancelled,
            )
        }
        .and_then(|output| {
            if output.len() > MAX_HANDOFF_BYTES {
                return Err("Agent output exceeds 16384 bytes".into());
            }
            if allowed_outcomes.is_empty() {
                Ok((Outcome::Completed, output))
            } else {
                let decision = guarded.take_decision().ok_or_else(|| {
                    "Agent completed without reporting a pipeline outcome".to_owned()
                })?;
                if !allowed_outcomes.contains(&decision.outcome) {
                    return Err(
                        "Agent reported an outcome not allowed by this pipeline step".into(),
                    );
                }
                if decision.summary.len() > MAX_HANDOFF_BYTES {
                    return Err("Pipeline outcome summary exceeds 16384 bytes".into());
                }
                Ok((decision.outcome, decision.summary))
            }
        });
        for message in runtime.take_events() {
            run.events.push(RunEvent {
                run_id: run.id.clone(),
                status: Status::Running,
                occurred_at: utc_now(),
                message: Some(message),
            });
        }
        if let Some(cancelled) = cancelled_run(repository, &run)? {
            return Ok(cancelled);
        }
        match result {
            Ok((outcome, output)) => {
                run.handoff = Some(output.clone());
                run.steps[index].output = Some(output);
                run.steps[index].outcome = Some(outcome.clone());
                run.steps[index].status = Status::Succeeded;
                route(&mut run, pipeline, index, outcome)?;
            }
            Err(error) => {
                run.steps[index].status = Status::Failed;
                run.steps[index].error = Some(error.chars().take(4096).collect());
                run.status = Status::Failed;
            }
        }
        publish(
            repository,
            &mut run,
            Some(format!("Step {} completed", step.id)),
        )?;
    }
    finish_run(&mut run, clock);
    publish(repository, &mut run, Some("Run stopped".into()))?;
    Ok(run)
}
