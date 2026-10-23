//! Sequential orchestration. Inputs and outputs are UTF-8 byte bounded; no implicit retries.
use crate::project::{Agent, ExecutionConfig, ProjectConfig};
use rig_core::{completion::CompletionRequest, providers::openai::OpenAI};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const MAX_HANDOFF_BYTES: usize = 16 * 1024;

pub trait AgentRuntime {
    fn execute(
        &mut self,
        agent: &Agent,
        execution: &ExecutionConfig,
        input: &str,
    ) -> Result<String, String>;
}

/// Deterministic spike adapter: records calls and prefixes each input with the agent ID.
#[derive(Default)]
pub struct FakeRuntime {
    pub calls: Vec<(String, String)>,
    pub fail_agent: Option<String>,
}
impl AgentRuntime for FakeRuntime {
    fn execute(
        &mut self,
        agent: &Agent,
        _: &ExecutionConfig,
        input: &str,
    ) -> Result<String, String> {
        self.calls.push((agent.id.clone(), input.into()));
        if self.fail_agent.as_deref() == Some(&agent.id) {
            return Err("Simulated agent failure".into());
        }
        Ok(format!("{}: {input}", agent.id))
    }
}

/// OpenAI implementation of the `AgentRuntime` port using Rig.
///
/// It intentionally supports text completion only. Tool and skill references are
/// rejected rather than being silently omitted from a provider request.
#[derive(Default)]
pub struct OpenAiRigRuntime;

impl AgentRuntime for OpenAiRigRuntime {
    fn execute(
        &mut self,
        agent: &Agent,
        _: &ExecutionConfig,
        input: &str,
    ) -> Result<String, String> {
        if agent.model.provider != "openai" {
            return Err(format!(
                "OpenAI runtime requires agent {} to use model.provider \\\"openai\\\"",
                agent.id
            ));
        }
        if !agent.tool_ids.is_empty() {
            return Err(format!(
                "OpenAI runtime does not support configured tools for agent {}",
                agent.id
            ));
        }
        if !agent.skill_ids.is_empty() {
            return Err(format!(
                "OpenAI runtime does not support configured skills for agent {}",
                agent.id
            ));
        }

        let client = OpenAI::from_env().map_err(|_| {
            "OpenAI credentials are unavailable; set OPENAI_API_KEY in your local environment"
                .to_owned()
        })?;
        let request = CompletionRequest::new(input)
            .preamble(agent_instructions(agent))
            .max_tokens(agent.model.max_output_tokens.map(u64::from));
        let model = client.completion(&agent.model.model);
        let response = run_async(async {
            tokio::time::timeout(Duration::from_secs(60), model.call(request)).await
        })
        .map_err(|_| "OpenAI request timed out after 60 seconds".to_owned())?
        .map_err(|_| {
            "OpenAI request failed; check API authentication, model availability, rate limits, and network access"
                .to_owned()
        })?;
        Ok(response.text())
    }
}

fn agent_instructions(agent: &Agent) -> String {
    if agent.rules.is_empty() {
        return agent.system_prompt.clone();
    }
    format!(
        "{}\n\nRules:\n{}",
        agent.system_prompt,
        agent
            .rules
            .iter()
            .map(|rule| format!("- {rule}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn run_async<T>(future: impl std::future::Future<Output = T>) -> T {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(future))
    } else {
        tokio::runtime::Runtime::new()
            .expect("Tokio runtime can be created for OpenAI execution")
            .block_on(future)
    }
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

fn utc_now() -> String {
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
    // Reject unsupported modes before creating a run or invoking any agents.
    if pipeline
        .steps
        .iter()
        .any(|s| !matches!(s.execution, ExecutionConfig::Local { .. }))
    {
        return Err("Only local execution is supported in this spike".into());
    }
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
        let result = runtime
            .execute(agent, &step.execution, &input)
            .and_then(|output| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ModelSettings;

    fn agent(rules: Vec<&str>) -> Agent {
        Agent {
            id: "writer".into(),
            system_prompt: "Be concise.".into(),
            model: ModelSettings {
                provider: "openai".into(),
                model: "gpt-4.1-mini".into(),
                max_output_tokens: Some(42),
            },
            rules: rules.into_iter().map(str::to_owned).collect(),
            tool_ids: Vec::new(),
            skill_ids: Vec::new(),
        }
    }

    #[test]
    fn openai_prompt_includes_system_prompt_and_rules() {
        assert_eq!(
            agent_instructions(&agent(vec!["No markdown.", "Use facts."])),
            "Be concise.\n\nRules:\n- No markdown.\n- Use facts."
        );
    }

    #[test]
    fn openai_rejects_unsupported_tools_without_credentials_or_network() {
        let mut agent = agent(Vec::new());
        agent.tool_ids.push("shell".into());
        assert_eq!(
            OpenAiRigRuntime.execute(
                &agent,
                &ExecutionConfig::Local {
                    command: vec!["rig-openai".into()],
                    working_directory: None,
                },
                "input"
            ),
            Err("OpenAI runtime does not support configured tools for agent writer".into())
        );
    }
}
