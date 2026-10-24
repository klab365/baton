//! Sequential orchestration. Inputs and outputs are UTF-8 byte bounded; no implicit retries.
use crate::project::{Agent, ProjectConfig};
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub const MAX_HANDOFF_BYTES: usize = 16 * 1024;

pub trait AgentRuntime {
    fn execute(&mut self, agent: &Agent, input: &str) -> Result<String, String>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PipelineRun {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub pipeline_id: String,
    pub task: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub events: Vec<RunEvent>,
    pub status: Status,
    pub steps: Vec<StepRun>,
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
    fn create(&self, run: &PipelineRun) -> Result<(), String>;
    fn save(&self, run: &PipelineRun) -> Result<(), String>;
    fn load(&self, id: &str) -> Result<PipelineRun, String>;
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
fn finish_run(run: &mut PipelineRun, started: Instant) {
    run.ended_at = Some(utc_now());
    run.duration_ms = Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
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
        pipeline_id: pipeline.id.clone(),
        task: task.into(),
        started_at: utc_now(),
        ended_at: None,
        duration_ms: None,
        events: Vec::new(),
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
            })
            .collect(),
    };
    repository.create(&run)?;
    publish(repository, &mut run, Some("Run started".into()))?;
    let mut input = task.to_owned();
    for (index, step) in pipeline.steps.iter().enumerate() {
        run.steps[index].status = Status::Running;
        run.steps[index].input = Some(input.clone());
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
        let result = runtime.execute(agent, &input).and_then(|output| {
            if output.len() > MAX_HANDOFF_BYTES {
                Err("Agent output exceeds 16384 bytes".into())
            } else {
                Ok(output)
            }
        });
        // Cancellation is cooperative: an in-flight adapter is allowed to return,
        // but its result is discarded and no subsequent step starts.
        if let Ok(current) = repository.load(&run.id)
            && current.status == Status::Cancelled
        {
            return Ok(current);
        }
        match result {
            Ok(output) => {
                run.steps[index].status = Status::Succeeded;
                run.steps[index].output = Some(output.clone());
                input = output;
            }
            Err(mut error) => {
                if error.len() > MAX_HANDOFF_BYTES {
                    let mut end = MAX_HANDOFF_BYTES;
                    while !error.is_char_boundary(end) {
                        end -= 1;
                    }
                    error.truncate(end);
                }
                run.steps[index].status = Status::Failed;
                run.steps[index].error = Some(error);
                run.status = Status::Failed;
                finish_run(&mut run, started_clock);
                publish(repository, &mut run, Some("Run failed".into()))?;
                return Ok(run);
            }
        }
        publish(
            repository,
            &mut run,
            Some(format!("Step {} completed", step.id)),
        )?;
    }
    run.status = Status::Succeeded;
    finish_run(&mut run, started_clock);
    publish(repository, &mut run, Some("Run completed".into()))?;
    Ok(run)
}
