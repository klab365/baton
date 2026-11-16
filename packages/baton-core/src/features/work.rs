//! Externally executed steps use the same run records and routing rules as managed runtimes.
use crate::{
    domain::{
        pipeline::{PipelineRun, Status, StepRun, transitions::route},
        project::{Agent, ConfigState, Outcome, Pipeline, PipelineStep, ProjectConfig, Skill},
        work::*,
    },
    features::{
        human_actions::human_handoff,
        pipeline_engine::{
            append_bounded_trace, elapsed_ms, publish, repeated_blocker, repeated_blocker_message,
            utc_now,
        },
    },
    ports::{
        project_config::ProjectConfigRepository, runs::RunRepository, runtime::MAX_HANDOFF_BYTES,
        skills::SkillRepository, work::WorkCheckRunner,
    },
};
use medi_rs::{MediCommand, medi_handler, medi_module};
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct WorkResources {
    pub config: Arc<dyn ProjectConfigRepository>,
    pub runs: Arc<dyn RunRepository>,
    pub skills: Arc<dyn SkillRepository>,
    pub checks: Arc<dyn WorkCheckRunner>,
    pub workspace: PathBuf,
}

#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct StartWork {
    pub id: String,
    pub pipeline_id: Option<String>,
    pub task: String,
    pub source: Option<String>,
    pub checks: Vec<WorkCheck>,
}
#[derive(MediCommand)]
#[medi_command(return_type = WorkItem, error_type = String)]
pub struct NextWork {
    pub id: String,
}
#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct SubmitWork {
    pub id: String,
    pub result: WorkSubmission,
    pub input: Option<String>,
    pub trace: Vec<crate::domain::pipeline::RunTraceEntry>,
}
#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct FailWork {
    pub id: String,
    pub revision: u64,
    pub step_id: String,
    pub attempt: u32,
    pub input: String,
    pub trace: Vec<crate::domain::pipeline::RunTraceEntry>,
    pub failure: WorkFailure,
}
#[derive(Clone, Copy)]
pub enum WorkFailure {
    InvalidOutcome,
    Runtime,
    Interrupted,
    MissingSession,
    ResourceUnavailable,
    ProviderTransient,
    ProviderPolicy,
    ProviderAuthentication,
    ProviderConfiguration,
    InvalidHandoff,
}
impl WorkFailure {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidOutcome => "Agent did not report a valid configured outcome",
            Self::Runtime => {
                "Agent runtime failed; inspect the runtime configuration and provider status before retrying"
            }
            Self::Interrupted => {
                "Previous execution was interrupted; reconcile side effects and remove its baton-inflight marker before resuming"
            }
            Self::MissingSession => "Agent session is missing; manual recovery required",
            Self::ResourceUnavailable => {
                "Pi CLI could not start after 2 retries; system resources are temporarily unavailable"
            }
            Self::ProviderTransient => {
                "Pi provider is temporarily unavailable; inspect retry status and reconcile side effects before retrying"
            }
            Self::ProviderPolicy => {
                "Pi provider rejected the prompt under its content policy; revise the request or provider configuration"
            }
            Self::ProviderAuthentication => "Pi provider authentication failed; check credentials",
            Self::ProviderConfiguration => {
                "Pi configuration is invalid; check the configured model and CLI options"
            }
            Self::InvalidHandoff => {
                "The previous handoff is invalid or too large; inspect the run before recovery"
            }
        }
    }
}
#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct RespondWork {
    pub id: String,
    pub action: WorkHumanAction,
}

#[derive(MediCommand)]
#[medi_command(return_type = Vec<PipelineRun>, error_type = String)]
pub struct ListWork;
#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct GetWork {
    pub id: String,
}
#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct CancelWork {
    pub id: String,
}
#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct ResumeWork {
    pub id: String,
}
#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct BeginWork {
    pub id: String,
    pub revision: u64,
    pub attempt: u32,
}
#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = String)]
pub struct RecoverWork {
    pub id: String,
}

