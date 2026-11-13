use baton_cli::work;

use std::{
    env,
    io::{self, Write},
    path::PathBuf,
};

const BATON_GITIGNORE: &str = "runs/\ntasks/\n";

fn pi_install_source(version: &str, release_tag: Option<&str>) -> anyhow::Result<String> {
    let tag = format!("v{version}");
    if release_tag != Some(tag.as_str()) {
        anyhow::bail!(
            "This development build has no matching published Pi extension; install an official Baton release."
        );
    }
    Ok(format!(
        "https://github.com/klab365/baton/raw/refs/tags/{tag}/extensions/pi/baton.ts"
    ))
}

fn create_staging_file(
    target: &std::path::Path,
    nonce: &str,
) -> io::Result<(PathBuf, std::fs::File)> {
    let path = target.join(format!(".baton.ts.{nonce}.tmp"));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    Ok((path, file))
}

fn install_pi_with(
    version: &str,
    release_tag: Option<&str>,
    target: &std::path::Path,
    download: impl FnOnce(&str) -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<()> {
    let url = pi_install_source(version, release_tag)?;
    let contents = download(&url)
        .map_err(|error| anyhow::anyhow!("Could not download Baton Pi extension: {error}"))?;
    std::fs::create_dir_all(target).map_err(|error| {
        anyhow::anyhow!(
            "Could not create Pi extensions directory {}: {error}",
            target.display()
        )
    })?;
    let destination = target.join("baton.ts");
    let mut staged_file = None;
    for attempt in 0..100 {
        let nonce = format!(
            "{}.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            attempt
        );
        match create_staging_file(target, &nonce) {
            Ok(staged) => {
                staged_file = Some(staged);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "Could not stage Baton Pi extension: {error}"
                ));
            }
        }
    }
    let (temporary, mut staged) = staged_file
        .ok_or_else(|| anyhow::anyhow!("Could not allocate a unique Baton Pi staging file"))?;
    let result = staged
        .write_all(&contents)
        .map_err(|error| anyhow::anyhow!("Could not stage Baton Pi extension: {error}"))
        .and_then(|()| {
            drop(staged);
            std::fs::rename(&temporary, &destination)
                .map_err(|error| anyhow::anyhow!("Could not install Baton Pi extension: {error}"))
        });
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn install_pi(version: &str, release_tag: Option<&str>) -> anyhow::Result<()> {
    let home =
        env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let target = PathBuf::from(home).join(".pi/agent/extensions");
    install_pi_with(version, release_tag, &target, |url| {
        let output = std::process::Command::new("curl")
            .args(["-fLsS", url])
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(output.stdout)
    })?;
    println!(
        "Installed Baton Pi extension to {}",
        target.join("baton.ts").display()
    );
    Ok(())
}

