use crate::{
    domain::{
        pipeline::{PipelineRun, Status, TaskRunContext},
        project::{ConfigState, ProjectConfig},
    },
    features::pipeline_engine::{start_pipeline_for_task_with_worktree, utc_now},
    ports::{
        project_config::ProjectConfigRepository,
        runs::RunRepository,
        runtime::{AgentRuntime, AgentRuntimeFactory},
        tasks::TaskRepository,
    },
};
use medi_rs::{MediCommand, medi_handler, medi_module};
use std::sync::Arc;

pub use crate::domain::tasks::{Task, TaskInput, TaskStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRunStartError {
    NotFound,
    Conflict(String),
    Infrastructure(String),
    Pipeline(String),
}

#[derive(Clone)]
pub struct TaskFeatureResources {
    pub tasks: Arc<dyn TaskRepository>,
    pub runs: Arc<dyn RunRepository>,
    pub project_config: Arc<dyn ProjectConfigRepository>,
    pub runtime_factory: Arc<dyn AgentRuntimeFactory>,
}

#[derive(MediCommand)]
#[medi_command(return_type = Vec<Task>, error_type = String)]
pub struct ListTasks;

#[derive(MediCommand)]
#[medi_command(return_type = Task, error_type = String)]
pub struct GetTask {
    pub id: String,
}

#[derive(MediCommand)]
#[medi_command(return_type = Task, error_type = String)]
pub struct CreateTask {
    pub id: String,
    pub input: TaskInput,
    pub now: String,
}

#[derive(MediCommand)]
#[medi_command(return_type = Task, error_type = String)]
pub struct UpdateTask {
    pub id: String,
    pub input: TaskInput,
    pub now: String,
}

#[derive(MediCommand)]
#[medi_command(error_type = String)]
pub struct DeleteTask {
    pub id: String,
}

#[derive(MediCommand)]
#[medi_command(return_type = PipelineRun, error_type = TaskRunStartError)]
pub struct StartTaskRun {
    pub task_id: String,
    pub run_id: String,
}

/// Prepare and execute a run for a task, including its persistent task worktree.
pub fn start_task_run(
    task: &mut Task,
    task_repository: &dyn TaskRepository,
    runs: &dyn RunRepository,
    config: &ProjectConfig,
    run_id: &str,
    runtime: &mut dyn AgentRuntime,
) -> Result<PipelineRun, TaskRunStartError> {
    if task.status == TaskStatus::Done {
        return Err(TaskRunStartError::Conflict(
            "Completed tasks cannot be started".into(),
        ));
    }
    let related = runs
        .list()
        .map_err(TaskRunStartError::Infrastructure)?
        .into_iter()
        .filter(|run| run.task_id.as_deref() == Some(&task.id))
        .collect::<Vec<_>>();
    if related
        .iter()
        .any(|run| matches!(run.status, Status::Running | Status::WaitingHuman))
    {
        return Err(TaskRunStartError::Conflict(
            "This task already has an active run".into(),
        ));
    }
    let worktree = match task.worktree.clone() {
        Some(worktree) if worktree.lifecycle == "retained" => worktree,
        Some(_) => {
            return Err(TaskRunStartError::Conflict(
                "Task worktree has already been removed".into(),
            ));
        }
        None => {
            let previous = related
                .iter()
                .filter(|run| {
                    run.worktree
                        .as_ref()
                        .is_some_and(|workspace| workspace.lifecycle == "retained")
                })
                .max_by(|left, right| left.started_at.cmp(&right.started_at))
                .and_then(|run| run.worktree.clone());
            let worktree = match previous {
                Some(worktree) => worktree,
                None => runs
                    .worktree_manager()
                    .map_err(TaskRunStartError::Infrastructure)?
                    .allocate(&task.id)
                    .map_err(TaskRunStartError::Infrastructure)?,
            };
            task.worktree = Some(worktree.clone());
            task_repository
                .save(task)
                .map_err(TaskRunStartError::Infrastructure)?;
            worktree
        }
    };
    let prompt = task.description.clone();
    let previous_status = task.status;
    task.status = TaskStatus::Doing;
    task.updated_at = utc_now().max(task.updated_at.clone());
    task_repository
        .save(task)
        .map_err(TaskRunStartError::Infrastructure)?;
    match start_pipeline_for_task_with_worktree(
        config,
        &task.pipeline_id,
        &prompt,
        run_id,
        TaskRunContext {
            task_id: Some(task.id.clone()),
            worktree: Some(worktree),
        },
        runtime,
        runs,
    ) {
        Ok(run) => Ok(run),
        Err(message) => {
            if previous_status == TaskStatus::Draft {
                task.status = TaskStatus::Draft;
                task.updated_at = utc_now().max(task.updated_at.clone());
                task_repository
                    .save(task)
                    .map_err(TaskRunStartError::Infrastructure)?;
            }
            Err(TaskRunStartError::Pipeline(message))
        }
    }
}

async fn load_task_project_config(
    resources: &TaskFeatureResources,
) -> Result<ProjectConfig, String> {
    match resources.project_config.load().await? {
        ConfigState::Ready(snapshot) => Ok(snapshot.config),
        ConfigState::Missing => Err("A valid project configuration is required".into()),
        ConfigState::Invalid(message) => Err(message),
    }
}

#[medi_handler]
async fn list_tasks_handler(
    resources: TaskFeatureResources,
    _command: ListTasks,
) -> Result<Vec<Task>, String> {
    let mut tasks = resources.tasks.list()?;
    tasks.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(tasks)
}

#[medi_handler]
async fn get_task_handler(
    resources: TaskFeatureResources,
    command: GetTask,
) -> Result<Task, String> {
    resources.tasks.load(&command.id)
}

#[medi_handler]
async fn create_task_handler(
    resources: TaskFeatureResources,
    command: CreateTask,
) -> Result<Task, String> {
    let config = load_task_project_config(&resources).await?;
    let task = Task::create(command.id, command.input, config, command.now)?;
    resources.tasks.save(&task)?;
    Ok(task)
}

#[medi_handler]
async fn update_task_handler(
    resources: TaskFeatureResources,
    command: UpdateTask,
) -> Result<Task, String> {
    let config = load_task_project_config(&resources).await?;
    let tasks = resources.tasks;
    let runs = resources.runs;
    tokio::task::spawn_blocking(move || {
        update_task(
            tasks.as_ref(),
            runs.as_ref(),
            &command.id,
            command.input,
            config,
            command.now,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[medi_handler]
async fn delete_task_handler(
    resources: TaskFeatureResources,
    command: DeleteTask,
) -> Result<(), String> {
    let tasks = resources.tasks;
    let runs = resources.runs;
    tokio::task::spawn_blocking(move || delete_task(tasks.as_ref(), runs.as_ref(), &command.id))
        .await
        .map_err(|error| error.to_string())?
}

#[medi_handler]
async fn start_task_run_handler(
    resources: TaskFeatureResources,
    command: StartTaskRun,
) -> Result<PipelineRun, TaskRunStartError> {
    let lease = resources
        .tasks
        .acquire_operation_lease(&command.task_id)
        .map_err(|_| TaskRunStartError::Conflict("This task already has an active run".into()))?;
    let mut task = resources.tasks.load(&command.task_id).map_err(|message| {
        if message.contains("No such file") {
            TaskRunStartError::NotFound
        } else {
            TaskRunStartError::Infrastructure(message)
        }
    })?;
    let config = match resources
        .project_config
        .load()
        .await
        .map_err(TaskRunStartError::Infrastructure)?
    {
        ConfigState::Ready(snapshot) => snapshot.config,
        ConfigState::Missing => {
            return Err(TaskRunStartError::Pipeline(
                "A valid project configuration is required".into(),
            ));
        }
        ConfigState::Invalid(message) => return Err(TaskRunStartError::Pipeline(message)),
    };
    let mut runtime = resources
        .runtime_factory
        .create(&config, &task.pipeline_id)
        .map_err(TaskRunStartError::Pipeline)?;
    let tasks = resources.tasks;
    let runs = resources.runs;
    let run_id = command.run_id;
    tokio::task::spawn_blocking(move || {
        let _lease = lease;
        start_task_run(
            &mut task,
            tasks.as_ref(),
            runs.as_ref(),
            &config,
            &run_id,
            runtime.as_mut(),
        )
    })
    .await
    .map_err(|error| TaskRunStartError::Infrastructure(error.to_string()))?
}

medi_module! {
    pub manifest tasks_feature;
    resources { task_resources: crate::features::tasks::TaskFeatureResources; }
    commands {
        crate::features::tasks::ListTasks => crate::features::tasks::list_tasks_handler;
        crate::features::tasks::GetTask => crate::features::tasks::get_task_handler;
        crate::features::tasks::CreateTask => crate::features::tasks::create_task_handler;
        crate::features::tasks::UpdateTask => crate::features::tasks::update_task_handler;
        crate::features::tasks::DeleteTask => crate::features::tasks::delete_task_handler;
        crate::features::tasks::StartTaskRun => crate::features::tasks::start_task_run_handler;
    }
}

/// Apply task lifecycle rules without coupling callers to HTTP or filesystem APIs.
pub fn update_task(
    repository: &dyn TaskRepository,
    runs: &dyn RunRepository,
    id: &str,
    input: TaskInput,
    config: ProjectConfig,
    now: String,
) -> Result<Task, String> {
    let _lease = repository.acquire_operation_lease(id)?;
    let mut task = repository.load(id)?;
    let was_done = task.status == TaskStatus::Done;
    let new_status = input.status;
    if new_status == TaskStatus::Done && !was_done {
        let latest_run = runs
            .list()?
            .into_iter()
            .filter(|run| run.task_id.as_deref() == Some(id))
            .max_by(|left, right| left.started_at.cmp(&right.started_at));
        if !latest_run.is_some_and(|run| run.status == Status::Succeeded) {
            return Err("A task can only be marked done after a successful run".into());
        }
    }
    task.update(input, config, now)?;
    if task.status == TaskStatus::Done && !was_done {
        cleanup_task_worktrees(runs, &mut task)?;
    }
    repository.save(&task)?;
    Ok(task)
}

pub fn delete_task(
    repository: &dyn TaskRepository,
    runs: &dyn RunRepository,
    id: &str,
) -> Result<(), String> {
    let _lease = repository.acquire_operation_lease(id)?;
    let mut task = repository.load(id)?;
    cleanup_task_worktrees(runs, &mut task)?;
    repository.delete(id)
}

fn cleanup_task_worktrees(runs: &dyn RunRepository, task: &mut Task) -> Result<(), String> {
    let mut related = runs
        .list()?
        .into_iter()
        .filter(|run| run.task_id.as_deref() == Some(&task.id))
        .collect::<Vec<_>>();
    if related
        .iter()
        .any(|run| matches!(run.status, Status::Running | Status::WaitingHuman))
    {
        return Err("Cannot change or delete this task while it has an active run".into());
    }
    let mut worktrees = Vec::new();
    if let Some(worktree) = task.worktree.clone() {
        worktrees.push(worktree);
    }
    for worktree in related.iter().filter_map(|run| run.worktree.clone()) {
        if !worktrees
            .iter()
            .any(|existing| existing.owner == worktree.owner)
        {
            worktrees.push(worktree);
        }
    }
    for worktree in &mut worktrees {
        if worktree.lifecycle != "removed" {
            runs.worktree_manager()?
                .cleanup(&worktree.owner.clone(), worktree)?;
        }
    }
    task.worktree = worktrees
        .iter()
        .find(|worktree| worktree.owner == task.id)
        .cloned()
        .or_else(|| worktrees.first().cloned());
    for run in &mut related {
        if let Some(worktree) = run.worktree.as_ref().and_then(|old| {
            worktrees
                .iter()
                .find(|worktree| worktree.owner == old.owner)
        }) {
            run.worktree = Some(worktree.clone());
            runs.save(run)?;
        }
    }
    Ok(())
}
