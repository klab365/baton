//! Externally executed steps use the same run records and routing rules as managed runtimes.
use crate::{
    domain::{
        pipeline::{PipelineRun, Status, StepRun, transitions::route},
        project::{Agent, ConfigState, Outcome, Pipeline, PipelineStep, ProjectConfig, Skill},
        work::*,
    },
    features::pipeline_engine::{publish, utc_now},
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
    pub max_attempts: u32,
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
        if step.human {
            vec![Outcome::Approved, Outcome::Rejected, Outcome::Responded]
        } else {
            vec![Outcome::Completed]
        }
    } else {
        step.transitions.iter().map(|t| t.outcome.clone()).collect()
    }
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
    let attempt = run.steps.iter().map(|s| s.attempt).max().unwrap_or(0) + 1;
    if attempt > run.session.as_ref().ok_or("Missing session")?.max_attempts {
        run.status = Status::LimitExhausted;
        run.ended_at = Some(utc_now());
        return Ok(());
    }
    let status = if step.human {
        Status::WaitingHuman
    } else {
        Status::Running
    };
    run.pending_actions = if step.human { outcomes(&step) } else { vec![] };
    run.status = status.clone();
    run.steps.push(StepRun {
        step_id: step.id,
        agent_id: step.agent_id,
        status,
        input: None,
        output: None,
        error: None,
        outcome: None,
        selected_transition: None,
        attempt,
    });
    Ok(())
}

#[medi_handler]
async fn start_work_handler(
    resources: WorkResources,
    command: StartWork,
) -> Result<PipelineRun, String> {
    bounded("Task", &command.task)?;
    if command.max_attempts == 0 || command.max_attempts > 1000 {
        return Err("maxAttempts must be between 1 and 1000".into());
    }
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
    let pipeline = match command.pipeline_id {
        Some(id) => config.pipelines.iter().find(|p| p.id == id).ok_or("Unknown pipeline ID")?,
        None if config.pipelines.len() == 1 => &config.pipelines[0],
        None => return Err("Select a pipeline with --pipeline (project must have exactly one pipeline to infer it)".into()),
    };
    if pipeline.steps.iter().any(|step| step.command.is_some()) {
        return Err("Shell steps are not supported in external work sessions".into());
    }
    for check in &command.checks {
        bounded("Check command", &check.command)?;
        if !pipeline
            .steps
            .iter()
            .any(|s| s.id == check.step_id && !s.human)
        {
            return Err("Checks must reference an agent step in the selected pipeline".into());
        }
    }
    // Validate assigned skills before persisting the run; no model/provider credentials are needed.
    let skills = resources.skills.list()?;
    for step in pipeline.steps.iter().filter(|s| !s.human) {
        let agent = config
            .agents
            .iter()
            .find(|a| a.id == step.agent_id)
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
            max_attempts: command.max_attempts,
            checks: command.checks,
            submissions: vec![],
        }),
    };
    prepare(&mut run)?;
    resources.runs.prepare_run_memory(&run)?;
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
        .filter(|s| !s.human)
        .and_then(|s| config.agents.iter().find(|a| a.id == s.agent_id))
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
    Ok(WorkItem {
        run_id: run.id.clone(),
        revision: session.revision,
        status: run.status.clone(),
        workspace: session.workspace.clone(),
        allowed_outcomes: step
            .as_ref()
            .map(|step| {
                let mut allowed = outcomes(step);
                if !step.human {
                    allowed.push(Outcome::Progress);
                }
                allowed
            })
            .unwrap_or_default(),
        attempt: if active {
            run.steps.last().map(|s| s.attempt)
        } else {
            None
        },
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
    if step.human != human || step.id != step_id {
        return Err("Submission does not match the current step".into());
    }
    let index = run.steps.len().checked_sub(1).ok_or("Missing attempt")?;
    if run.steps[index].attempt != attempt {
        return Err("Stale attempt; fetch next again".into());
    }
    if !outcomes(step).contains(outcome) && (human || *outcome != Outcome::Progress) {
        return Err("Outcome is not allowed by this step".into());
    }
    Ok(index)
}

fn complete(
    run: &mut PipelineRun,
    index: usize,
    outcome: Outcome,
    summary: String,
) -> Result<(), String> {
    let (_, pipeline) = graph(run)?;
    let pipeline = pipeline.clone();
    run.steps[index].status = Status::Succeeded;
    run.steps[index].output = Some(summary.clone());
    run.steps[index].outcome = Some(outcome.clone());
    run.status = Status::Running;
    run.pending_actions.clear();
    route(run, &pipeline, index, outcome)?;
    run.session.as_mut().ok_or("Missing session")?.revision += 1;
    prepare(run)?;
    if !matches!(run.status, Status::Running | Status::WaitingHuman) {
        run.ended_at = Some(utc_now());
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
        resources.runs.prepare_run_memory(&run)?;
        let mut routed = run.clone();
        if result.outcome != Outcome::Progress {
            let (_, pipeline) = graph(&run)?;
            route(&mut routed, pipeline, index, result.outcome.clone())?;
        }
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
        run.session
            .as_mut()
            .ok_or("Missing session")?
            .submissions
            .push(WorkRecord {
                attempt: result.attempt,
                checks,
            });
        let is_progress = result.outcome == Outcome::Progress;
        if is_progress {
            run.steps[index].status = Status::Succeeded;
            run.steps[index].output = Some(result.summary);
            run.steps[index].outcome = Some(Outcome::Progress);
            run.session.as_mut().ok_or("Missing session")?.revision += 1;
            prepare(&mut run)?;
        } else {
            complete(&mut run, index, result.outcome, result.summary)?;
        }
        let message = if run.status == Status::LimitExhausted && is_progress {
            "External work stopped after reaching the configured attempt limit"
        } else {
            "External worker submission accepted"
        };
        publish(resources.runs.as_ref(), &mut run, Some(message.into()))?;
        Ok(run)
    })
    .await
    .map_err(|e| e.to_string())?
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
    complete(&mut run, index, action.outcome, action.response)?;
    publish(
        resources.runs.as_ref(),
        &mut run,
        Some("Explicit human response received".into()),
    )?;
    Ok(run)
}

medi_module! {
    pub manifest work_feature;
    resources { work_resources: crate::features::work::WorkResources; }
    commands {
        crate::features::work::ListWork => crate::features::work::list_work_handler;
        crate::features::work::GetWork => crate::features::work::get_work_handler;
        crate::features::work::CancelWork => crate::features::work::cancel_work_handler;
        crate::features::work::StartWork => crate::features::work::start_work_handler;
        crate::features::work::NextWork => crate::features::work::next_work_handler;
        crate::features::work::SubmitWork => crate::features::work::submit_work_handler;
        crate::features::work::RespondWork => crate::features::work::respond_work_handler;
    }
}
