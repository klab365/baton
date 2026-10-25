//! Control-plane directed execution. No implicit retries or agent-selected destinations.
use crate::project::{Agent, ProjectConfig};
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub const MAX_HANDOFF_BYTES: usize = 16 * 1024;

pub trait AgentRuntime {
    /// `directory` is the isolated run workspace and the only project working
    /// directory adapters may use. Never fall back to the process directory or
    /// change global cwd (other runs can execute concurrently).
    fn execute(
        &mut self,
        directory: &std::path::Path,
        agent: &Agent,
        input: &str,
    ) -> Result<String, String>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    WaitingHuman,
    LimitExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StepRun {
    pub step_id: String,
    pub agent_id: String,
    pub status: Status,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub outcome: Option<crate::project::Outcome>,
    #[serde(default)]
    pub selected_transition: Option<crate::project::Transition>,
    #[serde(default)]
    pub traversal_count: u32,
    #[serde(default)]
    pub attempt: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PipelineRun {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default)]
    pub worktree: Option<crate::worktrees::RunWorktree>,
    #[serde(default)]
    pub setup_error: Option<String>,
    pub pipeline_id: String,
    pub task: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub events: Vec<RunEvent>,
    pub status: Status,
    pub steps: Vec<StepRun>,
    #[serde(default)]
    pub config_snapshot: Option<ProjectConfig>,
    #[serde(default)]
    pub current_step: Option<String>,
    #[serde(default)]
    pub transition_counts: std::collections::BTreeMap<String, u32>,
    #[serde(default)]
    pub handoff: Option<String>,
    #[serde(default)]
    pub pending_actions: Vec<crate::project::Outcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub run_id: String,
    pub status: Status,
    pub occurred_at: String,
    pub message: Option<String>,
}

pub trait RunRepository {
    fn worktree_manager(&self) -> Result<Box<dyn crate::worktrees::WorktreeManager>, String> {
        Err("Project Git worktree manager is not configured".into())
    }
    fn create(&self, run: &PipelineRun) -> Result<(), String>;
    fn save(&self, run: &PipelineRun) -> Result<(), String>;
    fn load(&self, id: &str) -> Result<PipelineRun, String>;
    fn list(&self) -> Result<Vec<PipelineRun>, String>;
    /// Durable publication point for future live-update consumers.
    fn publish(&self, _event: &RunEvent) -> Result<(), String> {
        Ok(())
    }
}

fn publish(
    repository: &impl RunRepository,
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
    repository: &impl RunRepository,
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
        repository.save(&current)?;
    }
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
    runtime: &mut impl AgentRuntime,
    repository: &impl RunRepository,
) -> Result<PipelineRun, String> {
    start_pipeline_for_task(config, pipeline_id, task, run_id, None, runtime, repository)
}

pub fn start_pipeline_for_task(
    config: &ProjectConfig,
    pipeline_id: &str,
    task: &str,
    run_id: &str,
    task_id: Option<String>,
    runtime: &mut impl AgentRuntime,
    repository: &impl RunRepository,
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
        task_id,
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
    let allocation = repository
        .worktree_manager()
        .and_then(|manager| manager.allocate(run_id));
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
        Some("Isolated worktree allocated; retained until explicit cleanup".into()),
    )?;
    execute_graph(config, run, runtime, repository)
}

fn pending(step: &crate::project::PipelineStep) -> StepRun {
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredResult {
    outcome: crate::project::Outcome,
    output: String,
}

/// Explicit user action only. Invalid actions do not consume or mutate the wait.
pub fn resume_human(
    id: &str,
    outcome: crate::project::Outcome,
    response: &str,
    runtime: &mut impl AgentRuntime,
    repository: &impl RunRepository,
) -> Result<PipelineRun, String> {
    use crate::project::Outcome;
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
        .find(|p| p.id == run.pipeline_id)
        .ok_or("Unknown pipeline")?;
    let step = pipeline
        .steps
        .iter()
        .find(|s| Some(&s.id) == run.current_step.as_ref())
        .ok_or("Unknown current step")?;
    if !step.human {
        return Err("Current step is not human".into());
    }
    if !step.transitions.is_empty() && !step.transitions.iter().any(|t| t.outcome == outcome) {
        return Err("No configured transition for human action".into());
    }
    repository.worktree_manager()?.validate(
        &run.id,
        run.worktree.as_ref().ok_or("Missing isolated worktree")?,
    )?;
    let index = run
        .steps
        .iter()
        .rposition(|s| s.status == Status::WaitingHuman)
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

fn route(
    run: &mut PipelineRun,
    pipeline: &crate::project::Pipeline,
    index: usize,
    outcome: crate::project::Outcome,
) -> Result<(), String> {
    use crate::project::Outcome;
    let position = pipeline
        .steps
        .iter()
        .position(|s| s.id == run.steps[index].step_id)
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
        pipeline.steps.get(position + 1).map(|s| s.id.clone())
    } else {
        let Some(transition) = step.transitions.iter().find(|t| t.outcome == outcome) else {
            run.status = Status::Failed;
            run.steps[index].status = Status::Failed;
            run.steps[index].error = Some("No configured transition for outcome".into());
            return Ok(());
        };
        let key = format!(
            "{}:{}",
            step.id,
            serde_json::to_string(&outcome).map_err(|e| e.to_string())?
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

fn execute_graph(
    config: &ProjectConfig,
    mut run: PipelineRun,
    runtime: &mut impl AgentRuntime,
    repository: &impl RunRepository,
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
                "Original task:\n{}\n\nPrevious step output and handoff (including any reviewer feedback):\n{}",
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
                vec![
                    crate::project::Outcome::Approved,
                    crate::project::Outcome::Rejected,
                    crate::project::Outcome::Responded,
                ]
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
        let prompt = if step.transitions.is_empty() {
            input
        } else {
            format!(
                "{input}\n\nReturn ONLY JSON with fields outcome and output. Allowed outcomes: {}. Do not select a destination.",
                serde_json::to_string(
                    &step
                        .transitions
                        .iter()
                        .map(|t| &t.outcome)
                        .collect::<Vec<_>>()
                )
                .map_err(|e| e.to_string())?
            )
        };
        let mut tool_audit = Vec::new();
        let result = if prompt.len() > MAX_HANDOFF_BYTES {
            Err("Agent input exceeds 16384 bytes including outcome instructions".into())
        } else {
            crate::tools::execute(
                runtime,
                &run.worktree
                    .as_ref()
                    .ok_or("Missing isolated worktree")?
                    .path,
                agent,
                &prompt,
                &mut tool_audit,
            )
        }
        .and_then(|output| {
            if output.len() > MAX_HANDOFF_BYTES {
                return Err("Agent output exceeds 16384 bytes".into());
            }
            if step.transitions.is_empty() {
                Ok((crate::project::Outcome::Completed, output))
            } else {
                // Preserve raw output even when the structured contract is invalid.
                run.steps[index].output = Some(output.clone());
                let result: StructuredResult = serde_json::from_str(&output)
                    .map_err(|e| format!("Invalid structured outcome: {e}"))?;
                Ok((result.outcome, result.output))
            }
        });
        for message in tool_audit {
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
