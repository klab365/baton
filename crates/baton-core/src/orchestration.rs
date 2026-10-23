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
    pub pipeline_id: String,
    pub task: String,
    /// UTC wall-clock time when the run was created, in RFC 3339 format.
    pub started_at: String,
    /// UTC wall-clock time when the run reached a terminal state.
    pub ended_at: Option<String>,
    /// Monotonic elapsed time in milliseconds; remains absent for interrupted runs.
    pub duration_ms: Option<u64>,
    pub status: Status,
    pub steps: Vec<StepRun>,
}
pub trait RunRepository {
    fn create(&self, run: &PipelineRun) -> Result<(), String>;
    fn save(&self, run: &PipelineRun) -> Result<(), String>;
    fn load(&self, id: &str) -> Result<PipelineRun, String>;
}

fn finish_run(run: &mut PipelineRun, started: Instant) {
    run.ended_at = Some(utc_now());
    run.duration_ms = Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
}

pub fn utc_now() -> String {
    // SystemTime is UTC-based; format as RFC 3339 with millisecond precision.
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

// Gregorian civil date from days since Unix epoch.
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
    config.validate()?;
    if task.trim().is_empty() || task.len() > MAX_HANDOFF_BYTES {
        return Err("Task must be nonempty and at most 16384 bytes".into());
    }
    let pipeline = config
        .pipelines
        .iter()
        .find(|p| p.id == pipeline_id)
        .ok_or("Unknown pipeline ID")?;
    let started_at = utc_now();
    let started_clock = Instant::now();
    let mut run = PipelineRun {
        id: run_id.into(),
        pipeline_id: pipeline.id.clone(),
        task: task.into(),
        started_at,
        ended_at: None,
        duration_ms: None,
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
    let mut input = task.to_owned();
    for (index, step) in pipeline.steps.iter().enumerate() {
        run.steps[index].status = Status::Running;
        run.steps[index].input = Some(input.clone());
        repository.save(&run)?;
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
        match result {
            Ok(output) => {
                run.steps[index].status = Status::Succeeded;
                run.steps[index].output = Some(output.clone());
                input = output;
            }
            Err(error) => {
                let mut error = error;
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
                repository.save(&run)?;
                return Ok(run);
            }
        }
        repository.save(&run)?;
    }
    run.status = Status::Succeeded;
    finish_run(&mut run, started_clock);
    repository.save(&run)?;
    Ok(run)
}
