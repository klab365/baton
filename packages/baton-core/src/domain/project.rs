use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectConfig {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub agents: Vec<Agent>,
    pub pipelines: Vec<Pipeline>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Pipeline {
    pub id: String,
    pub steps: Vec<PipelineStep>,
}

impl<'de> Deserialize<'de> for Pipeline {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct CompatiblePipeline {
            id: String,
            steps: Vec<PipelineStep>,
            #[serde(default, rename = "name")]
            _legacy_name: Option<serde::de::IgnoredAny>,
        }
        let value = CompatiblePipeline::deserialize(deserializer)?;
        Ok(Self {
            id: value.id,
            steps: value.steps,
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Agent {
    pub id: String,
    #[serde(alias = "systemPrompt")]
    pub instructions: String,
    #[serde(default, rename = "provider")]
    pub runtime_kind: crate::domain::runtime::RuntimeKind,
    #[serde(default)]
    pub model: Option<crate::domain::runtime::RuntimeModel>,
    #[serde(default)]
    pub credential_id: Option<String>,
    pub skill_ids: Vec<String>,
    /// None preserves the legacy default; Some([]) disables all tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
}

impl Agent {
    pub const TOOLS: &'static [&'static str] = &[
        "read",
        "list",
        "search",
        "shell",
        "write",
        "edit",
        "report_outcome",
    ];

    pub fn allows_tool(&self, tool: &str) -> bool {
        self.allowed_tools
            .as_ref()
            .is_none_or(|tools| tools.iter().any(|name| name == tool))
    }
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PipelineStep {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: PipelineStepKind,
    #[serde(default)]
    pub transitions: Vec<Transition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PipelineStepKind {
    Agent {
        agent_id: String,
        #[serde(default)]
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        include_task: Option<bool>,
    },
    Human,
    Shell {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_seconds: Option<u32>,
    },
}

impl<'de> Deserialize<'de> for PipelineStep {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(
            tag = "type",
            rename_all = "snake_case",
            rename_all_fields = "camelCase",
            deny_unknown_fields
        )]
        enum Wire {
            Agent {
                id: String,
                name: String,
                agent_id: String,
                #[serde(default)]
                prompt: String,
                #[serde(default)]
                include_task: Option<bool>,
                #[serde(default)]
                transitions: Vec<Transition>,
            },
            Human {
                id: String,
                name: String,
                #[serde(default)]
                transitions: Vec<Transition>,
            },
            Shell {
                id: String,
                name: String,
                command: String,
                #[serde(default)]
                timeout_seconds: Option<u32>,
                #[serde(default)]
                transitions: Vec<Transition>,
            },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Agent {
                id,
                name,
                agent_id,
                prompt,
                include_task,
                transitions,
            } => Self {
                id,
                name,
                kind: PipelineStepKind::Agent {
                    agent_id,
                    prompt,
                    include_task,
                },
                transitions,
            },
            Wire::Human {
                id,
                name,
                transitions,
            } => Self {
                id,
                name,
                kind: PipelineStepKind::Human,
                transitions,
            },
            Wire::Shell {
                id,
                name,
                command,
                timeout_seconds,
                transitions,
            } => Self {
                id,
                name,
                kind: PipelineStepKind::Shell {
                    command,
                    timeout_seconds,
                },
                transitions,
            },
        })
    }
}

impl PipelineStep {
    pub fn is_human(&self) -> bool {
        matches!(self.kind, PipelineStepKind::Human)
    }
    pub fn agent_id(&self) -> &str {
        match &self.kind {
            PipelineStepKind::Agent { agent_id, .. } => agent_id,
            _ => "",
        }
    }
    pub fn command(&self) -> Option<&str> {
        match &self.kind {
            PipelineStepKind::Shell { command, .. } => Some(command),
            _ => None,
        }
    }
    pub fn timeout_seconds(&self) -> Option<u32> {
        match self.kind {
            PipelineStepKind::Shell {
                timeout_seconds, ..
            } => timeout_seconds,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Retained only to deserialize outcomes recorded by older runs.
    Progress,
    Completed,
    Approved,
    ReviseBuild,
    RevisePlan,
    Blocked,
    Failed,
    Rejected,
    Responded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub outcome: Outcome,
    // Null is an explicit terminal transition.
    pub to: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Skill {
    pub id: String,
    pub description: String,
    /// Original SKILL.md frontmatter, retained to preserve source-specific metadata.
    #[serde(default)]
    pub metadata: String,
    pub instructions: String,
}

impl ProjectConfig {
    pub const CURRENT: Self = Self {
        schema_version: 7,
        project_id: None,
        agents: Vec::new(),
        pipelines: Vec::new(),
    };

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != Self::CURRENT.schema_version {
            return Err(format!(
                "Unsupported schemaVersion {}; expected 7",
                self.schema_version
            ));
        }
        if let Some(id) = &self.project_id
            && (id.len() != 36
                || !id.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_hexdigit()
                    }
                }))
        {
            return Err("projectId must be a UUID".into());
        }
        ids("pipelines", self.pipelines.iter().map(|x| x.id.as_str()))?;
        let agents = ids("agents", self.agents.iter().map(|x| x.id.as_str()))?;
        for agent in &self.agents {
            nonempty("agent.instructions", &agent.instructions)?;
            if agent.model.as_ref().is_some_and(|model| {
                model.provider.trim().is_empty() || model.model.trim().is_empty()
            }) {
                return Err(format!(
                    "Agent {:?} has an invalid provider or model",
                    agent.id
                ));
            }
            if agent.credential_id.as_ref().is_some_and(|id| {
                id.is_empty()
                    || !id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
            }) {
                return Err(format!("Agent {:?} has an invalid credential ID", agent.id));
            }
            ids("agent.skillIds", agent.skill_ids.iter().map(String::as_str))?;
            if let Some(tools) = &agent.allowed_tools {
                ids("agent.allowedTools", tools.iter().map(String::as_str))?;
                for tool in tools {
                    if !Agent::TOOLS.contains(&tool.as_str()) {
                        return Err(format!("Agent {:?} has unknown tool {tool:?}", agent.id));
                    }
                }
            }
        }
        for pipeline in &self.pipelines {
            let path = format!("pipelines[{}]", pipeline.id);
            if pipeline.steps.is_empty() {
                return Err(format!("{path}.steps must contain at least one step"));
            }
            ids(
                &format!("{path}.steps"),
                pipeline.steps.iter().map(|x| x.id.as_str()),
            )?;
            for step in &pipeline.steps {
                let step_path = format!("{path}.steps[{}]", step.id);
                nonempty(&format!("{step_path}.name"), &step.name)?;
                match &step.kind {
                    PipelineStepKind::Shell {
                        command,
                        timeout_seconds,
                    } => {
                        nonempty(&format!("{step_path}.command"), command)?;
                        if timeout_seconds.is_some_and(|timeout| !(1..=86400).contains(&timeout)) {
                            return Err(format!(
                                "{step_path}: timeoutSeconds must be between 1 and 86400"
                            ));
                        }
                    }
                    PipelineStepKind::Human => {}
                    PipelineStepKind::Agent { agent_id, .. } => {
                        reference(&format!("{step_path}.agentId"), agent_id, &agents)?;
                        if !step.transitions.is_empty()
                            && self
                                .agents
                                .iter()
                                .find(|agent| agent.id == *agent_id)
                                .is_some_and(|agent| !agent.allows_tool("report_outcome"))
                        {
                            return Err(format!(
                                "{step_path}: agent {agent_id:?} must allow report_outcome for steps with transitions"
                            ));
                        }
                    }
                }
                let mut outcomes = Vec::new();
                for transition in &step.transitions {
                    if transition.outcome == Outcome::Progress {
                        return Err(format!(
                            "{step_path}: progress is only supported in historical run data"
                        ));
                    }
                    if step.command().is_some()
                        && !matches!(transition.outcome, Outcome::Completed | Outcome::Failed)
                    {
                        return Err(format!(
                            "{step_path}: shell transitions require completed or failed"
                        ));
                    }
                    if step.is_human()
                        && !matches!(
                            transition.outcome,
                            Outcome::Approved | Outcome::Rejected | Outcome::Responded
                        )
                    {
                        return Err(format!(
                            "{step_path}: human transitions require approved, rejected, or responded"
                        ));
                    }
                    if outcomes.contains(&transition.outcome) {
                        return Err(format!("{step_path}: ambiguous outcome transition"));
                    }
                    outcomes.push(transition.outcome.clone());
                    if let Some(target) = &transition.to
                        && !pipeline.steps.iter().any(|s| &s.id == target)
                    {
                        return Err(format!("{step_path}: unknown transition target {target}"));
                    }
                }
            }
        }
        Ok(())
    }
}

fn nonempty(path: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{path} must not be empty"))
    } else {
        Ok(())
    }
}

