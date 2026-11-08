use serde::{Deserialize, Serialize};

/// Provider/model selection configured directly on an agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    #[default]
    Rig,
    Fake,
}

/// Provider/model selection configured directly on an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeModel {
    pub provider: String,
    pub model: String,
}
