use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectConfig {
    pub schema_version: u32,
}

impl ProjectConfig {
    pub const CURRENT: Self = Self { schema_version: 1 };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigState {
    Missing,
    Ready(ProjectConfig),
    Invalid(String),
}

pub fn parse_config(contents: &str) -> Result<ProjectConfig, String> {
    let config: ProjectConfig = serde_json::from_str(contents)
        .map_err(|error| format!("Invalid project configuration: {error}"))?;
    if config.schema_version != ProjectConfig::CURRENT.schema_version {
        return Err("This project configuration version is not supported.".into());
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_current_config() {
        assert_eq!(
            parse_config(r#"{"schemaVersion":1}"#),
            Ok(ProjectConfig::CURRENT)
        );
    }

    #[test]
    fn rejects_unknown_versions() {
        assert!(parse_config(r#"{"schemaVersion":2}"#).is_err());
    }
}
