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

const MAX_REPORT_RETRIES: usize = 2;
const MAX_RALPH_ITERATIONS: usize = 100;

fn run_ralph_loop(
    input: &str,
    max_iterations: usize,
    mut execute: impl FnMut(&str) -> Result<(String, Option<AgentDecision>), String>,
) -> Result<(String, Option<AgentDecision>), String> {
    let mut previous_output: Option<String> = None;
    for _ in 0..max_iterations {
        let iteration_input = if let Some(previous) = &previous_output {
            format!(
                "{input}\n\nPrevious iteration result:\n{previous}\n\nContinue the work; do not repeat completed work. Call report_outcome with loop_complete=false while work remains and true only when finished."
            )
        } else {
            input.to_owned()
        };
        let (output, decision) = execute(&iteration_input)?;
        if decision
            .as_ref()
            .is_none_or(|decision| decision.loop_complete)
        {
            return Ok((output, decision));
        }
        previous_output = Some(output);
    }
    Err(format!(
        "Agent did not complete the Ralph loop within {max_iterations} iterations"
    ))
}

struct CancellableRuntime<'a, R: ?Sized> {
    runtime: &'a mut R,
    cancelled: &'a (dyn Fn() -> bool + Sync),
}
impl<R: AgentRuntime + ?Sized> AgentRuntime for CancellableRuntime<'_, R> {
    fn set_run_id(&mut self, run_id: &str) -> Result<(), String> {
        self.runtime.set_run_id(run_id)
    }
    fn set_history_required(&mut self, required: bool) {
        self.runtime.set_history_required(required);
    }
    fn set_structured_handoff_required(&mut self, required: bool) {
        self.runtime.set_structured_handoff_required(required);
    }
    fn execute_shell(
        &mut self,
        directory: &std::path::Path,
        command: &str,
        timeout: u32,
        _: &dyn Fn() -> bool,
    ) -> Result<crate::domain::work::CheckResult, String> {
        self.runtime
            .execute_shell(directory, command, timeout, self.cancelled)
    }
    fn list_models(&mut self) -> Result<Vec<crate::domain::runtime::RuntimeModel>, String> {
        self.runtime.list_models()
    }
    fn execute_cancellable(
        &mut self,
        directory: &std::path::Path,
        agent: &Agent,
        input: &str,
        _: &dyn Fn() -> bool,
    ) -> Result<String, String> {
        self.runtime
            .execute_cancellable(directory, agent, input, self.cancelled)
    }
    fn supports_session_resumption(&self, agent: &Agent) -> bool {
        self.runtime.supports_session_resumption(agent)
    }

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
    let mut resolved_config = config.clone();
    let pipeline = resolved_config
        .pipelines
        .iter_mut()
        .find(|p| p.id == pipeline_id)
        .ok_or("Unknown pipeline ID")?;
    if pipeline.max_attempts.is_none() {
        pipeline.max_attempts = Some(50);
    }
    let pipeline = pipeline.clone();
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
        config_snapshot: Some(resolved_config.clone()),
        current_step: Some(pipeline.steps[0].id.clone()),
        handoff: None,
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
                structured_handoff: None,
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
    execute_graph(&resolved_config, run, runtime, repository)
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
        structured_handoff: None,
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
    let contract = if step.requires_structured_handoff() {
        "Required: call report_outcome with a valid structured_handoff.\n\n"
    } else {
        ""
    };
    let loop_guidance = "Continue iterating on this task until it is complete. Verify your work and call report_outcome with loop_complete=true only when finished; use false to continue.";
    match &step.kind {
        crate::domain::project::PipelineStepKind::Agent { prompt, .. } if !prompt.is_empty() => {
            format!(
                "{contract}Step instructions:\n{prompt}\n\nStep context:\n{input}\n\n{loop_guidance}"
            )
        }
        crate::domain::project::PipelineStepKind::Agent { .. } if !contract.is_empty() => {
            format!("{contract}Step context:\n{input}\n\n{loop_guidance}")
        }
        _ => input,
    }
}

pub fn validate_agent_response(
    step: &PipelineStep,
    allowed_outcomes: &[Outcome],
    output: String,
    decision: Option<AgentDecision>,
) -> Result<
    (
        Outcome,
        String,
        Option<crate::domain::pipeline::StructuredHandoff>,
    ),
    String,
> {
    let (outcome, summary, structured_handoff) = if allowed_outcomes.is_empty() {
        if output.trim().is_empty() {
            return Err("Agent returned an empty handoff".into());
        }
        if output.len() > MAX_HANDOFF_BYTES {
            return Err("Agent output exceeds 16384 bytes".into());
        }
        (
            Outcome::Completed,
            output,
            decision.and_then(|decision| decision.structured_handoff),
        )
    } else {
        let decision = decision
            .ok_or_else(|| "Agent completed without reporting a pipeline outcome".to_owned())?;
        if !allowed_outcomes.contains(&decision.outcome) {
            return Err("Agent reported an outcome not allowed by this pipeline step".into());
        }
        if decision.summary.trim().is_empty() {
            return Err("Pipeline outcome summary must not be empty".into());
        }
        if decision.summary.len() > MAX_HANDOFF_BYTES {
            return Err("Pipeline outcome summary exceeds 16384 bytes".into());
        }
        (
            decision.outcome,
            decision.summary,
            decision.structured_handoff,
        )
    };
    if let Some(handoff) = &structured_handoff {
        handoff.validate()?;
    }
    if step.requires_structured_handoff() && structured_handoff.is_none() {
        return Err(format!(
            "Pipeline step {} requires a structured handoff",
            step.id
        ));
    }
    Ok((outcome, summary, structured_handoff))
}