#[medi_handler]
async fn list_work_handler(
    resources: WorkResources,
    _command: ListWork,
) -> Result<Vec<PipelineRun>, String> {
    Ok(resources
        .runs
        .list()?
        .into_iter()
        .filter(|run| {
            run.session
                .as_ref()
                .is_some_and(|s| s.workspace == resources.workspace)
        })
        .collect())
}
#[medi_handler]
async fn get_work_handler(
    resources: WorkResources,
    command: GetWork,
) -> Result<PipelineRun, String> {
    let run = resources.runs.load(&command.id)?;
    validate_workspace(&run, &resources.workspace)?;
    Ok(run)
}
#[medi_handler]
async fn cancel_work_handler(
    resources: WorkResources,
    command: CancelWork,
) -> Result<PipelineRun, String> {
    let run = resources.runs.load(&command.id)?;
    validate_workspace(&run, &resources.workspace)?;
    crate::features::runs::cancel_run(resources.runs.as_ref(), &command.id)
}

fn execution_marker(attempt: u32) -> String {
    format!("External execution started for attempt {attempt}")
}

fn execution_started(run: &PipelineRun, attempt: u32) -> bool {
    let marker = execution_marker(attempt);
    run.events
        .iter()
        .any(|event| event.message.as_deref() == Some(&marker))
}

#[medi_handler]
async fn begin_work_handler(
    resources: WorkResources,
    command: BeginWork,
) -> Result<PipelineRun, String> {
    let _lease = resources.runs.acquire_action_lease(&command.id)?;
    let mut run = resources.runs.load(&command.id)?;
    validate_workspace(&run, &resources.workspace)?;
    if run.status != Status::Running
        || run.session.as_ref().ok_or("Missing session")?.revision != command.revision
        || run
            .steps
            .last()
            .is_none_or(|step| step.status != Status::Running || step.attempt != command.attempt)
    {
        return Err("Work attempt changed before execution".into());
    }
    if execution_started(&run, command.attempt) {
        return Err(
            "Work execution was interrupted; inspect side effects and resume explicitly".into(),
        );
    }
    publish(
        resources.runs.as_ref(),
        &mut run,
        Some(execution_marker(command.attempt)),
    )?;
    Ok(run)
}

#[medi_handler]
async fn recover_work_handler(
    resources: WorkResources,
    command: RecoverWork,
) -> Result<PipelineRun, String> {
    let _lease = resources.runs.acquire_action_lease(&command.id)?;
    let mut run = resources.runs.load(&command.id)?;
    validate_workspace(&run, &resources.workspace)?;
    if run.status != Status::Running
        || run.steps.last().is_none_or(|step| {
            step.status != Status::Running || !execution_started(&run, step.attempt)
        })
    {
        return Ok(run);
    }
    let ended_at = utc_now();
    let step = run.steps.last_mut().ok_or("Missing attempt")?;
    step.status = Status::Failed;
    step.error = Some("Execution interrupted; inspect side effects and resume explicitly".into());
    step.duration_ms = step
        .started_at
        .as_deref()
        .and_then(|start| elapsed_ms(start, &ended_at));
    step.ended_at = Some(ended_at.clone());
    run.status = Status::Failed;
    run.duration_ms = elapsed_ms(&run.started_at, &ended_at);
    run.ended_at = Some(ended_at);
    run.session.as_mut().ok_or("Missing session")?.revision += 1;
    publish(
        resources.runs.as_ref(),
        &mut run,
        Some("Execution interrupted; inspect side effects and resume explicitly".into()),
    )?;
    Ok(run)
}

#[medi_handler]
async fn resume_work_handler(
    resources: WorkResources,
    command: ResumeWork,
) -> Result<PipelineRun, String> {
    let _lease = resources.runs.acquire_action_lease(&command.id)?;
    let mut run = resources.runs.load(&command.id)?;
    validate_workspace(&run, &resources.workspace)?;
    if !matches!(run.status, Status::Failed | Status::Running) {
        return Err("Only failed or interrupted running work can be resumed".into());
    }
    let last = run.steps.last_mut().ok_or("Missing attempt")?;
    if run.status == Status::Running {
        if last.status != Status::Running {
            return Err("No running attempt to resume".into());
        }
        let ended_at = utc_now();
        last.duration_ms = last
            .started_at
            .as_deref()
            .and_then(|start| elapsed_ms(start, &ended_at));
        last.ended_at = Some(ended_at);
        last.status = Status::Failed;
        last.error = Some("Execution interrupted; side effects may have occurred".into());
    } else if last.status != Status::Failed {
        return Err("No failed attempt to resume".into());
    }
    run.status = Status::Running;
    run.ended_at = None;
    run.duration_ms = None;
    run.session.as_mut().ok_or("Missing session")?.revision += 1;
    prepare(&mut run)?;
    let message = publication_message(
        &run,
        "Work resumed explicitly; previous attempt may have side effects",
    )
    .to_owned();
    publish(resources.runs.as_ref(), &mut run, Some(message))?;
    Ok(run)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
    pub run_id: String,
    pub revision: u64,
    pub status: Status,
    pub workspace: PathBuf,
    pub step: Option<PipelineStep>,
    pub attempt: Option<u32>,
    pub agent: Option<Agent>,
    pub skills: Vec<Skill>,
    pub skills_directory: PathBuf,
    pub allowed_outcomes: Vec<Outcome>,
    pub checks: Vec<WorkCheck>,
    pub last_result: Option<WorkResult>,
    /// Explicit human responses in chronological order; later corrections take precedence.
    pub human_decisions: Vec<String>,
    /// Original task accompanying explicit human decisions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_task: Option<String>,
    /// Present only immediately after a human response when prior context was composed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkResult {
    pub step_id: String,
    pub attempt: u32,
    pub output: String,
}

