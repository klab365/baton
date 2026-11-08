use baton_cli::work;

use std::{
    env,
    io::{self, Write},
    path::PathBuf,
};

const BATON_GITIGNORE: &str = "runs/\ntasks/\n";

fn install_pi(
    version: &str,
    release_tag: Option<&str>,
    pi: &std::path::Path,
) -> anyhow::Result<()> {
    let tag = format!("v{version}");
    if release_tag != Some(tag.as_str()) {
        anyhow::bail!(
            "This development build has no matching published Pi package. Install an official Baton release, or use `pi install /absolute/path/to/baton` for local development."
        );
    }
    println!("Installing Baton Pi package for {tag}...");
    io::stdout().flush()?;
    let source = format!("git:github.com/klab365/baton@{tag}");
    let status = std::process::Command::new(pi)
        .args(["install", &source])
        .status()
        .map_err(|error| {
            anyhow::anyhow!("Could not run Pi: {error}. Install Pi and make sure `pi` is on PATH.")
        })?;
    if !status.success() {
        anyhow::bail!(
            "Pi could not install {source} ({status}). Check that the release tag exists and Pi can access GitHub, then retry `baton install pi`."
        );
    }
    Ok(())
}

fn bootstrap_project(project: &std::path::Path) -> anyhow::Result<bool> {
    let baton_dir = project.join(".baton");
    let was_missing = !baton_dir.exists();
    std::fs::create_dir_all(&baton_dir)?;
    let gitignore = baton_dir.join(".gitignore");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&gitignore)
    {
        Ok(mut file) => file.write_all(BATON_GITIGNORE.as_bytes())?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    Ok(was_missing)
}

async fn load_project_config(
    project: &std::path::Path,
) -> anyhow::Result<baton_core::domain::project::ProjectConfig> {
    use baton_core::ports::project_config::ProjectConfigRepository;
    let repository = baton_adapters::repositories::filesystem_project_config::FilesystemProjectConfigRepository::new(project.to_path_buf());
    match repository.load().await.map_err(anyhow::Error::msg)? {
        baton_core::domain::project::ConfigState::Ready(snapshot) => Ok(snapshot.config),
        baton_core::domain::project::ConfigState::Missing => {
            anyhow::bail!("Project configuration is missing")
        }
        baton_core::domain::project::ConfigState::Invalid(message) => anyhow::bail!(message),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if env::args().nth(1).as_deref() == Some("install") {
        let args: Vec<_> = env::args().skip(2).collect();
        if args != ["pi"] {
            anyhow::bail!("Usage: baton install pi");
        }
        return install_pi(
            env!("CARGO_PKG_VERSION"),
            option_env!("BATON_RELEASE_TAG"),
            std::path::Path::new("pi"),
        );
    }
    if env::args().nth(1).as_deref() == Some("work") {
        return work::run(env::args().skip(2).collect()).await;
    }
    if matches!(env::args().nth(1).as_deref(), Some("--version" | "-V")) {
        println!("baton {}", baton_server::VERSION);
        return Ok(());
    }
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
        let _project_lock = baton_server::lock_project_process(&project)?;
        let tasks = baton_adapters::repositories::filesystem_task::FilesystemTaskRepository(
            project.clone(),
        );
        let runs =
            baton_adapters::repositories::filesystem_run::FilesystemRunRepository(project.clone());
        match args[1].as_str() {
            "task-done" => {
                let mut task = tasks.load(&args[3]).map_err(anyhow::Error::msg)?;
                let config = load_project_config(&project).await?;
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
        let project = PathBuf::from(&args[0]).canonicalize()?;
        let _project_lock = baton_server::lock_project_process(&project)?;
        let repository =
            baton_adapters::repositories::filesystem_run::FilesystemRunRepository(project);
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
        let _project_lock = baton_server::lock_project_process(&project)?;
        let config = load_project_config(&project).await?;
        let id = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let skills = baton_core::ports::skills::SkillRepository::list(
            &baton_adapters::skills::FilesystemSkillRepository::new(project.clone()),
        )
        .map_err(anyhow::Error::msg)?;
        let credential_store =
            baton_adapters::credentials::FilesystemCredentialRepository::for_project(&project)
                .map_err(anyhow::Error::msg)?;
        let repository =
            baton_adapters::repositories::filesystem_run::FilesystemRunRepository(project);
        let auth_root = credential_store.root_path().to_path_buf();
        let credentials: std::sync::Arc<dyn baton_core::ports::credentials::CredentialRepository> =
            std::sync::Arc::new(credential_store);
        let run = baton_core::features::pipeline_engine::start_pipeline(
            &config,
            &args[1],
            &args[2],
            &id,
            &mut baton_adapters::runtime::ConfiguredRuntime::new(credentials, auth_root, skills),
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
    let project_lock = baton_server::lock_project_process(&project_dir)?;
    bootstrap_project(&project_dir)?;
    println!("Baton is starting for {}", project_dir.display());
    baton_server::serve_with_project_lock(project_dir, host, port, project_lock).await
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "baton-cli-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn development_and_mismatched_release_builds_are_rejected() {
        for tag in [None, Some("v0.4.0"), Some("main")] {
            let error = install_pi("0.5.0", tag, std::path::Path::new("pi"))
                .unwrap_err()
                .to_string();
            assert!(error.contains("development build"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn installs_the_selected_release_and_reports_failures() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let pi = temp.path().join("pi");
        let output = temp.path().join("args");
        std::fs::write(
            &pi,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
                output.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        for version in ["0.4.0", "0.5.0"] {
            install_pi(version, Some(&format!("v{version}")), &pi).unwrap();
            assert_eq!(
                std::fs::read_to_string(&output).unwrap(),
                format!("install\ngit:github.com/klab365/baton@v{version}\n")
            );
        }
        std::fs::write(&pi, "#!/bin/sh\nexit 1\n").unwrap();
        assert!(
            install_pi("0.5.0", Some("v0.5.0"), &pi)
                .unwrap_err()
                .to_string()
                .contains("release tag exists")
        );
        assert!(
            install_pi("0.5.0", Some("v0.5.0"), &temp.path().join("missing"))
                .unwrap_err()
                .to_string()
                .contains("on PATH")
        );
    }

    #[test]
    fn bootstraps_missing_baton_directory_and_gitignore() {
        let project = TempDir::new();
        assert!(bootstrap_project(&project.0).unwrap());
        assert_eq!(
            std::fs::read_to_string(project.0.join(".baton/.gitignore")).unwrap(),
            BATON_GITIGNORE
        );
        assert!(!bootstrap_project(&project.0).unwrap());
    }

    #[test]
    fn bootstrap_preserves_existing_gitignore() {
        let project = TempDir::new();
        let baton = project.0.join(".baton");
        std::fs::create_dir(&baton).unwrap();
        std::fs::write(baton.join(".gitignore"), "custom/\n").unwrap();
        assert!(!bootstrap_project(&project.0).unwrap());
        assert_eq!(
            std::fs::read_to_string(baton.join(".gitignore")).unwrap(),
            "custom/\n"
        );
    }
}
