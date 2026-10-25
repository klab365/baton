//! Built-in tool catalog and project-level activation use cases.
use crate::{
    project::{ConfigSnapshot, ConfigState, Tool},
    project_config::ProjectConfigRepository,
};
use serde::Serialize;

pub const IDS: &[&str] = &[
    "builtin.read.v1",
    "builtin.write.v1",
    "builtin.list.v1",
    "builtin.search.v1",
    "builtin.patch.v1",
    "builtin.delete.v1",
];
const LIMIT: usize = 8192;

/// Host-owned definition, independent of prompts and project enablement records.
#[derive(Debug, Serialize)]
pub struct Definition {
    pub id: String,
    pub version: u32,
    pub permission: &'static str,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub max_output_bytes: usize,
    pub session_timeout_seconds: u64,
}
pub fn definitions() -> Vec<Definition> {
    IDS.iter().filter_map(|id| definition(id)).collect()
}
pub fn definition(id: &str) -> Option<Definition> {
    if !IDS.contains(&id) {
        return None;
    }
    let write = matches!(
        id,
        "builtin.write.v1" | "builtin.patch.v1" | "builtin.delete.v1"
    );
    let mut properties = serde_json::Map::new();
    properties.insert(
        "path".into(),
        serde_json::json!({"type":"string", "maxLength":256}),
    );
    properties.insert(
        "content".into(),
        if matches!(id, "builtin.write.v1" | "builtin.patch.v1") {
            serde_json::json!({"type":"string", "maxLength":8192})
        } else if id == "builtin.delete.v1" {
            serde_json::json!({"type":"string", "const":"DELETE"})
        } else {
            serde_json::json!({"type":"null"})
        },
    );
    properties.insert(
        "query".into(),
        serde_json::json!({"type":"string", "maxLength":256}),
    );
    properties.insert(
        "expected".into(),
        serde_json::json!({"type":"string", "maxLength":8192}),
    );
    let required = match id {
        "builtin.write.v1" => vec!["path", "content"],
        "builtin.patch.v1" => vec!["path", "expected", "content"],
        "builtin.delete.v1" => vec!["path", "content"],
        "builtin.search.v1" => vec!["path", "query"],
        _ => vec!["path"],
    };
    Some(Definition {
        id: id.into(),
        version: 1,
        permission: if write {
            "worktree:write-files"
        } else {
            "worktree:read-files"
        },
        input_schema: serde_json::json!({"type":"object","additionalProperties":false,"required":required,"properties":properties}),
        output_schema: serde_json::json!({"type":"object","additionalProperties":false,"required":["content"],"properties":{"content":{"type":"string","maxLength":8192}}}),
        max_output_bytes: LIMIT,
        session_timeout_seconds: 60,
    })
}
#[derive(Debug)]
pub enum ToolError {
    NotFound,
    Conflict(String),
    Validation(String),
    Storage(String),
}
pub async fn list(repository: &dyn ProjectConfigRepository) -> Result<ConfigSnapshot, ToolError> {
    match repository.load().await.map_err(ToolError::Storage)? {
        ConfigState::Ready(snapshot) => Ok(snapshot),
        ConfigState::Missing => Err(ToolError::Conflict("Initialize the project first".into())),
        ConfigState::Invalid(message) => Err(ToolError::Conflict(message)),
    }
}
pub async fn enable(
    repository: &dyn ProjectConfigRepository,
    revision: &str,
    id: &str,
) -> Result<(), ToolError> {
    let mut snapshot = list(repository).await?;
    if snapshot.revision != revision {
        return Err(ToolError::Conflict(
            "Configuration changed; reload before saving".into(),
        ));
    }
    let def =
        definition(id).ok_or_else(|| ToolError::Validation("Unknown built-in tool".into()))?;
    if snapshot.config.tools.iter().any(|tool| tool.id == id) {
        return Err(ToolError::Conflict(
            "Built-in tool is already enabled".into(),
        ));
    }
    snapshot.config.tools.push(Tool {
        id: id.into(),
        name: def.id,
    });
    snapshot.config.validate().map_err(ToolError::Validation)?;
    repository
        .update(revision, &snapshot.config)
        .await
        .map_err(ToolError::Conflict)
}
pub async fn disable(
    repository: &dyn ProjectConfigRepository,
    revision: &str,
    id: &str,
) -> Result<(), ToolError> {
    let mut snapshot = list(repository).await?;
    if snapshot.revision != revision {
        return Err(ToolError::Conflict(
            "Configuration changed; reload before saving".into(),
        ));
    }
    let index = snapshot
        .config
        .tools
        .iter()
        .position(|tool| tool.id == id)
        .ok_or(ToolError::NotFound)?;
    if let Some(agent) = snapshot
        .config
        .agents
        .iter()
        .find(|agent| agent.tool_ids.iter().any(|tool| tool == id))
    {
        return Err(ToolError::Conflict(format!(
            "Tool is assigned to agent {:?}; remove it from that agent first",
            agent.id
        )));
    }
    snapshot.config.tools.remove(index);
    repository
        .update(revision, &snapshot.config)
        .await
        .map_err(ToolError::Conflict)
}
