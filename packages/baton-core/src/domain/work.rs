//! Persisted contract for workers running in an externally owned workspace.
use crate::domain::project::Outcome;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkCheck {
    pub step_id: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkSession {
    pub workspace: PathBuf,
    pub source: Option<String>,
    pub revision: u64,
    pub max_attempts: u32,
    pub checks: Vec<WorkCheck>,
    pub submissions: Vec<WorkRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkRecord {
    pub attempt: u32,
    pub checks: Vec<CheckResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkSubmission {
    pub revision: u64,
    pub step_id: String,
    pub attempt: u32,
    pub outcome: Outcome,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkHumanAction {
    pub revision: u64,
    pub step_id: String,
    pub attempt: u32,
    pub outcome: Outcome,
    pub response: String,
}