fn bounded(label: &str, text: &str) -> Result<(), String> {
    if text.trim().is_empty() || text.len() > MAX_HANDOFF_BYTES {
        return Err(format!("{label} must be nonempty and at most 16384 bytes"));
    }
    Ok(())
}
fn graph(run: &PipelineRun) -> Result<(&ProjectConfig, &Pipeline), String> {
    let config = run
        .config_snapshot
        .as_ref()
        .ok_or("Run has no configuration snapshot")?;
    let pipeline = config
        .pipelines
        .iter()
        .find(|p| p.id == run.pipeline_id)
        .ok_or("Unknown pipeline")?;
    Ok((config, pipeline))
}
pub fn validate_workspace(run: &PipelineRun, workspace: &std::path::Path) -> Result<(), String> {
    let session = run
        .session
        .as_ref()
        .ok_or("Run is not an external work session")?;
    if session.workspace != workspace {
        return Err("Work session belongs to a different workspace".into());
    }
    Ok(())
}
fn outcomes(step: &PipelineStep) -> Vec<Outcome> {
    if step.transitions.is_empty() {
        if step.is_human() {
            vec![Outcome::Approved, Outcome::Rejected, Outcome::Responded]
        } else {
            vec![Outcome::Completed]
        }
    } else {
        step.transitions.iter().map(|t| t.outcome.clone()).collect()
    }
}
fn publication_message<'a>(run: &'a PipelineRun, fallback: &'a str) -> &'a str {
    run.events
        .iter()
        .rev()
        .find(|event| event.status == Status::LimitExhausted)
        .and_then(|event| event.message.as_deref())
        .unwrap_or(fallback)
}

/// Prepare exactly one attempt. Re-reading `next` never creates another attempt.
fn prepare(run: &mut PipelineRun) -> Result<(), String> {
    if run.status != Status::Running {
        return Ok(());
    }
    let (_, pipeline) = graph(run)?;
    let step = pipeline
        .steps
        .iter()
        .find(|s| Some(&s.id) == run.current_step.as_ref())
        .ok_or("Unknown current step")?
        .clone();
    if run
        .steps
        .iter()
        .any(|s| s.step_id == step.id && matches!(s.status, Status::Running | Status::WaitingHuman))
    {
        return Ok(());
    }
    let global_limit = run
        .session
        .as_ref()
        .map(|session| session.max_attempts)
        .or_else(|| {
            run.config_snapshot
                .as_ref()
                .and_then(|config| config.pipelines.iter().find(|p| p.id == run.pipeline_id))
                .and_then(|pipeline| pipeline.max_attempts)
        });
    if let Some(limit) = global_limit
        && run
            .steps
            .iter()
            .filter(|s| s.status != Status::Pending)
            .count()
            >= limit as usize
    {
        run.status = Status::LimitExhausted;
        let ended_at = utc_now();
        run.duration_ms = elapsed_ms(&run.started_at, &ended_at);
        run.ended_at = Some(ended_at.clone());
        run.events.push(crate::domain::pipeline::RunEvent {
            run_id: run.id.clone(),
            status: Status::LimitExhausted,
            occurred_at: ended_at,
            message: Some(format!("Pipeline exhausted maxAttempts limit of {limit}")),
        });
        return Ok(());
    }
    if let Some(limit) = step.max_attempts
        && run
            .steps
            .iter()
            .filter(|s| s.step_id == step.id && s.status != Status::Pending)
            .count()
            >= limit as usize
    {
        run.status = Status::LimitExhausted;
        let ended_at = utc_now();
        run.duration_ms = elapsed_ms(&run.started_at, &ended_at);
        run.ended_at = Some(ended_at.clone());
        run.events.push(crate::domain::pipeline::RunEvent {
            run_id: run.id.clone(),
            status: Status::LimitExhausted,
            occurred_at: ended_at,
            message: Some(format!(
                "Step {} exhausted maxAttempts limit of {}",
                step.id, limit
            )),
        });
        return Ok(());
    }
    let attempt = run.steps.iter().map(|s| s.attempt).max().unwrap_or(0) + 1;
    let status = if step.is_human() {
        Status::WaitingHuman
    } else {
        Status::Running
    };
    run.pending_actions = if step.is_human() {
        outcomes(&step)
    } else {
        vec![]
    };
    run.status = status.clone();
    let agent_id = step.agent_id().to_owned();
    run.steps.push(StepRun {
        agent_id: agent_id.clone(),
        step_id: step.id,
        status,
        input: None,
        output: None,
        error: None,
        outcome: None,
        structured_handoff: None,
        selected_transition: None,
        attempt,
        started_at: Some(utc_now()),
        ended_at: None,
        duration_ms: None,
        model: run
            .config_snapshot
            .as_ref()
            .and_then(|config| config.agents.iter().find(|agent| agent.id == agent_id))
            .and_then(|agent| agent.model.clone()),
        usage: None,
    });
    Ok(())
}

