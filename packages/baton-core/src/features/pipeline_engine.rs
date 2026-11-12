//! Control-plane directed execution. No implicit retries or agent-selected destinations.
use crate::{
    domain::pipeline::{
        PipelineRun, RunEvent, RunTraceEntry, Status, StepRun, TaskRunContext, transitions::route,
    },
    domain::project::{Agent, Outcome, PipelineStep, ProjectConfig},
    ports::{
        runs::{RunActionLease, RunRepository},
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
    fn execute_pipeline_step_observed(
        &mut self,
        directory: &std::path::Path,
        agent: &Agent,
        input: &str,
        allowed_outcomes: &[Outcome],
        _cancelled: &dyn Fn() -> bool,
        observer: &mut dyn FnMut(RunTraceEntry) -> Result<(), String>,
    ) -> Result<String, String> {
        self.runtime.execute_pipeline_step_observed(
            directory,
            agent,
            input,
            allowed_outcomes,
            self.cancelled,
            observer,
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

pub fn acquire_action_lease_retry(
    repository: &dyn RunRepository,
    run_id: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Box<dyn RunActionLease>, String> {
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if cancelled() {
            return Err("Run cancelled".into());
        }
        match repository.acquire_action_lease(run_id) {
            Ok(lease) => return Ok(lease),
            Err(error)
                if matches!(
                    error.as_str(),
                    "Run action is already in progress" | "Work action is already in progress"
                ) && Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
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
fn acquire_active_run_lease(
    repository: &dyn RunRepository,
    run: &PipelineRun,
    cancelled: &dyn Fn() -> bool,
) -> Result<Box<dyn RunActionLease>, String> {
    let lease = acquire_action_lease_retry(repository, &run.id, cancelled)?;
    let persisted = repository.load(&run.id)?;
    if persisted.status != Status::Running || persisted.current_step != run.current_step {
        return Err("Run cancelled or active step changed".into());
    }
    Ok(lease)
}

fn acquire_current_attempt_lease(
    repository: &dyn RunRepository,
    run: &PipelineRun,
    index: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Box<dyn RunActionLease>, String> {
    let lease = acquire_action_lease_retry(repository, &run.id, cancelled)?;
    let persisted = repository.load(&run.id)?;
    if persisted.status != Status::Running
        || !persisted.steps.iter().any(|step| {
            step.step_id == run.steps[index].step_id
                && step.attempt == run.steps[index].attempt
                && step.status == Status::Running
        })
    {
        return Err("Run cancelled or attempt changed".into());
    }
    Ok(lease)
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
    let ended_at = utc_now();
    if let Some(step) = current
        .steps
        .iter_mut()
        .rev()
        .find(|step| step.status == Status::Running)
    {
        step.status = Status::Cancelled;
        step.ended_at = Some(ended_at.clone());
        step.duration_ms = step
            .started_at
            .as_deref()
            .and_then(|start| elapsed_ms(start, &ended_at));
    }
    current.ended_at.get_or_insert(ended_at);
    for event in &run.events {
        if !current.events.contains(event) {
            current.events.push(event.clone());
        }
    }
    repository.save(&current)?;
    Ok(Some(current))
}

pub fn append_bounded_trace(
    run: &mut PipelineRun,
    entries: impl IntoIterator<Item = RunTraceEntry>,
) {
    const MAX_TRACE_BYTES: usize = 256 * 1024;
    const MAX_TRACE_ENTRIES: usize = 512;
    const RUN_LIMIT_MARKER: &str = "Run trace limits reached; later events omitted";
    const MAX_ENTRY_BYTES: usize = 8 * 1024;
    const TRUNCATION_RESERVE: usize = 128;
    if run
        .trace
        .iter()
        .any(|entry| entry.kind == "traceTruncated" && entry.content == RUN_LIMIT_MARKER)
    {
        return;
    }
    let mut used = run.trace.iter().fold(0usize, |used, entry| {
        used.saturating_add(entry.content.len())
            .saturating_add(entry.kind.len())
            .saturating_add(entry.occurred_at.len())
    });
    let mut truncated = false;
    for mut entry in entries {
        if run.trace.iter().any(|saved| {
            saved.attempt == entry.attempt
                && saved.kind == entry.kind
                && saved.occurred_at == entry.occurred_at
                && (saved.content == entry.content
                    || matches!(
                        (
                            serde_json::from_str::<serde_json::Value>(&saved.content),
                            serde_json::from_str::<serde_json::Value>(&entry.content)
                        ),
                        (Ok(saved), Ok(incoming)) if saved == incoming
                    ))
        }) {
            continue;
        }
        if run.trace.len() >= MAX_TRACE_ENTRIES - 1 || used >= MAX_TRACE_BYTES - TRUNCATION_RESERVE
        {
            truncated = true;
            break;
        }
        let overhead = entry.kind.len() + entry.occurred_at.len();
        let available = (MAX_TRACE_BYTES - TRUNCATION_RESERVE)
            .saturating_sub(used)
            .saturating_sub(overhead);
        let allowed = available.min(MAX_ENTRY_BYTES.saturating_sub(overhead));
        if allowed == 0 {
            truncated = true;
            break;
        }
        if entry.content.len() > allowed {
            truncated = true;
            let marker = "…[truncated]";
            let mut end = if allowed > marker.len() {
                allowed - marker.len()
            } else {
                allowed
            };
            while !entry.content.is_char_boundary(end) {
                end -= 1;
            }
            entry.content.truncate(end);
            if allowed > marker.len() {
                entry.content.push_str(marker);
            }
        }
        used += entry.content.len() + overhead;
        run.trace.push(entry);
        if truncated {
            break;
        }
    }
    if truncated {
        run.trace.push(RunTraceEntry {
            attempt: None,
            occurred_at: utc_now(),
            kind: "traceTruncated".into(),
            content: RUN_LIMIT_MARKER.into(),
        });
    }
}

fn finish_run(run: &mut PipelineRun, _started: Instant) {
    let ended_at = utc_now();
    run.duration_ms = elapsed_ms(&run.started_at, &ended_at);
    run.ended_at = Some(ended_at);
}

fn finish_step_attempt(step: &mut StepRun, started: Instant) {
    let ended = utc_now();
    step.duration_ms = Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
    step.ended_at = Some(ended);
}

fn account_duration(run: &mut PipelineRun, started: Instant) {
    run.duration_ms = Some(
        run.duration_ms
            .unwrap_or(0)
            .saturating_add(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
    );
}

pub fn elapsed_ms(start: &str, end: &str) -> Option<u64> {
    fn parse(value: &str) -> Option<i64> {
        let b = value.as_bytes();
        if b.len() != 24
            || b[4] != b'-'
            || b[7] != b'-'
            || b[10] != b'T'
            || b[13] != b':'
            || b[16] != b':'
            || b[19] != b'.'
            || b[23] != b'Z'
        {
            return None;
        }
        let number = |range: std::ops::Range<usize>| {
            std::str::from_utf8(&b[range]).ok()?.parse::<i64>().ok()
        };
        let (y, m, d, hh, mm, ss, ms) = (
            number(0..4)?,
            number(5..7)?,
            number(8..10)?,
            number(11..13)?,
            number(14..16)?,
            number(17..19)?,
            number(20..23)?,
        );
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let month_days = match m {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => return None,
        };
        if !(1..=month_days).contains(&d) || hh > 23 || mm > 59 || ss > 60 || ms > 999 {
            return None;
        }
        let y = y - i64::from(m <= 2);
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = m + if m > 2 { -3 } else { 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Some((((days * 24 + hh) * 60 + mm) * 60 + ss) * 1000 + ms)
    }
    parse(end)?.checked_sub(parse(start)?)?.try_into().ok()
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
                agent_id: s.agent_id().to_owned(),
                status: Status::Pending,
                input: None,
                output: None,
                error: None,
                outcome: None,
                selected_transition: None,
                attempt: 0,
                started_at: None,
                ended_at: None,
                duration_ms: None,
                model: None,
                usage: None,
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
        agent_id: step.agent_id().to_owned(),
        status: Status::Pending,
        input: None,
        output: None,
        error: None,
        outcome: None,
        selected_transition: None,
        attempt: 0,
        started_at: None,
        ended_at: None,
        duration_ms: None,
        model: None,
        usage: None,
    }
}

pub fn with_step_prompt(input: String, step: &PipelineStep) -> String {
    match &step.kind {
        crate::domain::project::PipelineStepKind::Agent { prompt, .. } if !prompt.is_empty() => {
            format!("Step instructions:\n{prompt}\n\nTask and prior handoff:\n{input}")
        }
        _ => input,
    }
}

pub fn step_input(run: &PipelineRun, pipeline: &crate::domain::project::Pipeline) -> String {
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
            .any(|step| step.is_human() && step.id == attempt.step_id);
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
            .any(|step| step.is_human() && step.id == attempt.step_id)
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

pub fn execute_graph(
    config: &ProjectConfig,
    mut run: PipelineRun,
    runtime: &mut dyn AgentRuntime,
    repository: &dyn RunRepository,
) -> Result<PipelineRun, String> {
    let clock = Instant::now();
    runtime.set_run_id(&run.id)?;
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
        let input = with_step_prompt(step_input(&run, pipeline), step);
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
        let cancellation_id = run.id.clone();
        let cancelled = || {
            repository
                .load(&cancellation_id)
                .map(|r| r.status == Status::Cancelled)
                .unwrap_or(true)
        };
        let activation_lease = match acquire_active_run_lease(repository, &run, &cancelled) {
            Ok(lease) => lease,
            Err(error) => {
                if let Some(cancelled) = cancelled_run(repository, &run)? {
                    return Ok(cancelled);
                }
                return Err(error);
            }
        };
        run.steps[index].attempt = run.steps.iter().map(|s| s.attempt).max().unwrap_or(0) + 1;
        run.steps[index].input = Some(input.clone());
        if step.is_human() {
            run.steps[index].started_at = Some(utc_now());
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
        run.steps[index].started_at = Some(utc_now());
        let step_clock = Instant::now();
        publish(
            repository,
            &mut run,
            Some(format!("Step {} started", step.id)),
        )?;
        drop(activation_lease);
        if let Some(command) = step.command() {
            let result = runtime.execute_shell(
                &run.worktree.as_ref().ok_or("Missing worktree")?.path,
                command,
                step.timeout_seconds().unwrap_or(300),
                &cancelled,
            );
            if let Some(cancelled) = cancelled_run(repository, &run)? {
                return Ok(cancelled);
            }
            let _lease = match acquire_current_attempt_lease(repository, &run, index, &cancelled) {
                Ok(lease) => lease,
                Err(error) => {
                    if let Some(cancelled) = cancelled_run(repository, &run)? {
                        return Ok(cancelled);
                    }
                    return Err(error);
                }
            };
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
            finish_step_attempt(&mut run.steps[index], step_clock);
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
            .find(|a| a.id == step.agent_id())
            .ok_or("Unknown agent")?;
        let allowed_outcomes = step
            .transitions
            .iter()
            .map(|transition| transition.outcome.clone())
            .collect::<Vec<_>>();
        run.steps[index].model = agent.model.clone();
        let assigned = agent.clone();
        runtime.set_history_required(run.steps.iter().any(|attempt| {
            attempt.agent_id == agent.id && attempt.attempt > 0 && attempt.output.is_some()
        }));
        let worktree_path = run
            .worktree
            .as_ref()
            .ok_or("Missing isolated worktree")?
            .path
            .clone();
        let mut guarded = CancellableRuntime {
            runtime,
            cancelled: &cancelled,
        };
        let mut observer = |mut entry: RunTraceEntry| {
            let _lease = acquire_current_attempt_lease(repository, &run, index, &cancelled)?;
            entry.attempt = Some(run.steps[index].attempt);
            let kind = entry.kind.clone();
            append_bounded_trace(&mut run, [entry]);
            publish(
                repository,
                &mut run,
                Some(format!("Observed runtime activity: {kind}")),
            )
        };
        let result = guarded
            .execute_pipeline_step_observed(
                &worktree_path,
                &assigned,
                &input,
                &allowed_outcomes,
                &cancelled,
                &mut observer,
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
        let mut trace = runtime.take_trace();
        for entry in &mut trace {
            entry.attempt = Some(run.steps[index].attempt);
        }
        append_bounded_trace(&mut run, trace);
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
        let _lease = match acquire_current_attempt_lease(repository, &run, index, &cancelled) {
            Ok(lease) => lease,
            Err(error) => {
                if let Some(cancelled) = cancelled_run(repository, &run)? {
                    return Ok(cancelled);
                }
                return Err(error);
            }
        };
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
        finish_step_attempt(&mut run.steps[index], step_clock);
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
    fn trace_storage_is_bounded_by_bytes_entries_and_per_entry_size() {
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id": "r", "pipelineId": "p", "task": "t", "startedAt": "2026-01-01T00:00:00.000Z",
            "status": "running", "steps": [], "trace": [], "configSnapshot": null
        }))
        .unwrap();
        let entries = (0..600).map(|_| RunTraceEntry {
            attempt: Some(2),
            occurred_at: utc_now(),
            kind: "metadata".into(),
            content: "x".repeat(9000),
        });
        append_bounded_trace(&mut run, entries);
        assert!(run.trace.len() <= 512);
        assert!(
            run.trace
                .iter()
                .map(|entry| entry.content.len() + entry.kind.len() + entry.occurred_at.len())
                .sum::<usize>()
                <= 256 * 1024
        );
        assert!(run.trace.iter().all(|entry| entry.kind == "traceTruncated"
            || (entry.attempt == Some(2) && entry.content.len() <= 8 * 1024)));
        assert!(
            run.trace
                .iter()
                .any(|entry| entry.content.ends_with("…[truncated]"))
        );
        assert_eq!(run.trace.last().unwrap().kind, "traceTruncated");
    }

    #[test]
    fn capture_truncation_does_not_stop_later_run_or_attempt_traces() {
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id": "r", "pipelineId": "p", "task": "t", "startedAt": "2026-01-01T00:00:00.000Z",
            "status": "running", "steps": [], "trace": [], "configSnapshot": null
        }))
        .unwrap();
        run.trace.push(RunTraceEntry {
            attempt: Some(1),
            occurred_at: utc_now(),
            kind: "traceTruncated".into(),
            content: "Runtime activity capture limits reached; later events omitted".into(),
        });
        append_bounded_trace(
            &mut run,
            [RunTraceEntry {
                attempt: Some(2),
                occurred_at: utc_now(),
                kind: "modelRequest".into(),
                content: "later attempt data".into(),
            }],
        );
        assert!(run.trace.iter().any(|entry| entry.attempt == Some(2)));
    }

    #[test]
    fn step_prompts_are_applied_per_step_without_mutating_agent_instructions() {
        let first: PipelineStep = serde_json::from_value(serde_json::json!({
            "id":"first", "name":"First", "type":"agent", "agentId":"shared", "prompt":"Draft"
        }))
        .unwrap();
        let second: PipelineStep = serde_json::from_value(serde_json::json!({
            "id":"second", "name":"Second", "type":"agent", "agentId":"shared", "prompt":"Review"
        }))
        .unwrap();
        let first_input = with_step_prompt("Task".into(), &first);
        let second_input = with_step_prompt("Task".into(), &second);
        assert!(first_input.contains("Step instructions:\nDraft"));
        assert!(second_input.contains("Step instructions:\nReview"));
        assert!(!first_input.contains("Review"));
        assert!(!second_input.contains("Draft"));
    }

    #[test]
    fn computes_restart_safe_elapsed_time_from_persisted_timestamps() {
        assert_eq!(
            elapsed_ms("2024-02-29T23:59:59.500Z", "2024-03-01T00:00:00.250Z"),
            Some(750)
        );
        assert_eq!(
            elapsed_ms("2026-01-01T00:00:01.000Z", "2026-01-01T00:00:00.000Z"),
            None
        );
        assert_eq!(elapsed_ms("unavailable", "2026-01-01T00:00:00.000Z"), None);
        assert_eq!(
            elapsed_ms("2026-02-31T00:00:00.000Z", "2026-03-01T00:00:00.000Z"),
            None
        );
    }

    #[test]
    fn detects_only_repeated_blockers_after_human_response() {
        let config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let mut pipeline = config.pipelines[0].clone();
        let mut human = pipeline.steps[0].clone();
        human.id = "help".into();
        human.kind = crate::domain::project::PipelineStepKind::Human;
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
        human.kind = crate::domain::project::PipelineStepKind::Human;
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
        human.kind = crate::domain::project::PipelineStepKind::Human;
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