fn bootstrap_project(project: &std::path::Path) -> anyhow::Result<bool> {
    let git = |args: &[&str]| -> anyhow::Result<std::process::Output> {
        Ok(std::process::Command::new("git")
            .current_dir(project)
            .args(args)
            .output()?)
    };
    let probe = git(&["rev-parse", "--is-inside-work-tree"])?;
    let new_repo = !probe.status.success();
    if new_repo {
        let output = git(&["init"])?;
        anyhow::ensure!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
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
    baton_adapters::repositories::yaml_project_config::ensure_project_identity(project)
        .map_err(anyhow::Error::msg)?;
    if new_repo {
        for args in [
            &["add", "--", ".baton/project.yml"][..],
            &[
                "commit",
                "-m",
                "Initialize Baton configuration",
                "--",
                ".baton/project.yml",
            ][..],
        ] {
            let output = git(args)?;
            anyhow::ensure!(
                output.status.success(),
                "git {} failed: {}",
                args[0],
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    Ok(was_missing)
}

async fn load_project_config(
    project: &std::path::Path,
) -> anyhow::Result<baton_core::domain::project::ProjectConfig> {
    use baton_core::ports::project_config::ProjectConfigRepository;
    let repository =
        baton_adapters::repositories::yaml_project_config::YamlProjectConfigRepository::new(
            project.to_path_buf(),
        );
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
        return install_pi(env!("CARGO_PKG_VERSION"), option_env!("BATON_RELEASE_TAG"));
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
            let error = pi_install_source("0.5.0", tag).unwrap_err().to_string();
            assert!(error.contains("development build"));
        }
    }

    #[test]
    fn staging_collision_does_not_modify_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let existing = temp.path().join(".baton.ts.test.tmp");
        std::fs::write(&existing, "keep me").unwrap();
        assert_eq!(
            create_staging_file(temp.path(), "test").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read_to_string(existing).unwrap(), "keep me");
    }

    #[test]
    fn release_extension_installs_only_baton_and_surfaces_download_errors() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("extensions");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("other.ts"), "keep").unwrap();
        std::fs::write(target.join("baton.ts"), "old extension").unwrap();
        install_pi_with("0.5.0", Some("v0.5.0"), &target, |url| {
            assert_eq!(
                url,
                "https://github.com/klab365/baton/raw/refs/tags/v0.5.0/extensions/pi/baton.ts"
            );
            Ok(b"extension".to_vec())
        })
        .unwrap();
        assert_eq!(
            std::fs::read(target.join("baton.ts")).unwrap(),
            b"extension"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("other.ts")).unwrap(),
            "keep"
        );
        assert!(
            install_pi_with("0.5.0", Some("v0.5.0"), &target, |_| Err(anyhow::anyhow!(
                "offline"
            )))
            .unwrap_err()
            .to_string()
            .contains("offline")
        );
        let blocked_target = temp.path().join("blocked");
        std::fs::create_dir_all(blocked_target.join("baton.ts")).unwrap();
        assert!(
            install_pi_with("0.5.0", Some("v0.5.0"), &blocked_target, |_| Ok(
                b"new".to_vec()
            ))
            .is_err()
        );
        assert!(blocked_target.join("baton.ts").is_dir());
        assert!(
            pi_install_source("0.5.0", Some("main"))
                .unwrap_err()
                .to_string()
                .contains("development build")
        );
    }

    #[test]
    fn bootstraps_missing_baton_directory_and_gitignore() {
        if env::var_os("BATON_TEST_GIT_IDENTITY").is_none() {
            let status = std::process::Command::new(env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::bootstraps_missing_baton_directory_and_gitignore",
                ])
                .env("BATON_TEST_GIT_IDENTITY", "1")
                .env("GIT_AUTHOR_NAME", "Baton Test")
                .env("GIT_AUTHOR_EMAIL", "baton@example.com")
                .env("GIT_COMMITTER_NAME", "Baton Test")
                .env("GIT_COMMITTER_EMAIL", "baton@example.com")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let project = TempDir::new();
        assert!(bootstrap_project(&project.0).unwrap());
        assert_eq!(
            std::fs::read_to_string(project.0.join(".baton/.gitignore")).unwrap(),
            BATON_GITIGNORE
        );
        let config = std::fs::read(project.0.join(".baton/project.yml")).unwrap();
        assert!(!bootstrap_project(&project.0).unwrap());
        assert_eq!(
            std::fs::read(project.0.join(".baton/project.yml")).unwrap(),
            config
        );
        let output = std::process::Command::new("git")
            .current_dir(&project.0)
            .args(["rev-list", "--count", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "1");
        let output = std::process::Command::new("git")
            .current_dir(&project.0)
            .args(["show", "HEAD:.baton/project.yml"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, config);
    }

    #[test]
    fn bootstrap_preserves_existing_gitignore() {
        let project = TempDir::new();
        assert!(
            std::process::Command::new("git")
                .current_dir(&project.0)
                .arg("init")
                .output()
                .unwrap()
                .status
                .success()
        );
        let baton = project.0.join(".baton");
        std::fs::create_dir(&baton).unwrap();
        std::fs::write(baton.join(".gitignore"), "custom/\n").unwrap();
        assert!(!bootstrap_project(&project.0).unwrap());
        assert_eq!(
            std::fs::read_to_string(baton.join(".gitignore")).unwrap(),
            "custom/\n"
        );
        assert!(baton.join("project.yml").exists());
        assert!(
            !std::process::Command::new("git")
                .current_dir(&project.0)
                .args(["rev-parse", "--verify", "HEAD"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}