pub fn report_repair_prompt(error: &str, allowed_outcomes: &[Outcome]) -> String {
    let allowed = if allowed_outcomes.is_empty() {
        "completed".to_owned()
    } else {
        allowed_outcomes
            .iter()
            .map(|outcome| {
                serde_json::to_value(outcome)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "Baton rejected your previous report: {error}. The task work is complete; do not repeat or modify it. Call report_outcome again with an allowed outcome ({allowed}) and a valid structured_handoff. Correct only the report."
    )
}

pub fn validate_handoff(run: &PipelineRun) -> Result<(), String> {
    let previous = run
        .steps
        .iter()
        .filter(|attempt| attempt.status == Status::Succeeded)
        .max_by_key(|attempt| attempt.attempt);
    if run.handoff.is_none() && previous.is_some_and(|attempt| attempt.output.is_none()) {
        return Err("Previous step has no persisted handoff".into());
    }
    Ok(())
}

/// Explicit human responses in chronological order, never agent-generated summaries.
pub fn human_decisions(
    run: &PipelineRun,
    pipeline: &crate::domain::project::Pipeline,
) -> Vec<String> {
    run.steps
        .iter()
        .filter(|attempt| {
            attempt.status == Status::Succeeded
                && pipeline
                    .steps
                    .iter()
                    .any(|step| step.id == attempt.step_id && step.is_human())
        })
        .filter_map(|attempt| {
            let response = attempt.output.as_deref()?.trim();
            if response.is_empty() {
                return None;
            }
            Some(format!(
                "Human step {} (attempt {}, outcome {:?}):\n{}",
                attempt.step_id, attempt.attempt, attempt.outcome, response
            ))
        })
        .collect()
}

struct CarriedStructuredHandoff {
    report: crate::domain::pipeline::StructuredHandoff,
    verification_attempts: Vec<u32>,
    source_attempts: Vec<u32>,
    omitted_findings: usize,
    omitted_verification: usize,
}

fn carried_structured_handoff(
    run: &PipelineRun,
) -> Result<Option<CarriedStructuredHandoff>, String> {
    use crate::domain::pipeline::{Finding, StructuredHandoff, VerificationEvidence};
    let mut reports: Vec<_> = run
        .steps
        .iter()
        .filter(|attempt| attempt.status == Status::Succeeded)
        .filter_map(|attempt| {
            attempt
                .structured_handoff
                .as_ref()
                .map(|report| (attempt.attempt, report))
        })
        .collect();
    reports.sort_by_key(|(attempt, _)| *attempt);
    let Some((_, latest)) = reports.last().copied() else {
        return Ok(None);
    };

    let mut findings: Vec<(Finding, u32)> = Vec::new();
    let mut finding_indexes = std::collections::HashMap::new();
    let mut verification: Vec<(VerificationEvidence, u32)> = Vec::new();
    let mut source_attempts = Vec::new();
    for (attempt, report) in reports {
        source_attempts.push(attempt);
        for finding in &report.findings {
            if let Some(index) = finding_indexes.get(&finding.id).copied() {
                findings[index] = (finding.clone(), attempt);
            } else {
                finding_indexes.insert(finding.id.clone(), findings.len());
                findings.push((finding.clone(), attempt));
            }
        }
        verification.extend(
            report
                .verification
                .iter()
                .cloned()
                .map(|item| (item, attempt)),
        );
    }

    let mut carried = StructuredHandoff {
        version: latest.version,
        summary: latest.summary.clone(),
        findings: Vec::new(),
        verification: Vec::new(),
        next_action: latest.next_action.clone(),
    };
    let fits = |candidate: &StructuredHandoff| {
        serde_json::to_vec(candidate)
            .is_ok_and(|bytes| bytes.len() <= crate::domain::pipeline::MAX_STRUCTURED_HANDOFF_BYTES)
    };
    let mut selected_findings = Vec::new();
    let mut ordered_findings: Vec<_> = findings.iter().collect();
    ordered_findings.sort_by_key(|(_, attempt)| std::cmp::Reverse(*attempt));
    for (finding, _) in ordered_findings {
        if carried.findings.len() == crate::domain::pipeline::MAX_FINDINGS {
            break;
        }
        carried.findings.push(finding.clone());
        if fits(&carried) {
            selected_findings.push(finding.id.clone());
        } else {
            carried.findings.pop();
        }
    }
    let mut selected_verification = Vec::new();
    for (item, attempt) in verification.iter().rev() {
        carried.verification.push(item.clone());
        if fits(&carried) {
            selected_verification.push(*attempt);
        } else {
            carried.verification.pop();
        }
    }
    carried.validate()?;
    Ok(Some(CarriedStructuredHandoff {
        omitted_findings: findings.len() - selected_findings.len(),
        omitted_verification: verification.len() - selected_verification.len(),
        verification_attempts: selected_verification,
        source_attempts,
        report: carried,
    }))
}

fn gate_approved_outcome(
    run: &PipelineRun,
    step: &PipelineStep,
    outcome: Outcome,
    mut output: String,
) -> Result<(Outcome, String), String> {
    if outcome != Outcome::Approved {
        return Ok((outcome, output));
    }
    let open_findings = carried_structured_handoff(run)?
        .map(|handoff| {
            handoff
                .report
                .findings
                .into_iter()
                .filter(|finding| finding.status == crate::domain::pipeline::FindingStatus::Open)
                .map(|finding| finding.id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if open_findings.is_empty() {
        return Ok((outcome, output));
    }

    let message = format!(
        "Baton approval gate: unresolved structured finding(s): {}",
        open_findings.join(", ")
    );
    if step
        .transitions
        .iter()
        .any(|transition| transition.outcome == Outcome::ReviseBuild)
    {
        output.push_str("\n\n");
        output.push_str(&message);
        return Ok((Outcome::ReviseBuild, output));
    }
    Err(format!(
        "{message}; no revise_build transition is configured for step {}",
        step.id
    ))
}

pub fn step_input(
    run: &PipelineRun,
    pipeline: &crate::domain::project::Pipeline,
) -> Result<String, String> {
    let include_task = pipeline
        .steps
        .iter()
        .find(|step| Some(&step.id) == run.current_step.as_ref())
        .map(|step| match &step.kind {
            crate::domain::project::PipelineStepKind::Agent { include_task, .. } => {
                include_task.unwrap_or(false)
            }
            _ => true,
        })
        .unwrap_or(true);
    let decisions = human_decisions(run, pipeline);
    let first_step = run.steps.iter().all(|attempt| attempt.attempt == 0);
    if first_step
        && decisions.is_empty()
        && include_task
        && run
            .handoff
            .as_ref()
            .is_none_or(|handoff| handoff == &run.task)
    {
        return Ok(run.task.clone());
    }
    let mut parts = Vec::new();
    if include_task || !decisions.is_empty() {
        parts.push(format!("Original task:\n{}", run.task));
    }
    let handoff = run.handoff.as_ref().or_else(|| {
        run.steps
            .iter()
            .filter(|attempt| attempt.status == Status::Succeeded)
            .max_by_key(|attempt| attempt.attempt)
            .and_then(|attempt| attempt.output.as_ref())
    });
    if !decisions.is_empty() {
        parts.push(format!("Explicit human decisions (chronological; later corrections supersede earlier decisions where applicable):\n{}", decisions.join("\n\n")));
    }
    if let Some(handoff) = handoff
        && !(first_step && handoff == &run.task)
    {
        parts.push(format!("Previous handoff:\n{handoff}"));
    }
    if let Some(structured) = carried_structured_handoff(run)? {
        let evidence =
            serde_json::to_string(&structured.report).map_err(|error| error.to_string())?;
        parts.push(format!("Agent-reported structured handoff (claims and verification evidence are unverified unless independently checked):\n{evidence}"));
        let verification_provenance = structured
            .verification_attempts
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        if !verification_provenance.is_empty() {
            parts.push(format!("Verification evidence above was reported in attempt(s) {verification_provenance}; it remains unverified until independently checked."));
        }
        if structured.omitted_findings > 0 || structured.omitted_verification > 0 {
            let attempts = structured
                .source_attempts
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("Structured context limit: {} finding(s) and {} verification item(s) are not included here; their complete reports remain stored on the listed producing attempts [{attempts}]. Inspect run history; omission here does not mean resolved or verified.", structured.omitted_findings, structured.omitted_verification));
        }
    }
    if run
        .steps
        .iter()
        .filter(|attempt| Some(&attempt.step_id) == run.current_step.as_ref())
        .max_by_key(|attempt| attempt.attempt)
        .is_some_and(|attempt| {
            attempt.error.as_deref()
                == Some("Interrupted by server restart; outcome and side effects are unknown")
        })
    {
        parts.push("Previous attempt was interrupted by server restart. Inspect the existing worktree and reconcile any potentially completed external actions (including commits, pushes and pull requests) before continuing; do not repeat side effects blindly.".into());
    }
    if parts.is_empty() {
        Ok("No task or preceding handoff was supplied for this step.".into())
    } else {
        Ok(parts.join("\n\n"))
    }
}

pub(crate) fn repeated_blocker(
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

pub(crate) fn repeated_blocker_message(step_id: &str) -> String {
    format!("Step {step_id} repeated an identical blocker after a human response")
}

const LOOP_THRESHOLD: usize = 3;

fn repeated_unresolved_count(
    run: &PipelineRun,
    pipeline: &crate::domain::project::Pipeline,
    index: usize,
) -> usize {
    let current = &run.steps[index];
    if !matches!(
        current.outcome,
        Some(Outcome::Blocked | Outcome::Rejected | Outcome::ReviseBuild | Outcome::RevisePlan)
    ) {
        return 0;
    }
    let normalize = |text: &str| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let Some(output) = current
        .output
        .as_deref()
        .map(normalize)
        .filter(|text| !text.is_empty())
    else {
        return 0;
    };
    let mut count = 0;
    let mut attempts: Vec<_> = run
        .steps
        .iter()
        .filter(|attempt| attempt.attempt > 0 && attempt.attempt <= current.attempt)
        .collect();
    attempts.sort_by_key(|attempt| std::cmp::Reverse(attempt.attempt));
    for attempt in attempts {
        if pipeline
            .steps
            .iter()
            .any(|step| step.id == attempt.step_id && step.is_human())
            && attempt.outcome == Some(Outcome::Responded)
        {
            break;
        }
        if attempt.step_id == current.step_id {
            if attempt.outcome != current.outcome
                || attempt.output.as_deref().map(normalize).as_deref() != Some(output.as_str())
            {
                break;
            }
            count += 1;
        }
    }
    count
}

fn step_attempt_count(run: &PipelineRun, step_id: &str) -> usize {
    run.steps
        .iter()
        .filter(|step| step.step_id == step_id && step.status != Status::Pending)
        .count()
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
        if let Err(error) = validate_handoff(&run) {
            run.status = Status::Failed;
            finish_run(&mut run, clock);
            publish(repository, &mut run, Some(error))?;
            return Ok(run);
        }
        let input = match step_input(&run, pipeline) {
            Ok(input) => with_step_prompt(input, step),
            Err(error) => {
                run.status = Status::Failed;
                finish_run(&mut run, clock);
                publish(repository, &mut run, Some(error))?;
                return Ok(run);
            }
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
        if let Some(limit) = pipeline.max_attempts
            && run
                .steps
                .iter()
                .filter(|s| s.status != Status::Pending)
                .count()
                >= limit as usize
        {
            run.status = Status::LimitExhausted;
            let ended_at = utc_now();
            run.ended_at = Some(ended_at.clone());
            run.duration_ms = elapsed_ms(&run.started_at, &ended_at);
            publish(
                repository,
                &mut run,
                Some(format!("Pipeline exhausted maxAttempts limit of {limit}")),
            )?;
            return Ok(run);
        }
        if let Some(limit) = step.max_attempts
            && step_attempt_count(&run, &step.id) >= limit as usize
        {
            run.status = Status::LimitExhausted;
            let ended_at = utc_now();
            run.ended_at = Some(ended_at.clone());
            run.duration_ms = elapsed_ms(&run.started_at, &ended_at);
            publish(
                repository,
                &mut run,
                Some(format!(
                    "Step {} exhausted maxAttempts limit of {}",
                    step.id, limit
                )),
            )?;
            return Ok(run);
        }
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
                    if output.len() > MAX_HANDOFF_BYTES {
                        run.steps[index].status = Status::Failed;
                        run.steps[index].error = Some("Shell handoff exceeds 16384 bytes".into());
                        run.status = Status::Failed;
                    } else {
                        run.handoff = Some(output.clone());
                        run.steps[index].output = Some(output);
                        run.steps[index].outcome = Some(outcome.clone());
                        run.steps[index].status = Status::Succeeded;
                        route(&mut run, pipeline, index, outcome)?;
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
        let history_required = run.steps.iter().any(|attempt| {
            attempt.agent_id == agent.id && attempt.attempt > 0 && attempt.output.is_some()
        });
        let history_unavailable = history_required && !runtime.supports_session_resumption(agent);
        runtime.set_history_required(history_required);
        runtime.set_structured_handoff_history(
            run.steps
                .iter()
                .filter(|attempt| attempt.status == Status::Succeeded)
                .filter_map(|attempt| {
                    attempt.structured_handoff.clone().map(|report| {
                        crate::domain::pipeline::StructuredHandoffAttempt {
                            attempt: attempt.attempt,
                            step_id: attempt.step_id.clone(),
                            report,
                        }
                    })
                })
                .collect(),
        );
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
        let result = if history_unavailable {
            Err("Runtime does not support resuming this agent's session".to_owned())
        } else {
            guarded.set_structured_handoff_required(true);
            let mut runtime_input = input.clone();
            let mut retries = 0;
            loop {
                if cancelled() {
                    break Err("Run cancelled".into());
                }
                let loop_result = run_ralph_loop(
                    &runtime_input,
                    step.max_attempts.unwrap_or(MAX_RALPH_ITERATIONS as u32) as usize,
                    |iteration_input| {
                        if cancelled() {
                            return Err("Run cancelled".into());
                        }
                        let mut observer = |mut entry: RunTraceEntry| {
                            let _lease =
                                acquire_current_attempt_lease(repository, &run, index, &cancelled)?;
                            entry.attempt = Some(run.steps[index].attempt);
                            let kind = entry.kind.clone();
                            append_bounded_trace(&mut run, [entry]);
                            publish(
                                repository,
                                &mut run,
                                Some(format!("Observed runtime activity: {kind}")),
                            )
                        };
                        let output = guarded.execute_pipeline_step_observed(
                            &worktree_path,
                            &assigned,
                            iteration_input,
                            &allowed_outcomes,
                            &cancelled,
                            &mut observer,
                        )?;
                        Ok((output, guarded.take_decision()))
                    },
                );
                let (output, decision) = match loop_result {
                    Ok(result) => result,
                    Err(error) => break Err(error),
                };
                match validate_agent_response(step, &allowed_outcomes, output, decision) {
                    Ok(response) => break Ok(response),
                    Err(error) if retries < MAX_REPORT_RETRIES => {
                        retries += 1;
                        let repair = report_repair_prompt(&error, &allowed_outcomes);
                        let _lease = match acquire_current_attempt_lease(
                            repository, &run, index, &cancelled,
                        ) {
                            Ok(lease) => lease,
                            Err(error) => {
                                if let Some(cancelled) = cancelled_run(repository, &run)? {
                                    return Ok(cancelled);
                                }
                                break Err(error);
                            }
                        };
                        if let Some(recorded_input) = run.steps[index].input.as_mut() {
                            recorded_input.push_str("\n\nBaton report-repair request:\n");
                            recorded_input.push_str(&repair);
                        }
                        publish(
                            repository,
                            &mut run,
                            Some(format!(
                                "Baton rejected the report and requested correction ({retries}/{MAX_REPORT_RETRIES})"
                            )),
                        )?;
                        runtime_input = repair;
                    }
                    Err(error) => break Err(error),
                }
            }
        };
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
        let mut loop_message = None;
        match result {
            Ok((outcome, output, structured_handoff)) => {
                run.steps[index].structured_handoff = structured_handoff;
                run.steps[index].status = Status::Succeeded;
                let (outcome, output) = match gate_approved_outcome(&run, step, outcome, output) {
                    Ok(result) => result,
                    Err(error) => {
                        run.steps[index].status = Status::Failed;
                        run.steps[index].error = Some(error.clone());
                        run.status = Status::Failed;
                        finish_step_attempt(&mut run.steps[index], step_clock);
                        publish(repository, &mut run, Some(error))?;
                        continue;
                    }
                };
                run.handoff = Some(output.clone());
                run.steps[index].output = Some(output.clone());
                run.steps[index].outcome = Some(outcome.clone());
                let repeated =
                    outcome == Outcome::Blocked && repeated_blocker(&run, pipeline, index, &output);
                let unresolved_count = repeated_unresolved_count(&run, pipeline, index);
                route(&mut run, pipeline, index, outcome)?;
                if run.status == Status::Running
                    && unresolved_count >= LOOP_THRESHOLD
                    && unresolved_count.is_multiple_of(LOOP_THRESHOLD)
                {
                    run.status = Status::WaitingHuman;
                    run.pending_actions = vec![Outcome::Responded];
                    loop_message = Some(format!(
                        "Possible unproductive loop: step {} reported the same unresolved outcome and summary {unresolved_count} times (threshold {LOOP_THRESHOLD}). Review the attempts and worktree, then respond to continue or cancel.",
                        step.id
                    ));
                }
                if repeated && loop_message.is_none() {
                    publish(
                        repository,
                        &mut run,
                        Some(repeated_blocker_message(&step.id)),
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
            Some(loop_message.unwrap_or_else(|| format!("Step {} completed", step.id))),
        )?;
        if run.status == Status::WaitingHuman {
            return Ok(run);
        }
    }
    finish_run(&mut run, clock);
    publish(repository, &mut run, Some("Run stopped".into()))?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ralph_loop_repeats_with_previous_output_until_completion_marker() {
        let mut calls = Vec::new();
        let result = run_ralph_loop("task\n\nContinue iterating on this task until it is complete. Verify your work and call report_outcome with loop_complete=true only when finished; use false to continue.", MAX_RALPH_ITERATIONS, |input| {
            calls.push(input.to_owned());
            match calls.len() {
                1 => Ok((
                    "partial result".into(),
                    Some(AgentDecision {
                        loop_complete: false,
                        outcome: Outcome::Completed,
                        summary: "partial".into(),
                        structured_handoff: None,
                    }),
                )),
                _ => Ok((
                    "finished".into(),
                    Some(AgentDecision {
                        loop_complete: true,
                        outcome: Outcome::Completed,
                        summary: "finished".into(),
                        structured_handoff: None,
                    }),
                )),
            }
        })
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].contains("loop_complete=true only when finished"));
        assert!(calls[1].contains("Previous iteration result:\npartial result"));
        assert_eq!(result.0, "finished");
    }

    #[test]
    fn ralph_loop_fails_after_iteration_limit() {
        let mut calls = 0;
        let error = run_ralph_loop("task", MAX_RALPH_ITERATIONS, |_| {
            calls += 1;
            Ok((
                "still working".into(),
                Some(AgentDecision {
                    loop_complete: false,
                    outcome: Outcome::Completed,
                    summary: "still working".into(),
                    structured_handoff: None,
                }),
            ))
        })
        .unwrap_err();
        assert_eq!(calls, MAX_RALPH_ITERATIONS);
        assert!(error.contains("within 100 iterations"));
    }

    #[test]
    fn global_attempt_count_ignores_pending_placeholders_and_counts_started_visits() {
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"r", "pipelineId":"p", "task":"t", "startedAt":"now", "status":"running", "steps":[]
        })).unwrap();
        let mut attempt = pending(&PipelineStep {
            id: "human".into(),
            name: "Human".into(),
            max_attempts: None,
            kind: crate::domain::project::PipelineStepKind::Human,
            transitions: vec![],
        });
        run.steps.push(attempt.clone());
        assert_eq!(
            run.steps
                .iter()
                .filter(|s| s.status != Status::Pending)
                .count(),
            0
        );
        attempt.status = Status::WaitingHuman;
        run.steps[0] = attempt;
        assert_eq!(
            run.steps
                .iter()
                .filter(|s| s.status != Status::Pending)
                .count(),
            1
        );
    }

    #[test]
    fn step_attempt_count_ignores_pending_placeholders_but_counts_started_legacy_records() {
        let mut pending = pending(&PipelineStep {
            id: "step".into(),
            name: "Step".into(),
            max_attempts: None,
            kind: crate::domain::project::PipelineStepKind::Human,
            transitions: vec![],
        });
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"r", "pipelineId":"p", "task":"t", "startedAt":"now", "status":"running", "steps":[]
        })).unwrap();
        run.steps.push(pending.clone());
        assert_eq!(step_attempt_count(&run, "step"), 0);
        pending.status = Status::Failed;
        run.steps[0] = pending;
        assert_eq!(step_attempt_count(&run, "step"), 1);
    }

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
        assert!(first_input.contains("Required: call report_outcome"));
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
    fn detects_identical_unresolved_findings_across_visits_and_reload() {
        let config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let pipeline = &config.pipelines[0];
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"r", "pipelineId":pipeline.id, "task":"t", "startedAt":"now", "status":"running", "steps":[]
        })).unwrap();
        let mut review = pending(&pipeline.steps[0]);
        review.status = Status::Succeeded;
        review.outcome = Some(Outcome::ReviseBuild);
        review.output = Some("Fix the missing test".into());
        for attempt in 1..=3 {
            review.attempt = attempt;
            run.steps.push(review.clone());
        }
        run.steps.reverse();
        let recovered: PipelineRun =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        assert_eq!(repeated_unresolved_count(&recovered, pipeline, 0), 3);
        run.steps[1].output = Some("Fix a different test".into());
        assert_eq!(repeated_unresolved_count(&run, pipeline, 0), 1);
        run.steps[0].outcome = Some(Outcome::Completed);
        assert_eq!(repeated_unresolved_count(&run, pipeline, 0), 0);
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
        let reloaded: PipelineRun =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        assert!(repeated_blocker(
            &reloaded,
            &pipeline,
            2,
            " WHICH backends? "
        ));
        run.steps[1].step_id = "another-step".into();
        assert!(!repeated_blocker(&run, &pipeline, 2, "Which backends?"));
        run.steps[1].step_id = pipeline.steps[0].id.clone();
        run.steps[0].outcome = Some(Outcome::Rejected);
        assert!(!repeated_blocker(&run, &pipeline, 2, "Which backends?"));
    }

    #[test]
    fn human_decisions_survive_handoffs_corrections_and_reload() {
        let mut config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let pipeline = &mut config.pipelines[0];
        let human = serde_json::from_str(r#"{"id":"help","name":"Help","type":"human"}"#).unwrap();
        pipeline.steps.push(human);
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"r", "pipelineId":pipeline.id, "task":"original", "startedAt":"now",
            "status":"running", "steps":[], "currentStep":pipeline.steps[0].id, "handoff":"builder handoff"
        })).unwrap();
        for (attempt, response) in [
            (1, "List Pi models like Rig"),
            (3, "Correction: list only available Pi models"),
        ] {
            let mut decision = pending(pipeline.steps.last().unwrap());
            decision.attempt = attempt;
            decision.status = Status::Succeeded;
            decision.output = Some(response.into());
            decision.outcome = Some(Outcome::Responded);
            run.steps.push(decision);
        }
        let mut agent = pending(&pipeline.steps[0]);
        agent.attempt = 2;
        agent.status = Status::Succeeded;
        agent.output = Some("unverified agent summary".into());
        run.steps.insert(1, agent);
        let recovered: PipelineRun =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        let input = step_input(&recovered, pipeline).unwrap();
        assert!(input.contains("Original task:\noriginal"));
        assert!(input.contains("builder handoff"));
        assert!(input.contains("Human step help (attempt 1"));
        assert!(
            input.find("List Pi models like Rig").unwrap()
                < input
                    .find("Correction: list only available Pi models")
                    .unwrap()
        );
        assert!(!input.contains("unverified agent summary"));
        assert_eq!(human_decisions(&recovered, pipeline).len(), 2);
    }

    #[test]
    fn approval_is_routed_back_while_carried_findings_remain_open() {
        let mut config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let pipeline = &mut config.pipelines[0];
        let review = &mut pipeline.steps[1];
        review.transitions = vec![
            crate::domain::project::Transition {
                outcome: Outcome::Approved,
                to: None,
            },
            crate::domain::project::Transition {
                outcome: Outcome::ReviseBuild,
                to: Some("write".into()),
            },
        ];
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"r", "pipelineId":pipeline.id, "task":"task", "startedAt":"now",
            "status":"running", "currentStep":"review", "steps":[
                {"stepId":"write","agentId":"writer","status":"succeeded","attempt":1,
                 "structuredHandoff":{"version":1,"summary":"Build","findings":[
                    {"id":"F-1","status":"open","description":"Add regression test"}
                 ]}},
                {"stepId":"review","agentId":"reviewer","status":"succeeded","attempt":2,
                 "output":"Looks good despite prior finding"}
            ]
        }))
        .unwrap();

        let (outcome, output) =
            gate_approved_outcome(&run, review, Outcome::Approved, "Looks good".into()).unwrap();
        assert_eq!(outcome, Outcome::ReviseBuild);
        assert!(output.contains("F-1"));
        let mut without_rework = review.clone();
        without_rework
            .transitions
            .retain(|transition| transition.outcome != Outcome::ReviseBuild);
        assert!(
            gate_approved_outcome(
                &run,
                &without_rework,
                Outcome::Approved,
                "Looks good".into(),
            )
            .unwrap_err()
            .contains("no revise_build transition")
        );

        run.steps[1].structured_handoff = Some(
            serde_json::from_value(serde_json::json!({
                "version":1,"summary":"Verified","findings":[
                    {"id":"F-1","status":"resolved","description":"Added regression test"}
                ]
            }))
            .unwrap(),
        );
        let (outcome, output) = gate_approved_outcome(
            &run,
            review,
            Outcome::Approved,
            "Approved with evidence".into(),
        )
        .unwrap();
        assert_eq!(outcome, Outcome::Approved);
        assert_eq!(output, "Approved with evidence");
    }

    #[test]
    fn verification_evidence_survives_later_omission_with_provenance_after_reload() {
        let config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let pipeline = &config.pipelines[0];
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"r", "pipelineId":pipeline.id, "task":"task", "startedAt":"now",
            "status":"running", "currentStep":pipeline.steps[0].id, "handoff":"prose",
            "steps":[
                {"stepId":"build","agentId":"a","status":"succeeded","attempt":1,"structuredHandoff":{"version":1,"summary":"A","verification":[{"description":"cargo test","result":"passed"}]}},
                {"stepId":"review","agentId":"b","status":"succeeded","attempt":2,"structuredHandoff":{"version":1,"summary":"B"}}
            ]
        })).unwrap();
        run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        let input = step_input(&run, pipeline).unwrap();
        assert!(input.contains("cargo test") && input.contains("passed"));
        assert!(input.contains("attempt(s) 1"));
    }

    #[test]
    fn oversized_aggregate_is_bounded_with_persisted_attempt_notice_not_run_failure() {
        let config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let pipeline = &config.pipelines[0];
        let steps: Vec<_> = (1..=150).map(|attempt| serde_json::json!({
            "stepId":"build","agentId":"a","status":"succeeded","attempt":attempt,
            "structuredHandoff":{"version":1,"summary":"Latest","findings":[{"id":format!("F-{attempt}"),"status":"open","description":"A distinct unresolved finding with enough detail to consume the bounded prompt budget."}]}
        })).collect();
        let run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"r", "pipelineId":pipeline.id, "task":"task", "startedAt":"now",
            "status":"running", "currentStep":pipeline.steps[0].id, "handoff":"prose", "steps":steps
        }))
        .unwrap();
        let input = step_input(&run, pipeline).unwrap();
        assert!(input.contains("Structured context limit:"));
        assert!(input.contains("complete reports remain stored"));
        let structured_json = input
            .lines()
            .find(|line| line.starts_with("{\"version\":1"))
            .unwrap();
        assert!(structured_json.len() <= crate::domain::pipeline::MAX_STRUCTURED_HANDOFF_BYTES);
    }

    #[test]
    fn carried_findings_survive_omission_and_apply_later_status_updates_after_reload() {
        let config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let pipeline = &config.pipelines[0];
        let make_run = |reports: serde_json::Value| -> PipelineRun {
            serde_json::from_value(serde_json::json!({
                "id":"r", "pipelineId":pipeline.id, "task":"task", "startedAt":"now",
                "status":"running", "currentStep":pipeline.steps[0].id, "handoff":"latest prose",
                "steps": reports
            }))
            .unwrap()
        };
        let a = make_run(serde_json::json!([
            {"stepId":"build","agentId":"a","status":"succeeded","attempt":1,"structuredHandoff":{"version":1,"summary":"A","findings":[{"id":"F-1","status":"open","description":"Investigate"}]}},
            {"stepId":"human","agentId":"","status":"succeeded","attempt":2,"output":"human response"},
            {"stepId":"review","agentId":"b","status":"succeeded","attempt":3,"output":"legacy prose", "structuredHandoff":{"version":1,"summary":"B","findings":[{"id":"F-2","status":"open","description":"Add tests"}]}}
        ]));
        let recovered: PipelineRun =
            serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        let input = step_input(&recovered, pipeline).unwrap();
        assert!(input.contains("F-1") && input.contains("F-2"));
        let updated = make_run(serde_json::json!([
            {"stepId":"build","agentId":"a","status":"succeeded","attempt":1,"structuredHandoff":{"version":1,"summary":"A","findings":[{"id":"F-1","status":"open","description":"Investigate"}]}},
            {"stepId":"human","agentId":"","status":"succeeded","attempt":2,"output":"human response"},
            {"stepId":"review","agentId":"b","status":"succeeded","attempt":3,"output":"legacy prose", "structuredHandoff":{"version":1,"summary":"B","findings":[{"id":"F-2","status":"open","description":"Add tests"}]}},
            {"stepId":"build","agentId":"a","status":"succeeded","attempt":4,"structuredHandoff":{"version":1,"summary":"C","findings":[{"id":"F-1","status":"resolved","description":"Investigate"}]}}
        ]));
        let input = step_input(&updated, pipeline).unwrap();
        assert!(input.contains("F-2") && input.contains("resolved"));
    }

    #[test]
    fn structured_handoff_is_persisted_and_forwarded_as_unverified_evidence() {
        let config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let pipeline = &config.pipelines[0];
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"r", "pipelineId":pipeline.id, "task":"original", "startedAt":"now",
            "status":"running", "currentStep":pipeline.steps[0].id, "handoff":"legacy text",
            "steps":[{"stepId":"plan", "agentId":"writer", "status":"succeeded", "attempt":1,
                "structuredHandoff":{"version":1,"summary":"Plan", "findings":[{"id":"F-1","status":"open","description":"Check lockfile"}]}}]
        })).unwrap();
        let persisted: PipelineRun =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        let input = step_input(&persisted, pipeline).unwrap();
        assert!(input.contains("legacy text"));
        assert!(input.contains("F-1"));
        assert!(input.contains("unverified"));
        run.steps[0].structured_handoff.as_mut().unwrap().findings[0].status =
            crate::domain::pipeline::FindingStatus::Resolved;
        assert_eq!(
            run.steps[0].structured_handoff.as_ref().unwrap().findings[0].status,
            crate::domain::pipeline::FindingStatus::Resolved
        );
    }

    #[test]
    fn only_latest_handoff_is_forwarded_after_reload_and_in_loops() {
        let mut config: ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let pipeline = &mut config.pipelines[0];
        if let crate::domain::project::PipelineStepKind::Agent { include_task, .. } =
            &mut pipeline.steps[0].kind
        {
            *include_task = Some(true);
        }
        let mut build = pipeline.steps[0].clone();
        build.id = "build".into();
        if let crate::domain::project::PipelineStepKind::Agent { include_task, .. } =
            &mut build.kind
        {
            *include_task = None;
        }
        pipeline.steps.push(build);
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"r", "pipelineId":pipeline.id, "task":"original", "startedAt":"now",
            "status":"running", "steps":[], "currentStep":"build", "handoff":"plan"
        }))
        .unwrap();
        assert_eq!(
            step_input(&run, pipeline).unwrap(),
            "Previous handoff:\nplan"
        );
        run.handoff = None;
        let legacy: PipelineRun =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        assert_eq!(
            step_input(&legacy, pipeline).unwrap(),
            "No task or preceding handoff was supplied for this step."
        );
        run.current_step = Some(pipeline.steps[0].id.clone());
        run.handoff = Some("build result".into());
        let reloaded: PipelineRun =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        let input = step_input(&reloaded, pipeline).unwrap();
        assert!(
            input.contains("original") && input.contains("build result") && !input.contains("plan")
        );
        run.current_step = Some("build".into());
        run.handoff = Some("review findings".into());
        assert_eq!(
            step_input(&run, pipeline).unwrap(),
            "Previous handoff:\nreview findings"
        );
        let mut legacy = pending(&pipeline.steps[0]);
        legacy.attempt = 1;
        legacy.status = Status::Succeeded;
        legacy.output = Some("legacy result".into());
        run.steps.push(legacy);
        run.handoff = None;
        assert_eq!(
            step_input(&run, pipeline).unwrap(),
            "Previous handoff:\nlegacy result"
        );
        run.steps.last_mut().unwrap().output = None;
        assert!(validate_handoff(&run).is_err());
        run.handoff = Some("review findings".into());
        let mut interrupted = pending(pipeline.steps.last().unwrap());
        interrupted.error =
            Some("Interrupted by server restart; outcome and side effects are unknown".into());
        run.steps.push(interrupted);
        let input = step_input(&run, pipeline).unwrap();
        assert!(
            input.contains("review findings") && input.contains("interrupted by server restart")
        );
    }
}
