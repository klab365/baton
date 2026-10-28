use std::{env, path::PathBuf};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if matches!(
        env::args().nth(1).as_deref(),
        Some("task-done" | "task-delete")
    ) {
        use baton_core::ports::tasks::TaskRepository;
        let args: Vec<String> = env::args().collect();
        if args.len() != 4 {
            anyhow::bail!("Usage: baton <task-done|task-delete> <project-directory> <task-id>");
        }
        let project = PathBuf::from(&args[2]).canonicalize()?;
        let tasks = baton_adapters::repositories::filesystem_task::FilesystemTaskRepository(
            project.clone(),
        );
        let runs =
            baton_adapters::repositories::filesystem_run::FilesystemRunRepository(project.clone());
        match args[1].as_str() {
            "task-done" => {
                let mut task = tasks.load(&args[3]).map_err(anyhow::Error::msg)?;
                let config = baton_core::domain::project::parse_config(&std::fs::read_to_string(
                    project.join(".baton/config.json"),
                )?)
                .map_err(anyhow::Error::msg)?;
                let input = baton_core::domain::tasks::TaskInput {
                    title: task.title.clone(),
                    description: task.description.clone(),
                    pipeline_id: task.pipeline_id.clone(),
                    status: baton_core::domain::tasks::TaskStatus::Done,
                };
                task = baton_core::features::tasks::update_task(
                    &tasks,
                    &runs,
                    &args[3],
                    input,
                    config,
                    baton_core::features::pipeline_engine::utc_now(),
                )
                .map_err(anyhow::Error::msg)?;
                println!("Task {} marked done", task.id);
            }
            "task-delete" => {
                baton_core::features::tasks::delete_task(&tasks, &runs, &args[3])
                    .map_err(anyhow::Error::msg)?;
                println!("Task {} deleted", args[3]);
            }
            _ => unreachable!(),
        }
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("cleanup") {
        use baton_core::{domain::pipeline::Status, ports::runs::RunRepository};
        let args: Vec<String> = env::args().skip(2).collect();
        if args.len() != 2 {
            anyhow::bail!("Usage: baton cleanup <project-directory> <run-id>");
        }
        let repository = baton_adapters::repositories::filesystem_run::FilesystemRunRepository(
            PathBuf::from(&args[0]).canonicalize()?,
        );
        let mut run = repository.load(&args[1]).map_err(anyhow::Error::msg)?;
        if matches!(
            run.status,
            Status::Running | Status::Pending | Status::WaitingHuman
        ) {
            anyhow::bail!("Cannot clean up an active run");
        }
        if run.task_id.is_some() {
            anyhow::bail!("Task worktrees are retained until the task is marked done or deleted");
        }
        let workspace = run
            .worktree
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Run has no allocated worktree"))?;
        let result = repository
            .worktree_manager()
            .map_err(anyhow::Error::msg)?
            .cleanup(&workspace.owner.clone(), workspace);
        repository.save(&run).map_err(anyhow::Error::msg)?;
        result.map_err(anyhow::Error::msg)?;
        println!("Worktree removed; branch retained");
        return Ok(());
    }
    // Runtime selection belongs exclusively to the snapshotted agent profile.
    if env::args().nth(1).as_deref() == Some("run") {
        let args: Vec<String> = env::args().skip(2).collect();
        if args.len() != 3 {
            anyhow::bail!("Usage: baton run <project-directory> <pipeline-id> <task-prompt>");
        }
        let project = PathBuf::from(&args[0]).canonicalize()?;
        let config = baton_core::domain::project::parse_config(&std::fs::read_to_string(
            project.join(".baton/config.json"),
        )?)
        .map_err(anyhow::Error::msg)?;
        let id = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let repository =
            baton_adapters::repositories::filesystem_run::FilesystemRunRepository(project);
        let credential_store =
            baton_adapters::credentials::FilesystemCredentialRepository::for_local_user()
                .map_err(anyhow::Error::msg)?;
        let auth_root = credential_store.root_path().to_path_buf();
        let credentials: std::sync::Arc<dyn baton_core::ports::credentials::CredentialRepository> =
            std::sync::Arc::new(credential_store);
        let run = baton_core::features::pipeline_engine::start_pipeline(
            &config,
            &args[1],
            &args[2],
            &id,
            &mut baton_adapters::runtime::ConfiguredRuntime::new(
                config.clone(),
                credentials,
                auth_root,
            ),
            &repository,
        )
        .map_err(anyhow::Error::msg)?;
        println!("{}", serde_json::to_string_pretty(&run)?);
        if run.status == baton_core::domain::pipeline::Status::Failed {
            anyhow::bail!("Pipeline run failed");
        }
        return Ok(());
    }
    let project_dir = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    let project_dir = project_dir.canonicalize()?;
    if !project_dir.is_dir() {
        anyhow::bail!("Project path is not a directory: {}", project_dir.display());
    }
    let host = "127.0.0.1";
    let port = env::var("BATON_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5173);
    println!("Baton is starting for {}", project_dir.display());
    println!("Web interface: http://{host}:{port}");
    baton_server::serve(project_dir, host, port).await
}