#[medi_handler]
async fn start_work_handler(
    resources: WorkResources,
    command: StartWork,
) -> Result<PipelineRun, String> {
    bounded("Task", &command.task)?;
    if command.source.as_ref().is_some_and(|s| s.len() > 4096) {
        return Err("Source exceeds 4096 bytes".into());
    }
    let config = match resources.config.load().await? {
        ConfigState::Ready(snapshot) => snapshot.config,
        ConfigState::Missing => {
            return Err(
                "Project configuration is missing; configure a pipeline before starting work"
                    .into(),
            );
        }
        ConfigState::Invalid(error) => return Err(error),
    };
    config.validate()?;
    let pipeline_index = match command.pipeline_id {
        Some(id) => config.pipelines.iter().position(|p| p.id == id).ok_or("Unknown pipeline ID")?,
        None if config.pipelines.len() == 1 => 0,
        None => return Err("Select a pipeline with --pipeline (project must have exactly one pipeline to infer it)".into()),
    };
    let mut config = config;
    let pipeline = &mut config.pipelines[pipeline_index];
    let max_attempts = *pipeline.max_attempts.get_or_insert(50);
    for check in &command.checks {
        bounded("Check command", &check.command)?;
        if !pipeline
            .steps
            .iter()
            .any(|s| s.id == check.step_id && !s.is_human())
        {
            return Err("Checks must reference an agent step in the selected pipeline".into());
        }
    }
    // Validate assigned skills before persisting the run; no model/provider credentials are needed.
    let skills = resources.skills.list()?;
    for step in pipeline
        .steps
        .iter()
        .filter(|s| !s.is_human() && s.command().is_none())
    {
        let agent = config
            .agents
            .iter()
            .find(|a| a.id == step.agent_id())
            .ok_or("Unknown agent")?;
        for id in &agent.skill_ids {
            if !skills.iter().any(|s| &s.id == id) {
                return Err(format!("Assigned skill {id:?} is missing"));
            }
        }
    }
    let mut run = PipelineRun {
        id: command.id,
        task_id: None,
        worktree: None,
        setup_error: None,
        pipeline_id: pipeline.id.clone(),
        task: command.task,
        started_at: utc_now(),
        ended_at: None,
        duration_ms: None,
        events: vec![],
        trace: vec![],
        status: Status::Running,
        steps: vec![],
        current_step: Some(pipeline.steps[0].id.clone()),
        handoff: None,
        pending_actions: vec![],
        config_snapshot: Some(config),
        session: Some(WorkSession {
            workspace: resources.workspace,
            source: command.source,
            revision: 0,
            max_attempts,
            checks: command.checks,
            submissions: vec![],
        }),
    };
    prepare(&mut run)?;
    resources.runs.create(&run)?;
    publish(
        resources.runs.as_ref(),
        &mut run,
        Some("External work session started; workspace remains externally owned".into()),
    )?;
    Ok(run)
}

