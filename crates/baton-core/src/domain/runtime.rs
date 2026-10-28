use serde::{Deserialize, Serialize};

/// Provider/model selection stored on a reusable runtime profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeModel {
    pub provider: String,
    pub model: String,
}
