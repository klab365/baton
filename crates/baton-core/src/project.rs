use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
dto!(PipelineStep {
    id: String,
    name: String,
    agent_id: String,
    execution: ExecutionConfig
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ExecutionConfig {
    Local {
        command: Vec<String>,
        working_directory: Option<String>,
    },
    Docker {
        image: String,
        command: Vec<String>,
        working_directory: Option<String>,
    },
    Remote {
        host: String,
        command: Vec<String>,
        working_directory: Option<String>,
    },
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
                reference(&format!("{step_path}.agentId"), &step.agent_id, &agents)?;
                validate_execution(&format!("{step_path}.execution"), &step.execution)?;
            }
        }
        for tool in &self.tools {
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

fn validate_execution(path: &str, execution: &ExecutionConfig) -> Result<(), String> {
    match execution {
        ExecutionConfig::Local {
            command,
            working_directory,
        } => {
            command_values(&format!("{path}.command"), command)?;
            optional_nonempty(&format!("{path}.workingDirectory"), working_directory)
        }
        ExecutionConfig::Docker {
            image,
            command,
            working_directory,
        } => {
            nonempty(&format!("{path}.image"), image)?;
            command_values(&format!("{path}.command"), command)?;
            optional_nonempty(&format!("{path}.workingDirectory"), working_directory)
        }
        ExecutionConfig::Remote {
            host,
            command,
            working_directory,
        } => {
            nonempty(&format!("{path}.host"), host)?;
            command_values(&format!("{path}.command"), command)?;
            optional_nonempty(&format!("{path}.workingDirectory"), working_directory)
        }
    }
}

fn command_values(path: &str, command: &[String]) -> Result<(), String> {
    if command.is_empty() {
        return Err(format!("{path} must contain at least one command argument"));
    }
    for (index, value) in command.iter().enumerate() {
        nonempty(&format!("{path}[{index}]"), value)?;
    }
    Ok(())
}

fn optional_nonempty(path: &str, value: &Option<String>) -> Result<(), String> {
    if let Some(value) = value {
        nonempty(path, value)?;
    }
    Ok(())
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