#[medi_handler]
async fn next_work_handler(
    resources: WorkResources,
    command: NextWork,
) -> Result<WorkItem, String> {
    let run = resources.runs.load(&command.id)?;
    validate_workspace(&run, &resources.workspace)?;
    let session = run.session.as_ref().ok_or("Missing session")?;
    let (config, pipeline) = graph(&run)?;
    let active = matches!(run.status, Status::Running | Status::WaitingHuman);
    let step = if active {
        pipeline
            .steps
            .iter()
            .find(|s| Some(&s.id) == run.current_step.as_ref())
            .cloned()
    } else {
        None
    };
    let agent = step
        .as_ref()
        .filter(|s| !s.is_human())
        .and_then(|s| config.agents.iter().find(|a| a.id == s.agent_id()))
        .cloned();
    let mut skills = vec![];
    if let Some(agent) = &agent {
        let available = resources.skills.list()?;
        for id in &agent.skill_ids {
            skills.push(
                available
                    .iter()
                    .find(|s| &s.id == id)
                    .ok_or_else(|| format!("Assigned skill {id:?} is missing"))?
                    .clone(),
            );
        }
    }
    let human_decisions = crate::features::pipeline_engine::human_decisions(&run, pipeline);
    Ok(WorkItem {
        run_id: run.id.clone(),
        revision: session.revision,
        status: run.status.clone(),
        workspace: session.workspace.clone(),
        allowed_outcomes: step.as_ref().map(outcomes).unwrap_or_default(),
        attempt: if active {
            run.steps.last().map(|s| s.attempt)
        } else {
            None
        },
        original_task: (!human_decisions.is_empty()).then(|| run.task.clone()),
        human_decisions,
        last_result: run.steps.iter().rev().find_map(|s| {
            s.output.as_ref().map(|output| WorkResult {
                step_id: s.step_id.clone(),
                attempt: s.attempt,
                output: output.clone(),
            })
        }),
        handoff: run
            .steps
            .iter()
            .rev()
            .find(|attempt| attempt.output.is_some())
            .and_then(|attempt| {
                let output = attempt.output.as_ref()?;
                let previous_step = pipeline
                    .steps
                    .iter()
                    .find(|step| step.id == attempt.step_id)?;
                (previous_step.is_human()
                    && run
                        .handoff
                        .as_ref()
                        .is_some_and(|handoff| handoff != output))
                .then(|| run.handoff.clone())
                .flatten()
            }),
        checks: session
            .checks
            .iter()
            .filter(|c| step.as_ref().is_some_and(|s| s.id == c.step_id))
            .cloned()
            .collect(),
        step,
        agent,
        skills,
        skills_directory: session.workspace.join(".agents/skills"),
    })
}

fn validate_submission(
    run: &PipelineRun,
    revision: u64,
    step_id: &str,
    attempt: u32,
    human: bool,
    outcome: &Outcome,
) -> Result<usize, String> {
    let session = run
        .session
        .as_ref()
        .ok_or("Run is not an external work session")?;
    if session.revision != revision {
        return Err("Stale submission revision; fetch next again".into());
    }
    let expected = if human {
        Status::WaitingHuman
    } else {
        Status::Running
    };
    if run.status != expected {
        return Err("Run is not accepting this action (human decisions require respond)".into());
    }
    let (_, pipeline) = graph(run)?;
    let step = pipeline
        .steps
        .iter()
        .find(|s| Some(&s.id) == run.current_step.as_ref())
        .ok_or("Unknown current step")?;
    if step.is_human() != human || step.id != step_id {
        return Err("Submission does not match the current step".into());
    }
    let index = run.steps.len().checked_sub(1).ok_or("Missing attempt")?;
    if run.steps[index].attempt != attempt {
        return Err("Stale attempt; fetch next again".into());
    }
    if !outcomes(step).contains(outcome)
        && !(step.command().is_some() && *outcome == Outcome::Failed)
    {
        return Err("Outcome is not allowed by this step".into());
    }
    Ok(index)
}

