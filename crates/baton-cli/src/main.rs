use std::{env, path::PathBuf};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // baton run <project-directory> <pipeline-id> <task-prompt>
    if env::args().nth(1).as_deref() == Some("run") {
        let args: Vec<String> = env::args().skip(2).collect();
        if args.len() != 3 {
            anyhow::bail!(
                "Usage: baton run <project-directory> <pipeline-id> <task-prompt> (deterministic fake runtime)"
            );
        }
        let project = PathBuf::from(&args[0]).canonicalize()?;
        let config = baton_core::project::parse_config(&std::fs::read_to_string(
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
        let run = baton_core::orchestration::start_pipeline(
            &config,
            &args[1],
            &args[2],
            &id,
            &mut baton_core::orchestration::FakeRuntime::default(),
            &baton_server::run_repository::FilesystemRunRepository(project),
        )
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
