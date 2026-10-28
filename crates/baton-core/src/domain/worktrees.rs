use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunWorktree {
    pub owner: String,
    pub path: PathBuf,
    pub branch: String,
    pub base_revision: String,
    pub lifecycle: String,
    pub cleanup_outcome: String,
}
