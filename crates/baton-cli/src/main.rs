use std::{env, path::PathBuf};

fn ensure_runtime_provider(
    config: &baton_core::project::ProjectConfig,
    pipeline_id: &str,
    runtime: &str,
) -> anyhow::Result<()> {
    let pipeline = config
        .pipelines
        .iter()
        .find(|pipeline| pipeline.id == pipeline_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown pipeline ID"))?;
    for step in pipeline.steps.iter().filter(|step| !step.human) {
        let agent = config
            .agents
            .iter()
            .find(|agent| agent.id == step.agent_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown agent"))?;
        if agent.model.provider != runtime {
            anyhow::bail!(
                "Runtime {runtime:?} does not match agent {:?} provider {:?}",
                agent.id,
                agent.model.provider
            );
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if matches!(
        env::args().nth(1).as_deref(),
        Some("task-done" | "task-delete")
    ) {
        use baton_core::tasks::TaskRepository;
        let args: Vec<String> = env::args().collect();
        if args.len() != 4 {
            anyhow::bail!("Usage: baton <task-done|task-delete> <project-directory> <task-id>");
        }
        let project = PathBuf::from(&args[2]).canonicalize()?;
        let tasks =
            baton_adapters::filesystem_task_repository::FilesystemTaskRepository(project.clone());
        let runs = baton_adapters::filesystem_run_repository::FilesystemRunRepository(project);
        match args[1].as_str() {
            "task-done" => {
                let mut task = tasks.load(&args[3]).map_err(anyhow::Error::msg)?;
                let config = task.config_snapshot.clone();
                let input = baton_core::tasks::TaskInput {
                    title: task.title.clone(),
                    description: task.description.clone(),
                    acceptance_criteria: task.acceptance_criteria.clone(),
                    pipeline_id: task.pipeline_id.clone(),
                    status: baton_core::tasks::TaskStatus::Done,
                };
                task = baton_core::tasks::update_task(
                    &tasks,
                    &runs,
                    &args[3],
                    input,
                    config,
                    baton_core::orchestration::utc_now(),
                )
                .map_err(anyhow::Error::msg)?;
                println!("Task {} marked done", task.id);
            }
            "task-delete" => {
                baton_core::tasks::delete_task(&tasks, &runs, &args[3])
                    .map_err(anyhow::Error::msg)?;
                println!("Task {} deleted", args[3]);
            }
            _ => unreachable!(),
        }
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("cleanup") {
        use baton_core::orchestration::{RunRepository, Status};
        let args: Vec<String> = env::args().skip(2).collect();
        if args.len() != 2 {
            anyhow::bail!("Usage: baton cleanup <project-directory> <run-id>");
        }
        let repository = baton_adapters::filesystem_run_repository::FilesystemRunRepository(
            PathBuf::from(&args[0]).canonicalize()?,
        );
        let mut run = repository.load(&args[1]).map_err(anyhow::Error::msg)?;
        if matches!(
            run.status,
            Status::Running | Status::Pending | Status::WaitingHuman
        ) {
            anyhow::bail!("Cannot clean up an active run");
        }
        let workspace = run
            .worktree
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Run has no allocated worktree"))?;
        let result = repository
            .worktree_manager()
            .map_err(anyhow::Error::msg)?
            .cleanup(&run.id, workspace);
        repository.save(&run).map_err(anyhow::Error::msg)?;
        result.map_err(anyhow::Error::msg)?;
        println!("Worktree removed; branch retained");
        return Ok(());
    }
    // baton run --runtime <fake|openai|chatgpt> <project-directory> <pipeline-id> <task-prompt>
    if env::args().nth(1).as_deref() == Some("run") {
        let args: Vec<String> = env::args().skip(2).collect();
        if args.len() != 5 || args.first().map(String::as_str) != Some("--runtime") {
            anyhow::bail!(
                "Usage: baton run --runtime <fake|openai|chatgpt> <project-directory> <pipeline-id> <task-prompt>"
            );
        }
        let runtime = match args[1].as_str() {
            "fake" | "openai" | "chatgpt" => args[1].as_str(),
            _ => anyhow::bail!(
                "Unknown runtime {:?}; use fake, openai, or chatgpt",
                args[1]
            ),
        };
        let project = PathBuf::from(&args[2]).canonicalize()?;
        let config = baton_core::project::parse_config(&std::fs::read_to_string(
            project.join(".baton/config.json"),
        )?)
        .map_err(anyhow::Error::msg)?;
        ensure_runtime_provider(&config, &args[3], runtime)?;
        let id = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let repository =
            baton_adapters::filesystem_run_repository::FilesystemRunRepository(project);
        let run = match runtime {
            "fake" => baton_core::orchestration::start_pipeline(
                &config,
                &args[3],
                &args[4],
                &id,
                &mut baton_adapters::FakeRuntime::default(),
                &repository,
            ),
            "openai" => baton_core::orchestration::start_pipeline(
                &config,
                &args[3],
                &args[4],
                &id,
                &mut baton_adapters::OpenAiRigRuntime,
                &repository,
            ),
            "chatgpt" => {
                let token_dir = env::var_os("BATON_CHATGPT_TOKEN_DIR")
                    .map(PathBuf::from)
                    .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/baton/chatgpt")))
                    .ok_or_else(|| anyhow::anyhow!("Could not determine a default ChatGPT token directory; set BATON_CHATGPT_TOKEN_DIR"))?;
                baton_core::orchestration::start_pipeline(
                    &config,
                    &args[3],
                    &args[4],
                    &id,
                    &mut baton_adapters::ChatGptRigRuntime::new(token_dir),
                    &repository,
                )
            }
            _ => unreachable!(),
        }
        .map_err(anyhow::Error::msg)?;
        println!("{}", serde_json::to_string_pretty(&run)?);
        if run.status == baton_core::orchestration::Status::Failed {
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
