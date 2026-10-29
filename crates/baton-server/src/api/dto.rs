use baton_core::domain::{
    project as domain,
    runtime::{RuntimeKind as CoreRuntimeKind, RuntimeModel as CoreRuntimeModel},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectConfig {
    pub schema_version: u32,
    pub agents: Vec<Agent>,
    pub pipelines: Vec<Pipeline>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Agent {
    pub id: String,
    pub system_prompt: String,
    pub runtime_kind: RuntimeKind,
    pub model: Option<RuntimeModel>,
    pub credential_id: Option<String>,
    pub rules: Vec<String>,
    pub skill_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pipeline {
    pub id: String,
    pub name: String,
    pub steps: Vec<PipelineStep>,
}

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
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub outcome: Outcome,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Skill {
    pub id: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    Rig,
    Fake,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeModel {
    pub provider: String,
    pub model: String,
}

impl From<domain::ProjectConfig> for ProjectConfig {
    fn from(value: domain::ProjectConfig) -> Self {
        Self {
            schema_version: value.schema_version,
            agents: value.agents.into_iter().map(Into::into).collect(),
            pipelines: value.pipelines.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ProjectConfig> for domain::ProjectConfig {
    fn from(value: ProjectConfig) -> Self {
        Self {
            schema_version: value.schema_version,
            agents: value.agents.into_iter().map(Into::into).collect(),
            pipelines: value.pipelines.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<domain::Agent> for Agent {
    fn from(value: domain::Agent) -> Self {
        Self {
            id: value.id,
            system_prompt: value.system_prompt,
            runtime_kind: value.runtime_kind.into(),
            model: value.model.map(Into::into),
            credential_id: value.credential_id,
            rules: value.rules,
            skill_ids: value.skill_ids,
        }
    }
}

impl From<Agent> for domain::Agent {
    fn from(value: Agent) -> Self {
        Self {
            id: value.id,
            system_prompt: value.system_prompt,
            runtime_kind: value.runtime_kind.into(),
            model: value.model.map(Into::into),
            credential_id: value.credential_id,
            rules: value.rules,
            skill_ids: value.skill_ids,
        }
    }
}

impl From<domain::Pipeline> for Pipeline {
    fn from(value: domain::Pipeline) -> Self {
        Self {
            id: value.id,
            name: value.name,
            steps: value.steps.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<Pipeline> for domain::Pipeline {
    fn from(value: Pipeline) -> Self {
        Self {
            id: value.id,
            name: value.name,
            steps: value.steps.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<domain::PipelineStep> for PipelineStep {
    fn from(value: domain::PipelineStep) -> Self {
        Self {
            id: value.id,
            name: value.name,
            agent_id: value.agent_id,
            human: value.human,
            transitions: value.transitions.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<PipelineStep> for domain::PipelineStep {
    fn from(value: PipelineStep) -> Self {
        Self {
            id: value.id,
            name: value.name,
            agent_id: value.agent_id,
            human: value.human,
            transitions: value.transitions.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<domain::Outcome> for Outcome {
    fn from(value: domain::Outcome) -> Self {
        match value {
            domain::Outcome::Completed => Self::Completed,
            domain::Outcome::Approved => Self::Approved,
            domain::Outcome::ReviseBuild => Self::ReviseBuild,
            domain::Outcome::RevisePlan => Self::RevisePlan,
            domain::Outcome::Blocked => Self::Blocked,
            domain::Outcome::Failed => Self::Failed,
            domain::Outcome::Rejected => Self::Rejected,
            domain::Outcome::Responded => Self::Responded,
        }
    }
}

impl From<Outcome> for domain::Outcome {
    fn from(value: Outcome) -> Self {
        match value {
            Outcome::Completed => Self::Completed,
            Outcome::Approved => Self::Approved,
            Outcome::ReviseBuild => Self::ReviseBuild,
            Outcome::RevisePlan => Self::RevisePlan,
            Outcome::Blocked => Self::Blocked,
            Outcome::Failed => Self::Failed,
            Outcome::Rejected => Self::Rejected,
            Outcome::Responded => Self::Responded,
        }
    }
}

impl From<domain::Transition> for Transition {
    fn from(value: domain::Transition) -> Self {
        Self {
            outcome: value.outcome.into(),
            to: value.to,
        }
    }
}

impl From<Transition> for domain::Transition {
    fn from(value: Transition) -> Self {
        Self {
            outcome: value.outcome.into(),
            to: value.to,
        }
    }
}

impl From<domain::Skill> for Skill {
    fn from(value: domain::Skill) -> Self {
        Self {
            id: value.id,
            description: value.description,
            instructions: value.instructions,
        }
    }
}

impl From<Skill> for domain::Skill {
    fn from(value: Skill) -> Self {
        Self {
            id: value.id,
            description: value.description,
            instructions: value.instructions,
        }
    }
}

impl From<CoreRuntimeKind> for RuntimeKind {
    fn from(value: CoreRuntimeKind) -> Self {
        match value {
            CoreRuntimeKind::Rig => Self::Rig,
            CoreRuntimeKind::Fake => Self::Fake,
        }
    }
}

impl From<RuntimeKind> for CoreRuntimeKind {
    fn from(value: RuntimeKind) -> Self {
        match value {
            RuntimeKind::Rig => Self::Rig,
            RuntimeKind::Fake => Self::Fake,
        }
    }
}

impl From<CoreRuntimeModel> for RuntimeModel {
    fn from(value: CoreRuntimeModel) -> Self {
        Self {
            provider: value.provider,
            model: value.model,
        }
    }
}

impl From<RuntimeModel> for CoreRuntimeModel {
    fn from(value: RuntimeModel) -> Self {
        Self {
            provider: value.provider,
            model: value.model,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_config_dto_round_trips_without_changing_the_json_contract() {
        let config: domain::ProjectConfig = serde_json::from_str(include_str!(
            "../../../baton-core/tests/fixture_config.json"
        ))
        .unwrap();
        let expected = serde_json::to_value(&config).unwrap();
        let dto: ProjectConfig = config.clone().into();
        assert_eq!(serde_json::to_value(&dto).unwrap(), expected);
        let restored: domain::ProjectConfig = dto.into();
        assert_eq!(restored, config);
    }
}
