use std::{env, path::PathBuf};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
