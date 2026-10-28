use crate::domain::{project::ProjectConfig, worktrees::RunWorktree};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, alias = "prompt")]
    pub description: String,
    #[serde(default)]
    pub acceptance_criteria: String,
    pub pipeline_id: String,
    #[serde(default)]
    pub worktree: Option<RunWorktree>,
    pub status: TaskStatus,
    /// UTC timestamps in RFC 3339 format.
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    #[default]
    Draft,
    Done,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskInput {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub acceptance_criteria: String,
    pub pipeline_id: String,
    #[serde(default)]
    pub status: TaskStatus,
}

impl Task {
    pub fn create(
        id: String,
        input: TaskInput,
        config: ProjectConfig,
        now: String,
    ) -> Result<Self, String> {
        validate(&input, &config)?;
        Ok(Self {
            id,
            title: input.title,
            description: input.description,
            acceptance_criteria: input.acceptance_criteria,
            pipeline_id: input.pipeline_id,
            worktree: None,
            status: TaskStatus::Draft,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn update(
        &mut self,
        input: TaskInput,
        config: ProjectConfig,
        now: String,
    ) -> Result<(), String> {
        validate(&input, &config)?;
        self.title = input.title;
        self.description = input.description;
        self.acceptance_criteria = input.acceptance_criteria;
        self.pipeline_id = input.pipeline_id;
        self.status = input.status;
        self.updated_at = now.max(self.updated_at.clone());
        Ok(())
    }
}

fn validate(input: &TaskInput, config: &ProjectConfig) -> Result<(), String> {
    if input.title.trim().is_empty() {
        return Err("Title must not be empty".into());
    }
    if input.description.trim().is_empty() {
        return Err("Description must not be empty".into());
    }
    if !config
        .pipelines
        .iter()
        .any(|pipeline| pipeline.id == input.pipeline_id)
    {
        return Err("Selected pipeline does not exist".into());
    }
    Ok(())
}