fn complete(
    run: &mut PipelineRun,
    index: usize,
    outcome: Outcome,
    summary: String,
    structured_handoff: Option<crate::domain::pipeline::StructuredHandoff>,
) -> Result<(), String> {
    let (_, pipeline) = graph(run)?;
    let pipeline = pipeline.clone();
    let human = run.steps[index].status == Status::WaitingHuman;
    run.steps[index].status = Status::Succeeded;
    let ended_at = utc_now();
    run.steps[index].duration_ms = run.steps[index]
        .started_at
        .as_deref()
        .and_then(|started| elapsed_ms(started, &ended_at));
    run.steps[index].ended_at = Some(ended_at);
    run.steps[index].output = Some(summary.clone());
    run.steps[index].structured_handoff = structured_handoff;
    run.handoff = Some(if human {
        human_handoff(run.handoff.as_deref(), &summary)?
    } else {
        summary
    });
    run.steps[index].outcome = Some(outcome.clone());
    run.status = Status::Running;
    run.pending_actions.clear();
    route(run, &pipeline, index, outcome)?;
    run.session.as_mut().ok_or("Missing session")?.revision += 1;
    prepare(run)?;
    if !matches!(run.status, Status::Running | Status::WaitingHuman) {
        let ended_at = utc_now();
        run.duration_ms = elapsed_ms(&run.started_at, &ended_at);
        run.ended_at = Some(ended_at);
    }
    Ok(())
}

