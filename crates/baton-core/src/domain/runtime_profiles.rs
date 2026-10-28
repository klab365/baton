//! Runtime profiles are project-domain configuration, independent of adapters.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeProfile {
    pub id: String,
    pub name: String,
    pub runtime_kind: RuntimeKind,
    #[serde(default)]
    pub model: Option<super::runtime::RuntimeModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    /// Embedded Rust runtime. The adapter is intentionally introduced separately.
    Rig,
    /// Deterministic offline runtime used by tests.
    Fake,
}

pub fn profile_id(kind: &RuntimeKind, model: Option<&super::runtime::RuntimeModel>) -> String {
    let kind = match kind {
        RuntimeKind::Rig => "rig",
        RuntimeKind::Fake => "fake",
    };
    match model {
        Some(model) => format!(
            "{kind}.{}.{}",
            segment(&model.provider),
            segment(&model.model)
        ),
        None => format!("{kind}.default"),
    }
}

fn segment(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn validate(profiles: &[RuntimeProfile]) -> Result<HashSet<&str>, String> {
    let mut ids = HashSet::new();
    for profile in profiles {
        if profile.id != profile_id(&profile.runtime_kind, profile.model.as_ref())
            || !ids.insert(profile.id.as_str())
            || profile.name.trim().is_empty()
            || profile.model.as_ref().is_some_and(|model| {
                model.provider.trim().is_empty() || model.model.trim().is_empty()
            })
        {
            return Err(format!(
                "Invalid or duplicate runtime profile {:?}; IDs must match runtime.provider.model",
                profile.id
            ));
        }
    }
    Ok(ids)
}

pub fn resolve(
    config: &crate::domain::project::ProjectConfig,
    id: &str,
) -> Result<RuntimeProfile, String> {
    config
        .runtime_profiles
        .iter()
        .find(|profile| profile.id == id)
        .cloned()
        .ok_or_else(|| format!("Runtime profile {id:?} is not configured in .baton/config.json"))
}

pub fn list(config: &crate::domain::project::ProjectConfig) -> Vec<RuntimeProfile> {
    config.runtime_profiles.clone()
}
