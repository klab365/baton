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
    for step in &pipeline.steps {
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
    // baton run --runtime <fake|openai> <project-directory> <pipeline-id> <task-prompt>
    if env::args().nth(1).as_deref() == Some("run") {
        let args: Vec<String> = env::args().skip(2).collect();
        if args.len() != 5 || args.first().map(String::as_str) != Some("--runtime") {
            anyhow::bail!(
                "Usage: baton run --runtime <fake|openai> <project-directory> <pipeline-id> <task-prompt>"
            );
        }
        let runtime = match args[1].as_str() {
            "fake" | "openai" => args[1].as_str(),
            _ => anyhow::bail!("Unknown runtime {:?}; use fake or openai", args[1]),
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
        let repository = baton_server::run_repository::FilesystemRunRepository(project);
        let run = match runtime {
            "fake" => baton_core::orchestration::start_pipeline(
                &config,
                &args[3],
                &args[4],
                &id,
                &mut baton_core::orchestration::FakeRuntime::default(),
                &repository,
            ),
            "openai" => baton_core::orchestration::start_pipeline(
                &config,
                &args[3],
                &args[4],
                &id,
                &mut baton_core::orchestration::OpenAiRigRuntime,
                &repository,
            ),
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
