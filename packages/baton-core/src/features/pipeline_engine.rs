//! Control-plane directed execution. No implicit retries or agent-selected destinations.
use crate::{
    domain::pipeline::{
        PipelineRun, RunEvent, RunTraceEntry, Status, StepRun, TaskRunContext, transitions::route,
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
    fn take_trace(&mut self) -> Vec<RunTraceEntry> {
        self.runtime.take_trace()
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
        session: None,
        pipeline_id: pipeline.id.clone(),
        task: task.into(),
        started_at: utc_now(),
        ended_at: None,
        duration_ms: None,
        events: Vec::new(),
        trace: Vec::new(),
        config_snapshot: Some(config.clone()),
        current_step: Some(pipeline.steps[0].id.clone()),
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
        attempt: 0,
    }
}

fn step_input(run: &PipelineRun, pipeline: &crate::domain::project::Pipeline) -> String {
    let previous_output = run.handoff.as_deref().unwrap_or(&run.task);
    if run.steps.iter().all(|step| step.attempt == 0) && previous_output == run.task {
        return run.task.clone();
    }
    let mut input = format!("Original task:\n{}", run.task);
    let mut attempts: Vec<_> = run.steps.iter().filter(|step| step.attempt > 0).collect();
    attempts.sort_by_key(|attempt| attempt.attempt);
    // Human decisions remain complete; recent agent results have a separate budget.
    const HISTORY_BYTES: usize = 100 * 1024;
    let mut remaining = HISTORY_BYTES;
    let mut history = Vec::new();
    for attempt in attempts.iter().rev() {
        let human = pipeline
            .steps
            .iter()
            .any(|step| step.human && step.id == attempt.step_id);
        if human && attempt.outcome.is_none() {
            continue;
        }
        let Some(output) = attempt.output.as_deref().or(human.then_some("")) else {
            continue;
        };
        let actor = if human { "Human" } else { &attempt.agent_id };
        let outcome = attempt
            .outcome
            .as_ref()
            .map(|outcome| format!("{outcome:?}"))
            .unwrap_or_else(|| "Result".into());
        let transition = attempt
            .selected_transition
            .as_ref()
            .map(|transition| format!(" → {}", transition.to.as_deref().unwrap_or("end")))
            .unwrap_or_default();
        let text = if human && output.trim().is_empty() {
            "No comment provided."
        } else {
            output
        };
        let entry = format!(
            "\n\n{}. {} · {} · {}{}\n{}",
            attempt.attempt, attempt.step_id, actor, outcome, transition, text
        );
        if !human {
            if entry.len() > remaining {
                continue;
            }
            remaining -= entry.len();
        }
        history.push(entry);
    }
    if attempts
        .last()
        .and_then(|attempt| attempt.output.as_deref())
        != Some(previous_output)
    {
        input.push_str(&format!("\n\nLatest handoff:\n{previous_output}"));
    }
    if !history.is_empty() {
        input.push_str("\n\nRun history (chronological):");
        for entry in history.iter().rev() {
            input.push_str(entry);
        }
    }
    input
}

fn repeated_blocker(
    run: &PipelineRun,
    pipeline: &crate::domain::project::Pipeline,
    index: usize,
    output: &str,
) -> bool {
    let normalize = |text: &str| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let current = normalize(output);
    if current.is_empty() {
        return false;
    }
    let mut human_responded = false;
    let current_attempt = run.steps[index].attempt;
    let mut attempts: Vec<_> = run
        .steps
        .iter()
        .filter(|attempt| attempt.attempt > 0 && attempt.attempt < current_attempt)
        .collect();
    attempts.sort_by_key(|attempt| std::cmp::Reverse(attempt.attempt));
    for attempt in attempts {
        if pipeline
            .steps
            .iter()
            .any(|step| step.human && step.id == attempt.step_id)
            && attempt.outcome == Some(Outcome::Responded)
        {
            human_responded = true;
        }
        if human_responded
            && attempt.step_id == run.steps[index].step_id
            && attempt.outcome == Some(Outcome::Blocked)
            && attempt
                .output
                .as_deref()
                .is_some_and(|previous| normalize(previous) == current)
        {
            return true;
        }
    }
    false
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
        let input = step_input(&run, pipeline);
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
        let cancellation_id = run.id.clone();
        let cancelled = || {
            repository
                .load(&cancellation_id)
                .map(|r| r.status == Status::Cancelled)
                .unwrap_or(true)
        };
        if let Some(command) = &step.command {
            let result = runtime.execute_shell(
                &run.worktree.as_ref().ok_or("Missing worktree")?.path,
                command,
                step.timeout_seconds.unwrap_or(300),
                &cancelled,
            );
            if let Some(cancelled) = cancelled_run(repository, &run)? {
                return Ok(cancelled);
            }
            match result {
                Ok(result) => {
                    let outcome = if result.exit_code == Some(0) {
                        Outcome::Completed
                    } else {
                        Outcome::Failed
                    };
                    let output = format!(
                        "Exit status: {}\n{}",
                        result
                            .exit_code
                            .map_or_else(|| "none".into(), |code| code.to_string()),
                        result.output
                    );
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
            continue;
        }
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
        let assigned = agent.clone();
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
        let result = guarded
            .execute_pipeline_step(
                &run.worktree
                    .as_ref()
                    .ok_or("Missing isolated worktree")?
                    .path,
                &assigned,
                &input,
                &allowed_outcomes,
                &cancelled,
            )
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
        run.trace
            .extend(runtime.take_trace().into_iter().map(|mut entry| {
                entry.attempt = Some(run.steps[index].attempt);
                entry
            }));
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
                let repeated =
                    outcome == Outcome::Blocked && repeated_blocker(&run, pipeline, index, &output);
                run.handoff = Some(output.clone());
                run.steps[index].output = Some(output);
                run.steps[index].outcome = Some(outcome.clone());
                run.steps[index].status = Status::Succeeded;
                route(&mut run, pipeline, index, outcome)?;
                if repeated {
                    publish(
                        repository,
                        &mut run,
                        Some(format!(
                            "Step {} repeated an identical blocker after a human response",
                            step.id
                        )),
                    )?;
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_only_repeated_blockers_after_human_response() {
        let config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let mut pipeline = config.pipelines[0].clone();
        let mut human = pipeline.steps[0].clone();
        human.id = "help".into();
        human.human = true;
        pipeline.steps.push(human.clone());
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id": "run", "pipelineId": pipeline.id, "task": "task", "startedAt": "now",
            "endedAt": null, "durationMs": null, "status": "running", "steps": []
        }))
        .unwrap();
        let mut blocked = pending(&pipeline.steps[0]);
        blocked.attempt = 1;
        blocked.outcome = Some(Outcome::Blocked);
        blocked.output = Some("Which backends?".into());
        let mut response = pending(&human);
        response.attempt = 2;
        response.outcome = Some(Outcome::Responded);
        response.output = Some("All three".into());
        let mut current = pending(&pipeline.steps[0]);
        current.attempt = 3;
        run.steps = vec![response, blocked, current];
        assert!(repeated_blocker(&run, &pipeline, 2, " which   BACKENDS? "));
        assert!(!repeated_blocker(&run, &pipeline, 2, "What timer API?"));
        run.steps[0].outcome = Some(Outcome::Rejected);
        assert!(!repeated_blocker(&run, &pipeline, 2, "Which backends?"));
    }

    #[test]
    fn run_history_preserves_blocker_and_response_after_reload_and_intermediate_step() {
        let config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let mut pipeline = config.pipelines[0].clone();
        let mut human = pipeline.steps[0].clone();
        human.id = "help".into();
        human.human = true;
        pipeline.steps.push(human.clone());
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id": "run", "pipelineId": pipeline.id, "task": "implement timers",
            "startedAt": "now", "endedAt": null, "durationMs": null,
            "status": "running", "steps": [], "handoff": "use embassy-time"
        }))
        .unwrap();
        let mut blocked = pending(&pipeline.steps[0]);
        blocked.attempt = 1;
        blocked.outcome = Some(Outcome::Blocked);
        blocked.output = Some("Which Embassy timer driver?".into());
        let mut response = pending(&human);
        response.attempt = 2;
        response.outcome = Some(Outcome::Responded);
        response.output = Some("use embassy-time".into());
        run.steps = vec![response, blocked];
        let reloaded: PipelineRun =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        let planner_input = step_input(&reloaded, &pipeline);
        assert!(planner_input.contains("Which Embassy timer driver?"));
        assert!(planner_input.contains("use embassy-time"));
        assert!(
            planner_input.find("Which Embassy timer driver?").unwrap()
                < planner_input.find("use embassy-time").unwrap()
        );
        assert_eq!(
            planner_input.matches("Which Embassy timer driver?").count(),
            1
        );
        assert!(!planner_input.contains("Some("));
        let mut resumed = reloaded;
        let mut planner = pending(&pipeline.steps[0]);
        planner.attempt = 3;
        planner.outcome = Some(Outcome::Completed);
        planner.output = Some("Implement the plan".into());
        resumed.steps.push(planner);
        resumed.handoff = Some("Implement the plan".into());
        let builder_input = step_input(&resumed, &pipeline);
        assert!(builder_input.contains("Which Embassy timer driver?"));
        assert!(builder_input.contains("use embassy-time"));
        assert!(builder_input.contains("Implement the plan"));
    }

    #[test]
    fn agent_history_is_bounded_and_prefers_recent_attempts() {
        let config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let pipeline = &config.pipelines[0];
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id": "run", "pipelineId": pipeline.id, "task": "task",
            "startedAt": "now", "endedAt": null, "durationMs": null,
            "status": "running", "steps": [], "handoff": "latest"
        }))
        .unwrap();
        for attempt in 1..=15 {
            let mut step = pending(&pipeline.steps[0]);
            step.attempt = attempt;
            step.output = Some(format!("attempt-{attempt} {}", "x".repeat(10_000)));
            run.steps.push(step);
        }
        let input = step_input(&run, pipeline);
        assert!(input.contains("attempt-15"));
        assert!(!input.contains("attempt-1 "));
        assert!(input.len() < 101 * 1024);
    }

    #[test]
    fn human_decisions_survive_intermediate_agent_handoffs_without_truncation() {
        let config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let mut pipeline = config.pipelines[0].clone();
        let mut human = pipeline.steps[0].clone();
        human.id = "help".into();
        human.human = true;
        pipeline.steps.push(human.clone());
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id": "run", "pipelineId": pipeline.id, "task": "implement timers",
            "startedAt": "now", "endedAt": null, "durationMs": null,
            "status": "running", "steps": [], "handoff": "planner output"
        }))
        .unwrap();
        let mut response = pending(&human);
        response.status = Status::Succeeded;
        response.attempt = 1;
        response.output = Some("support Tokio, WASM and Embassy".into());
        response.outcome = Some(Outcome::Responded);
        run.steps.push(response.clone());
        let input = step_input(&run, &pipeline);
        assert!(input.contains("planner output"));
        assert!(input.contains("support Tokio, WASM and Embassy"));
        assert!(input.contains("Responded"));

        response.attempt = 2;
        response.output = Some("x".repeat(MAX_HANDOFF_BYTES));
        run.steps.insert(0, response);
        let input = step_input(&run, &pipeline);
        assert!(input.len() > MAX_HANDOFF_BYTES);
        assert!(input.contains("support Tokio, WASM and Embassy"));
        assert!(input.contains(&"x".repeat(MAX_HANDOFF_BYTES)));
        assert!(input.find("support Tokio").unwrap() < input.find(&"x".repeat(100)).unwrap());

        let mut approval = pending(&human);
        approval.attempt = 3;
        approval.outcome = Some(Outcome::Approved);
        approval.output = Some(String::new());
        run.steps.push(approval);
        assert!(step_input(&run, &pipeline).contains("Approved\nNo comment provided."));
    }
}
