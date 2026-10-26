use crate::{
    orchestration::{RunRepository, Status},
    project::ProjectConfig,
};
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
    /// Immutable configuration, including referenced agents, tools and skills.
    #[serde(deserialize_with = "crate::project::deserialize_snapshot")]
    pub config_snapshot: ProjectConfig,
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

pub trait TaskRepository {
    fn load(&self, id: &str) -> Result<Task, String>;
    fn save(&self, task: &Task) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// Apply task lifecycle rules without coupling callers to HTTP or filesystem APIs.
pub fn update_task(
    repository: &impl TaskRepository,
    runs: &impl RunRepository,
    id: &str,
    input: TaskInput,
    config: ProjectConfig,
    now: String,
) -> Result<Task, String> {
    let mut task = repository.load(id)?;
    let was_done = task.status == TaskStatus::Done;
    task.update(input, config, now)?;
    if task.status == TaskStatus::Done && !was_done {
        cleanup_task_worktrees(runs, id)?;
    }
    repository.save(&task)?;
    Ok(task)
}

pub fn delete_task(
    repository: &impl TaskRepository,
    runs: &impl RunRepository,
    id: &str,
) -> Result<(), String> {
    cleanup_task_worktrees(runs, id)?;
    repository.delete(id)
}

fn cleanup_task_worktrees(runs: &impl RunRepository, task_id: &str) -> Result<(), String> {
    for mut run in runs
        .list()?
        .into_iter()
        .filter(|run| run.task_id.as_deref() == Some(task_id))
    {
        if matches!(run.status, Status::Running | Status::WaitingHuman) {
            return Err("Cannot change or delete this task while it has an active run".into());
        }
        if let Some(worktree) = run.worktree.as_mut()
            && worktree.lifecycle != "removed"
        {
            runs.worktree_manager()?.cleanup(&run.id, worktree)?;
            runs.save(&run)?;
        }
    }
    Ok(())
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
            config_snapshot: config,
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
        // Preserve the original snapshot unless the pipeline selection changes.
        if self.pipeline_id != input.pipeline_id {
            self.config_snapshot = config;
        }
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
    if !config.pipelines.iter().any(|p| p.id == input.pipeline_id) {
        return Err("Selected pipeline does not exist".into());
    }
    Ok(())
}
