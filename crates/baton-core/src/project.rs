use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectConfig {
    pub schema_version: u32,
    pub agents: Vec<Agent>,
    pub pipelines: Vec<Pipeline>,
    pub tools: Vec<Tool>,
    pub skills: Vec<Skill>,
}

macro_rules! dto {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $name { $(pub $field: $ty),* }
    };
}

dto!(ModelSettings { provider: String, model: String, max_output_tokens: Option<u32> });
dto!(Pipeline { id: String, name: String, steps: Vec<PipelineStep> });
dto!(Agent {
    id: String,
    system_prompt: String,
    model: ModelSettings,
    rules: Vec<String>,
    tool_ids: Vec<String>,
    skill_ids: Vec<String>
});
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PipelineStep {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub human: bool,
    #[serde(default)]
    pub transitions: Vec<Transition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Approved,
    ReviseBuild,
    RevisePlan,
    Blocked,
    Failed,
    Rejected,
    Responded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transition {
    pub outcome: Outcome,
    // Null is an explicit terminal transition.
    pub to: Option<String>,
    // Every edge is bounded, so every possible cycle is bounded.
    pub max_traversals: u32,
}
dto!(Tool {
    id: String,
    name: String
});
dto!(Skill {
    id: String,
    name: String,
    instructions: String
});

impl ProjectConfig {
    pub const CURRENT: Self = Self {
        schema_version: 3,
        agents: Vec::new(),
        pipelines: Vec::new(),
        tools: Vec::new(),
        skills: Vec::new(),
    };

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != Self::CURRENT.schema_version {
            return Err(format!(
                "Unsupported schemaVersion {}; expected 3",
                self.schema_version
            ));
        }
        ids("pipelines", self.pipelines.iter().map(|x| x.id.as_str()))?;
        let tools = ids("tools", self.tools.iter().map(|x| x.id.as_str()))?;
        let skills = ids("skills", self.skills.iter().map(|x| x.id.as_str()))?;
        let agents = ids("agents", self.agents.iter().map(|x| x.id.as_str()))?;
        for agent in &self.agents {
            nonempty("agent.systemPrompt", &agent.system_prompt)?;
            nonempty("agent.model.provider", &agent.model.provider)?;
            nonempty("agent.model.model", &agent.model.model)?;
            if agent.model.max_output_tokens == Some(0) {
                return Err("agent.model.maxOutputTokens must be positive".into());
            }
            for rule in &agent.rules {
                nonempty("agent.rules", rule)?;
            }
            references("agent.toolIds", &agent.tool_ids, &tools)?;
            if agent
                .tool_ids
                .iter()
                .any(|id| !crate::tools::IDS.contains(&id.as_str()))
            {
                return Err(
                    "Agent references an unsupported or incompatible built-in tool version".into(),
                );
            }
            references("agent.skillIds", &agent.skill_ids, &skills)?;
        }
        for pipeline in &self.pipelines {
            let path = format!("pipelines[{}]", pipeline.id);
            nonempty(&format!("{path}.name"), &pipeline.name)?;
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
                if step.human {
                    if !step.agent_id.is_empty() {
                        return Err(format!("{step_path}: human steps cannot have an agent"));
                    }
                } else {
                    reference(&format!("{step_path}.agentId"), &step.agent_id, &agents)?;
                }
                let mut outcomes = Vec::new();
                for transition in &step.transitions {
                    if step.human
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
                    if transition.max_traversals == 0 || transition.max_traversals > 1000 {
                        return Err(format!("{step_path}: maxTraversals must be 1..1000"));
                    }
                    if let Some(target) = &transition.to
                        && !pipeline.steps.iter().any(|s| &s.id == target)
                    {
                        return Err(format!("{step_path}: unknown transition target {target}"));
                    }
                }
            }
        }
        for tool in &self.tools {
            if !crate::tools::IDS.contains(&tool.id.as_str()) {
                return Err("Project enables an unsupported built-in tool version".into());
            }
            nonempty(&format!("tools[{}].name", tool.id), &tool.name)?;
        }
        for skill in &self.skills {
            nonempty(&format!("skills[{}].name", skill.id), &skill.name)?;
            nonempty(
                &format!("skills[{}].instructions", skill.id),
                &skill.instructions,
            )?;
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

fn references(path: &str, values: &[String], targets: &HashSet<&str>) -> Result<(), String> {
    ids(path, values.iter().map(String::as_str))?;
    for value in values {
        reference(path, value, targets)?;
    }
    Ok(())
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
    let config: ProjectConfig = serde_json::from_str(contents)
        .map_err(|error| format!("Invalid project configuration: {error}"))?;
    config.validate()?;
    Ok(config)
}