fn ids<'a>(path: &str, values: impl Iterator<Item = &'a str>) -> Result<HashSet<&'a str>, String> {
    let mut seen = HashSet::new();
    for id in values {
        if id.is_empty()
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        {
            return Err(format!(
                "{path}: invalid ID {id:?}; use ASCII letters, digits, '-', '_' or '.'"
            ));
        }
        if !seen.insert(id) {
            return Err(format!("{path}: duplicate ID {id:?}"));
        }
    }
    Ok(seen)
}

fn reference(path: &str, id: &str, targets: &HashSet<&str>) -> Result<(), String> {
    if targets.contains(id) {
        Ok(())
    } else {
        Err(format!("{path}: unknown reference {id:?}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSnapshot {
    pub config: ProjectConfig,
    /// Exact bytes read, used as an optimistic concurrency token (not serialized).
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigState {
    Missing,
    Ready(ConfigSnapshot),
    Invalid(String),
}

fn parse_snapshot(mut value: serde_json::Value) -> Result<ProjectConfig, String> {
    if let Some(pipelines) = value
        .get_mut("pipelines")
        .and_then(serde_json::Value::as_array_mut)
    {
        for pipeline in pipelines {
            if let Some(steps) = pipeline
                .get_mut("steps")
                .and_then(serde_json::Value::as_array_mut)
            {
                for step in steps {
                    let Some(object) = step.as_object_mut() else {
                        continue;
                    };
                    if object.contains_key("type") {
                        continue;
                    }
                    let human = match object.remove("human") {
                        None => false,
                        Some(serde_json::Value::Bool(value)) => value,
                        Some(_) => return Err("Invalid legacy step human flag".into()),
                    };
                    let agent = match object.remove("agentId") {
                        None => String::new(),
                        Some(serde_json::Value::String(value)) => value,
                        Some(_) => return Err("Invalid legacy step agentId".into()),
                    };
                    let command = match object.remove("command") {
                        None | Some(serde_json::Value::Null) => None,
                        Some(serde_json::Value::String(value)) => Some(value),
                        Some(_) => return Err("Invalid legacy step command".into()),
                    };
                    let timeout = object.remove("timeoutSeconds");
                    let kind = if let Some(command) = command {
                        if human || !agent.is_empty() {
                            return Err("Invalid legacy shell step".into());
                        }
                        object.insert("command".into(), command.into());
                        if let Some(timeout) = timeout.filter(|value| !value.is_null()) {
                            object.insert("timeoutSeconds".into(), timeout);
                        }
                        "shell"
                    } else if human {
                        if !agent.is_empty() || timeout.is_some_and(|value| !value.is_null()) {
                            return Err("Invalid legacy human step".into());
                        }
                        "human"
                    } else {
                        if timeout.is_some_and(|value| !value.is_null()) {
                            return Err("Invalid legacy agent step".into());
                        }
                        object.insert("agentId".into(), agent.into());
                        "agent"
                    };
                    object.insert("type".into(), kind.into());
                }
            }
        }
    }
    parse_config(&value.to_string())
}

/// Read historical snapshots without accepting legacy step syntax in new configuration.
pub fn deserialize_snapshot<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ProjectConfig, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    parse_snapshot(value).map_err(serde::de::Error::custom)
}

pub fn deserialize_optional_snapshot<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ProjectConfig>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    value
        .map(|value| parse_snapshot(value).map_err(serde::de::Error::custom))
        .transpose()
}

pub fn parse_config(contents: &str) -> Result<ProjectConfig, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct LegacyConfig {
        schema_version: u32,
    }

    // Only the exact v1 marker is migrated. Never discard unknown legacy data.
    if let Ok(legacy) = serde_json::from_str::<LegacyConfig>(contents)
        && legacy.schema_version == 1
    {
        return Ok(ProjectConfig::CURRENT);
    }
    let mut value: serde_json::Value = serde_json::from_str(contents)
        .map_err(|error| format!("Invalid project configuration: {error}"))?;
    if value
        .get("skills")
        .and_then(serde_json::Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        value.as_object_mut().unwrap().remove("skills");
    }
    // Early external-work snapshots used a per-step execution mode. Execution is
    // now selected by the host, but those snapshots must remain resumable.
    if let Some(pipelines) = value
        .get_mut("pipelines")
        .and_then(serde_json::Value::as_array_mut)
    {
        for pipeline in pipelines {
            if let Some(steps) = pipeline
                .get_mut("steps")
                .and_then(serde_json::Value::as_array_mut)
            {
                for step in steps {
                    if let Some(step) = step.as_object_mut()
                        && let Some(execution) = step.remove("execution")
                        && !matches!(execution.as_str(), Some("inline" | "subagent"))
                    {
                        return Err("Unknown legacy step execution mode".into());
                    }
                }
            }
        }
    }
    // Rules were folded into the system prompt. Accept and discard the removed
    // field so existing project configurations remain loadable.
    if let Some(agents) = value
        .get_mut("agents")
        .and_then(serde_json::Value::as_array_mut)
    {
        for agent in agents {
            if let Some(agent) = agent.as_object_mut() {
                agent.remove("rules");
            }
        }
    }
    if matches!(value["schemaVersion"].as_u64(), Some(2..=6)) {
        return Err(
            "Legacy runtime profiles are no longer supported; update the project configuration"
                .into(),
        );
    }
    let config: ProjectConfig = serde_json::from_value(value)
        .map_err(|error| format!("Invalid project configuration: {error}"))?;
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod project_identity_tests {
    use super::parse_config;

    #[test]
    fn legacy_run_snapshots_are_read_and_written_with_explicit_types() {
        use crate::domain::pipeline::PipelineRun;
        let snapshot = serde_json::json!({
            "schemaVersion": 7,
            "agents": [{"id":"worker", "systemPrompt":"work", "skillIds":[]}],
            "pipelines": [{"id":"p", "steps":[
                {"id":"a", "name":"Agent", "agentId":"worker", "human":false},
                {"id":"h", "name":"Human", "agentId":"", "human":true},
                {"id":"s", "name":"Shell", "agentId":"", "human":false, "command":"true", "timeoutSeconds":2}
            ]}]
        });
        let run = serde_json::json!({"id":"run", "pipelineId":"p", "task":"task", "startedAt":"now", "endedAt":null, "durationMs":null, "status":"running", "steps":[], "configSnapshot":snapshot});
        let parsed: PipelineRun = serde_json::from_value(run.clone()).unwrap();
        let saved = serde_json::to_value(parsed).unwrap();
        let steps = saved["configSnapshot"]["pipelines"][0]["steps"]
            .as_array()
            .unwrap();
        assert_eq!(
            steps
                .iter()
                .map(|step| step["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["agent", "human", "shell"]
        );
        assert!(steps.iter().all(|step| step.get("human").is_none()));
        let mut invalid = run;
        invalid["configSnapshot"]["pipelines"][0]["steps"][1]["agentId"] = "worker".into();
        assert!(serde_json::from_value::<PipelineRun>(invalid).is_err());
    }

    #[test]
    fn step_type_is_explicit_and_rejects_incompatible_fields() {
        use super::{PipelineStep, PipelineStepKind};
        for (json, kind) in [
            (
                r#"{"id":"a","name":"Agent","type":"agent","agentId":"worker"}"#,
                PipelineStepKind::Agent {
                    agent_id: "worker".into(),
                    prompt: String::new(),
                    include_task: None,
                },
            ),
            (
                r#"{"id":"h","name":"Human","type":"human"}"#,
                PipelineStepKind::Human,
            ),
            (
                r#"{"id":"s","name":"Shell","type":"shell","command":"true"}"#,
                PipelineStepKind::Shell {
                    command: "true".into(),
                    timeout_seconds: None,
                },
            ),
        ] {
            let step: PipelineStep = serde_json::from_str(json).unwrap();
            assert_eq!(step.kind, kind);
            assert_eq!(
                serde_json::from_str::<PipelineStep>(&serde_json::to_string(&step).unwrap())
                    .unwrap(),
                step
            );
            let yaml = serde_yaml_ng::to_string(&step).unwrap();
            assert_eq!(
                serde_yaml_ng::from_str::<PipelineStep>(&yaml).unwrap(),
                step
            );
        }
        for invalid in [
            r#"{"id":"a","name":"Agent","agentId":"worker"}"#,
            r#"{"id":"a","name":"Agent","type":"agent","agentId":"worker","human":true}"#,
            r#"{"id":"h","name":"Human","type":"human","agentId":"worker"}"#,
            r#"{"id":"s","name":"Shell","type":"shell","command":"true","agentId":"worker"}"#,
        ] {
            assert!(
                serde_json::from_str::<PipelineStep>(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn shell_timeout_must_be_in_range_and_only_on_shell_steps() {
        let mut config: super::ProjectConfig =
            serde_json::from_str(include_str!("../../tests/fixture_config.json")).unwrap();
        let step = &mut config.pipelines[0].steps[0];
        step.kind = super::PipelineStepKind::Shell {
            command: "true".into(),
            timeout_seconds: Some(1),
        };
        assert!(config.validate().is_ok());
        for timeout in [0, 86401] {
            config.pipelines[0].steps[0].kind = super::PipelineStepKind::Shell {
                command: "true".into(),
                timeout_seconds: Some(timeout),
            };
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn tool_configuration_preserves_defaults_and_validates_transitions() {
        let agent = r#"{"id":"planner","systemPrompt":"plan","skillIds":[]}"#;
        let parsed: super::Agent = serde_json::from_str(agent).unwrap();
        assert!(parsed.allowed_tools.is_none());
        assert!(parsed.allows_tool("write"));
        assert!(
            !serde_json::to_string(&parsed)
                .unwrap()
                .contains("allowedTools")
        );

        let explicit = agent.replace("\"skillIds\":[]", "\"skillIds\":[],\"allowedTools\":[]");
        let parsed: super::Agent = serde_json::from_str(&explicit).unwrap();
        assert!(!parsed.allows_tool("write"));
        assert_eq!(
            serde_json::from_str::<super::Agent>(&serde_json::to_string(&parsed).unwrap()).unwrap(),
            parsed
        );
        let pipeline = r#"[{"id":"p","steps":[{"id":"s","name":"Step","type":"agent","agentId":"planner","transitions":[{"outcome":"completed","to":null}]}]}]"#;
        let config =
            format!(r#"{{"schemaVersion":7,"agents":[{explicit}],"pipelines":{pipeline}}}"#);
        assert!(
            parse_config(&config)
                .unwrap_err()
                .contains("report_outcome")
        );
        assert!(
            parse_config(&config.replace(
                "\"allowedTools\":[]",
                "\"allowedTools\":[\"report_outcome\"]"
            ))
            .is_ok()
        );
        assert!(parse_config(&config.replace(pipeline, "[]")).is_ok());
        assert!(
            parse_config(&config.replace("\"allowedTools\":[]", "\"allowedTools\":[\"unknown\"]"))
                .is_err()
        );
    }

    #[test]
    fn accepts_legacy_configs_and_preserves_valid_project_ids() {
        let legacy = r#"{"schemaVersion":7,"agents":[],"pipelines":[]}"#;
        assert!(parse_config(legacy).unwrap().project_id.is_none());
        let with_id = r#"{"schemaVersion":7,"projectId":"018f0000-0000-7000-8000-000000000001","agents":[],"pipelines":[]}"#;
        let parsed = parse_config(with_id).unwrap();
        assert_eq!(
            parse_config(&serde_json::to_string(&parsed).unwrap()).unwrap(),
            parsed
        );
        let invalid = with_id.replace("018f0000-0000-7000-8000-000000000001", "../unsafe");
        assert!(parse_config(&invalid).is_err());
    }
}