#[medi_handler]
async fn submit_work_handler(
    resources: WorkResources,
    command: SubmitWork,
) -> Result<PipelineRun, String> {
    tokio::task::spawn_blocking(move || {
        let _lease = resources.runs.acquire_action_lease(&command.id)?;
        let mut run = resources.runs.load(&command.id)?;
        validate_workspace(&run, &resources.workspace)?;
        let result = command.result;
        let index = validate_submission(
            &run,
            result.revision,
            &result.step_id,
            result.attempt,
            false,
            &result.outcome,
        )?;
        bounded("Summary", &result.summary)?;
        if let Some(handoff) = &result.structured_handoff {
            handoff.validate()?;
        }
        let mut routed = run.clone();
        let (_, pipeline) = graph(&run)?;
        route(&mut routed, pipeline, index, result.outcome.clone())?;
        let repeated = result.outcome == Outcome::Blocked
            && repeated_blocker(&run, pipeline, index, &result.summary);
        let session = run.session.as_ref().ok_or("Missing session")?;
        let mut checks = vec![];
        if matches!(
            result.outcome,
            Outcome::Completed | Outcome::Approved | Outcome::Responded
        ) || routed.status == Status::Succeeded
        {
            for check in session
                .checks
                .iter()
                .filter(|c| c.step_id == result.step_id)
            {
                let checked = resources.checks.run(&session.workspace, &check.command)?;
                if checked.exit_code != Some(0) {
                    return Err(format!(
                        "Configured check failed: {}\n{}",
                        checked.command, checked.output
                    ));
                }
                checks.push(checked);
            }
        }
        run.steps[index].input = command.input;
        let mut trace = command.trace;
        for entry in &mut trace {
            entry.attempt = Some(result.attempt);
        }
        append_bounded_trace(&mut run, trace);
        run.session
            .as_mut()
            .ok_or("Missing session")?
            .submissions
            .push(WorkRecord {
                attempt: result.attempt,
                checks,
            });
        let step_id = result.step_id.clone();
        complete(
            &mut run,
            index,
            result.outcome,
            result.summary,
            result.structured_handoff,
        )?;
        if repeated {
            publish(
                resources.runs.as_ref(),
                &mut run,
                Some(repeated_blocker_message(&step_id)),
            )?;
        }
        let message = publication_message(&run, "External worker submission accepted").to_owned();
        publish(resources.runs.as_ref(), &mut run, Some(message))?;
        Ok(run)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[medi_handler]
async fn fail_work_handler(
    resources: WorkResources,
    command: FailWork,
) -> Result<PipelineRun, String> {
    let _lease = resources.runs.acquire_action_lease(&command.id)?;
    let mut run = resources.runs.load(&command.id)?;
    validate_workspace(&run, &resources.workspace)?;
    let (_, pipeline) = graph(&run)?;
    let step = pipeline
        .steps
        .iter()
        .find(|s| Some(&s.id) == run.current_step.as_ref())
        .ok_or("Unknown current step")?;
    let permitted = outcomes(step)
        .into_iter()
        .next()
        .ok_or("No permitted outcomes")?;
    let index = validate_submission(
        &run,
        command.revision,
        &command.step_id,
        command.attempt,
        false,
        &permitted,
    )?;
    run.steps[index].status = Status::Failed;
    let ended_at = utc_now();
    run.steps[index].duration_ms = run.steps[index]
        .started_at
        .as_deref()
        .and_then(|started| elapsed_ms(started, &ended_at));
    run.steps[index].ended_at = Some(ended_at);
    run.steps[index].input = Some(command.input);
    let mut trace = command.trace;
    for entry in &mut trace {
        entry.attempt = Some(command.attempt);
    }
    append_bounded_trace(&mut run, trace);
    let message = command.failure.message();
    run.steps[index].error = Some(message.into());
    run.status = Status::Failed;
    let ended_at = utc_now();
    run.duration_ms = elapsed_ms(&run.started_at, &ended_at);
    run.ended_at = Some(ended_at);
    run.session.as_mut().ok_or("Missing session")?.revision += 1;
    publish(resources.runs.as_ref(), &mut run, Some(message.into()))?;
    Ok(run)
}

#[medi_handler]
async fn respond_work_handler(
    resources: WorkResources,
    command: RespondWork,
) -> Result<PipelineRun, String> {
    let _lease = resources.runs.acquire_action_lease(&command.id)?;
    let mut run = resources.runs.load(&command.id)?;
    validate_workspace(&run, &resources.workspace)?;
    let action = command.action;
    let index = validate_submission(
        &run,
        action.revision,
        &action.step_id,
        action.attempt,
        true,
        &action.outcome,
    )?;
    if action.response.len() > MAX_HANDOFF_BYTES {
        return Err("Response exceeds 16384 bytes".into());
    }
    complete(&mut run, index, action.outcome, action.response, None)?;
    let message = publication_message(&run, "Explicit human response received").to_owned();
    publish(resources.runs.as_ref(), &mut run, Some(message))?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_submission_preserves_structured_handoff_for_next_step_and_recovery() {
        use crate::domain::pipeline::{Finding, FindingStatus, StructuredHandoff};

        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"external", "pipelineId":"p", "task":"task", "startedAt":"now",
            "status":"running", "currentStep":"review",
            "steps":[{"stepId":"review","agentId":"reviewer","status":"running","attempt":1}],
            "configSnapshot":{"schemaVersion":7,"agents":[{"id":"reviewer","systemPrompt":"prompt","credentialId":"credential","skillIds":[]}],"pipelines":[{"id":"p","steps":[
                {"id":"review","name":"Review","type":"agent","agentId":"reviewer","transitions":[{"outcome":"approved","to":"build"}]},
                {"id":"build","name":"Build","type":"human"}
            ]}]},
            "session":{"workspace":"/tmp/work","revision":0,"maxAttempts":10,"checks":[],"submissions":[]}
        })).unwrap();
        let handoff = StructuredHandoff {
            version: 1,
            summary: "Fix the reported issue".into(),
            findings: vec![Finding {
                id: "F-1".into(),
                status: FindingStatus::Open,
                description: "Validate the response".into(),
            }],
            verification: vec![],
            next_action: Some("Add the validation".into()),
        };
        complete(
            &mut run,
            0,
            Outcome::Approved,
            "One issue found".into(),
            Some(handoff.clone()),
        )
        .unwrap();

        let input = crate::features::pipeline_engine::step_input(
            &run,
            &run.config_snapshot.as_ref().unwrap().pipelines[0],
        )
        .unwrap();
        assert!(input.contains("One issue found"));
        assert!(input.contains("Validate the response"));
        let recovered: PipelineRun =
            serde_json::from_slice(&serde_json::to_vec(&run).unwrap()).unwrap();
        assert_eq!(recovered.steps[0].structured_handoff, Some(handoff));
    }

    #[test]
    fn legacy_submission_without_structured_handoff_deserializes() {
        let submission: WorkSubmission = serde_json::from_value(serde_json::json!({
            "revision":0,"stepId":"review","attempt":1,"outcome":"approved","summary":"ok"
        }))
        .unwrap();
        assert!(submission.structured_handoff.is_none());
    }

    #[test]
    fn external_human_completion_keeps_prior_handoff_and_response() {
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"external", "pipelineId":"p", "task":"task", "startedAt":"now",
            "status":"waitingHuman", "currentStep":"help", "handoff":"prior blocker diagnosis",
            "steps":[{"stepId":"help","agentId":"","status":"waitingHuman","attempt":1}],
            "configSnapshot":{"schemaVersion":7,"agents":[],"pipelines":[{"id":"p","steps":[
                {"id":"help","name":"Help","type":"human","transitions":[{"outcome":"responded","to":"build"}]},
                {"id":"build","name":"Build","type":"human"}
            ]}]},
            "session":{"workspace":"/tmp/work","revision":0,"maxAttempts":10,"checks":[],"submissions":[]}
        })).unwrap();
        complete(
            &mut run,
            0,
            Outcome::Responded,
            "use approved mirror".into(),
            None,
        )
        .unwrap();
        assert_eq!(run.status, Status::WaitingHuman);
        assert_eq!(run.current_step.as_deref(), Some("build"));
        let handoff = run.handoff.as_deref().unwrap();
        assert!(handoff.contains("prior blocker diagnosis"));
        assert!(handoff.contains("use approved mirror"));
        assert_eq!(
            run.steps[0]
                .selected_transition
                .as_ref()
                .and_then(|edge| edge.to.as_deref()),
            Some("build")
        );
    }

    #[test]
    fn limit_reason_is_used_for_external_publication_message() {
        let run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"external", "pipelineId":"p", "task":"task", "startedAt":"now",
            "status":"limitExhausted", "steps":[], "events":[{
                "runId":"external", "status":"limitExhausted", "occurredAt":"now",
                "message":"Step review exhausted maxAttempts limit of 2"
            }]
        }))
        .unwrap();
        assert_eq!(
            publication_message(&run, "External worker submission accepted"),
            "Step review exhausted maxAttempts limit of 2"
        );
    }

    #[test]
    fn external_prepare_uses_frozen_session_limit_and_legacy_session_precedence() {
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"external", "pipelineId":"p", "task":"task", "startedAt":"2026-01-01T00:00:00Z",
            "status":"running", "currentStep":"next", "steps":[{
                "stepId":"old", "agentId":"agent", "status":"failed", "attempt":1
            }],
            "configSnapshot":{"schemaVersion":7,"agents":[],"pipelines":[{"id":"p","maxAttempts":100,"steps":[
                {"id":"old","name":"Old","type":"human"},{"id":"next","name":"Next","type":"human"}
            ]}]},
            "session":{"workspace":"/tmp/work","revision":0,"maxAttempts":1,"checks":[],"submissions":[]}
        })).unwrap();
        prepare(&mut run).unwrap();
        assert_eq!(run.status, Status::LimitExhausted);
        assert!(
            run.events
                .iter()
                .any(|e| e.message.as_deref() == Some("Pipeline exhausted maxAttempts limit of 1"))
        );
        assert!(run.ended_at.is_some());
    }

    #[test]
    fn external_prepare_exhausts_per_step_limit_and_keeps_reason_in_persisted_events() {
        let mut run: PipelineRun = serde_json::from_value(serde_json::json!({
            "id":"external", "pipelineId":"p", "task":"task", "startedAt":"2026-01-01T00:00:00Z",
            "status":"running", "currentStep":"step", "steps":[{
                "stepId":"step", "agentId":"agent", "status":"failed", "attempt":1
            }],
            "configSnapshot":{"schemaVersion":7,"agents":[],"pipelines":[{"id":"p","steps":[{
                "id":"step","name":"Step","type":"human","maxAttempts":1
            }]}]},
            "session":{"workspace":"/tmp/work","revision":0,"maxAttempts":10,"checks":[],"submissions":[]}
        })).unwrap();
        prepare(&mut run).unwrap();
        assert_eq!(run.status, Status::LimitExhausted);
        assert!(run.events.iter().any(|event| {
            event
                .message
                .as_deref()
                .is_some_and(|message| message.contains("step") && message.contains("1"))
        }));
        assert!(run.ended_at.is_some());
    }
}

medi_module! {
    pub manifest work_feature;
    resources { work_resources: crate::features::work::WorkResources; }
    commands {
        crate::features::work::ListWork => crate::features::work::list_work_handler;
        crate::features::work::GetWork => crate::features::work::get_work_handler;
        crate::features::work::CancelWork => crate::features::work::cancel_work_handler;
        crate::features::work::ResumeWork => crate::features::work::resume_work_handler;
        crate::features::work::BeginWork => crate::features::work::begin_work_handler;
        crate::features::work::RecoverWork => crate::features::work::recover_work_handler;
        crate::features::work::StartWork => crate::features::work::start_work_handler;
        crate::features::work::NextWork => crate::features::work::next_work_handler;
        crate::features::work::SubmitWork => crate::features::work::submit_work_handler;
        crate::features::work::FailWork => crate::features::work::fail_work_handler;
        crate::features::work::RespondWork => crate::features::work::respond_work_handler;
    }
}
