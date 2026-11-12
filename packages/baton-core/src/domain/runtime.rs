use serde::{Deserialize, Serialize};

/// Provider/model selection configured directly on an agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    #[default]
    Rig,
    Fake,
    Pi,
}

/// Provider/model selection configured directly on an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeModel {
    pub provider: String,
    pub model: String,
}

impl Serialize for RuntimeModel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.provider.is_empty() || self.provider.contains('/') || self.model.is_empty() {
            return Err(serde::ser::Error::custom(
                "model must be in provider/model format",
            ));
        }
        serializer.serialize_str(&format!("{}/{}", self.provider, self.model))
    }
}

impl<'de> Deserialize<'de> for RuntimeModel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let selection = String::deserialize(deserializer)?;
        let (provider, model) = selection
            .split_once('/')
            .ok_or_else(|| serde::de::Error::custom("model must be in provider/model format"))?;
        if provider.is_empty() || model.is_empty() {
            return Err(serde::de::Error::custom(
                "model must be in provider/model format",
            ));
        }
        Ok(Self {
            provider: provider.into(),
            model: model.into(),
        })
    }
}
