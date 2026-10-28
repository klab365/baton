use std::{env, path::PathBuf};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let project_dir = env::var_os("BATON_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    let host = env::var("BATON_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = env::var("BATON_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5173);
    println!(
        "Baton API listening on http://{host}:{port} for {}",
        project_dir.display()
    );
    baton_server::serve(project_dir, &host, port).await
}
