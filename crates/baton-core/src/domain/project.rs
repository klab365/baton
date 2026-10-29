use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectConfig {
    pub schema_version: u32,
    pub agents: Vec<Agent>,
    pub pipelines: Vec<Pipeline>,
}

macro_rules! dto {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $name { $(pub $field: $ty),* }
    };
}

dto!(Pipeline { id: String, name: String, steps: Vec<PipelineStep> });
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Agent {
    pub id: String,
    pub system_prompt: String,
    #[serde(default)]
    pub runtime_kind: crate::domain::runtime::RuntimeKind,
    #[serde(default)]
    pub model: Option<crate::domain::runtime::RuntimeModel>,
    #[serde(default)]
    pub credential_id: Option<String>,
    pub skill_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
        ids("pipelines", self.pipelines.iter().map(|x| x.id.as_str()))?;
        let agents = ids("agents", self.agents.iter().map(|x| x.id.as_str()))?;
        for agent in &self.agents {
            nonempty("agent.systemPrompt", &agent.system_prompt)?;
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

/// Snapshot readers share the same explicit breaking migration as project reads.
pub fn deserialize_snapshot<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ProjectConfig, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    parse_config(&value.to_string()).map_err(serde::de::Error::custom)
}

pub fn deserialize_optional_snapshot<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ProjectConfig>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    value
        .map(|value| parse_config(&value.to_string()).map_err(serde::de::Error::custom))
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
    let legacy_version = value["schemaVersion"].as_u64();
    if legacy_version == Some(5) {
        return Err("Pi runtime profiles are no longer supported; replace them with explicit Rig profiles before upgrading to schemaVersion 6".into());
    }
    if matches!(legacy_version, Some(2..=4)) {
        if let Some(agents) = value["agents"].as_array_mut() {
            for agent in agents {
                let object = agent
                    .as_object_mut()
                    .ok_or("Invalid legacy agent; migrate to schemaVersion 4")?;
                if let Some(model) = object.remove("model") {
                    let model = model.as_object().ok_or(
                        "Invalid legacy model; remove model and create a runtime profile in Settings",
                    )?;
                    if model.keys().any(|key| {
                        !["provider", "model", "maxOutputTokens"].contains(&key.as_str())
                    }) {
                        return Err("Unsupported legacy model settings; remove direct runtime/provider settings and configure a profile in Settings".into());
                    }
                }
                object.remove("toolIds");
                if object.contains_key("runtimeProfileId") && legacy_version != Some(4) {
                    return Err("Legacy configuration already contains runtimeProfileId; explicitly migrate to schemaVersion 4".into());
                }
                if legacy_version != Some(4) {
                    object.insert("runtimeProfileId".into(), "rig.default".into());
                }
            }
        }
        value
            .as_object_mut()
            .ok_or("Invalid project configuration")?
            .remove("tools");
        if legacy_version != Some(4) {
            value["runtimeProfiles"] = serde_json::json!([{"id":"rig.default", "name":"Rig default", "runtimeKind":"rig", "model":null}]);
        }
        value["schemaVersion"] = 6.into();
    }
    if value["schemaVersion"].as_u64() == Some(6) {
        let profiles = value["runtimeProfiles"]
            .as_array()
            .ok_or("SchemaVersion 6 configuration has no runtimeProfiles to migrate")?
            .clone();
        let mut migrated_agents = Vec::new();
        if let Some(agents) = value["agents"].as_array() {
            for agent in agents {
                let mut agent = agent.clone();
                let object = agent
                    .as_object_mut()
                    .ok_or("Invalid agent while migrating runtime profiles")?;
                let profile_id = object
                    .remove("runtimeProfileId")
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or("Agent is missing runtimeProfileId while migrating schemaVersion 6")?;
                let profile = profiles
                    .iter()
                    .find(|profile| profile["id"].as_str() == Some(profile_id.as_str()))
                    .ok_or_else(|| {
                        format!("Agent references missing runtime profile {profile_id:?}")
                    })?;
                object.insert(
                    "runtimeKind".into(),
                    profile
                        .get("runtimeKind")
                        .cloned()
                        .unwrap_or(serde_json::Value::String("rig".into())),
                );
                object.insert(
                    "model".into(),
                    profile
                        .get("model")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                );
                if let Some(credential_id) = profile.get("credentialId") {
                    object.insert("credentialId".into(), credential_id.clone());
                }
                migrated_agents.push(agent);
            }
        }
        value["agents"] = serde_json::Value::Array(migrated_agents);
        value
            .as_object_mut()
            .ok_or("Invalid project configuration")?
            .remove("runtimeProfiles");
        value["schemaVersion"] = 7.into();
    }
    let config: ProjectConfig = serde_json::from_value(value)
        .map_err(|error| format!("Invalid project configuration: {error}"))?;
    config.validate()?;
    Ok(config)
}
