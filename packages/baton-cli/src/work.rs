use anyhow::{Context, bail};
use baton_core::domain::work::{WorkCheck, WorkHumanAction, WorkSubmission};
use baton_core::{domain::project::ConfigState, ports::project_config::ProjectConfigRepository};
pub mod application;
use application::{WorkApplication, WorkCommand};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};

const HELP: &str =
    "Usage: baton work <start|next|execute|submit|status|respond|cancel|pipelines> [options]
  start --issue <task text> [--pipeline ID]
        [--max-attempts 50] [--check STEP=COMMAND ...]
  next --run ID
  execute --run ID
  submit --run ID --result FILE
  status [--run ID]
  respond --run ID --result FILE  (explicit human decision only)
  cancel --run ID
  pipelines
Common: --workspace PATH (current Git worktree by default)
Output: JSON on stdout; errors are JSON on stderr with a nonzero exit status.
Baton executes configured agents in the existing worktree; it does not create or delete it.";

struct Options {
    action: String,
    values: HashMap<String, String>,
    checks: Vec<String>,
}
impl Options {
    fn parse(args: Vec<String>) -> anyhow::Result<Self> {
        use clap::{Arg, ArgAction, Command};
        let option = |name: &'static str| Arg::new(name).long(name).value_name("VALUE");
        let common = |command: Command| command.arg(option("workspace"));
        let matches = Command::new("work")
            .disable_help_flag(true)
            .subcommand_required(true)
            .subcommand(
                common(Command::new("start"))
                    .arg(option("issue"))
                    .arg(option("pipeline"))
                    .arg(option("max-attempts"))
                    .arg(option("check").action(ArgAction::Append)),
            )
            .subcommand(common(Command::new("pipelines")))
            .subcommand(common(Command::new("next")).arg(option("run")))
            .subcommand(common(Command::new("execute")).arg(option("run")))
            .subcommand(common(Command::new("status")).arg(option("run")))
            .subcommand(common(Command::new("cancel")).arg(option("run")))
            .subcommand(
                common(Command::new("submit"))
                    .arg(option("run"))
                    .arg(option("result")),
            )
            .subcommand(
                common(Command::new("respond"))
                    .arg(option("run"))
                    .arg(option("result")),
            )
            .try_get_matches_from(std::iter::once("work".to_string()).chain(args))?;
        let (action, matches) = matches.subcommand().context(HELP)?;
        let mut values = HashMap::new();
        for name in [
            "workspace",
            "run",
            "issue",
            "pipeline",
            "max-attempts",
            "result",
        ] {
            if let Ok(Some(value)) = matches.try_get_one::<String>(name) {
                values.insert(format!("--{name}"), value.clone());
            }
        }
        let checks = matches
            .try_get_many::<String>("check")
            .ok()
            .flatten()
            .map(|values| values.cloned().collect())
            .unwrap_or_default();
        Ok(Self {
            action: action.to_string(),
            values,
            checks,
        })
    }
    fn required(&self, key: &str) -> anyhow::Result<String> {
        self.values
            .get(key)
            .cloned()
            .with_context(|| format!("Missing {key}\n{HELP}"))
    }
}

async fn application_pipelines(directory: &Path) -> anyhow::Result<Vec<String>> {
    let repository =
        baton_adapters::repositories::yaml_project_config::YamlProjectConfigRepository::new(
            directory.to_path_buf(),
        );
    match repository.load().await.map_err(anyhow::Error::msg)? {
        ConfigState::Ready(snapshot) => Ok(snapshot
            .config
            .pipelines
            .into_iter()
            .map(|pipeline| pipeline.id)
            .collect()),
        ConfigState::Missing => Ok(vec![]),
        ConfigState::Invalid(error) => bail!("Invalid project configuration: {error}"),
    }
}

fn workspace(path: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()?;
    if !output.status.success() {
        bail!("Run baton work inside an existing Git worktree");
    }
    PathBuf::from(String::from_utf8(output.stdout)?.trim())
        .canonicalize()
        .map_err(Into::into)
}
pub async fn run(args: Vec<String>) -> anyhow::Result<()> {
    match execute(args).await {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!("{}", serde_json::json!({"error": format!("{error:#}")}));
            std::process::exit(1);
        }
    }
}

async fn execute(args: Vec<String>) -> anyhow::Result<()> {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", serde_json::json!({"help": HELP}));
        return Ok(());
    }
    let options = Options::parse(args)?;
    let directory = workspace(
        &options
            .values
            .get("--workspace")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?),
    )?;
    if options.action == "pipelines" {
        println!(
            "{}",
            serde_json::to_string_pretty(&application_pipelines(&directory).await?)?
        );
        return Ok(());
    }
    let command = match options.action.as_str() {
        "start" => {
            let checks = options
                .checks
                .iter()
                .map(|s| {
                    let (step_id, command) =
                        s.split_once('=').context("Expected --check STEP=COMMAND")?;
                    Ok(WorkCheck {
                        step_id: step_id.into(),
                        command: command.into(),
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            WorkCommand::Start {
                task: options.required("--issue")?,
                source: None,
                pipeline: options.values.get("--pipeline").cloned(),
                max_attempts: options
                    .values
                    .get("--max-attempts")
                    .map(|s| s.parse())
                    .transpose()?
                    .unwrap_or(50),
                checks,
            }
        }
        "next" => WorkCommand::Next {
            id: options.required("--run")?,
        },
        "execute" => WorkCommand::Execute {
            id: options.required("--run")?,
        },
        "status" => WorkCommand::Status {
            id: options.values.get("--run").cloned(),
        },
        "cancel" => WorkCommand::Cancel {
            id: options.required("--run")?,
        },
        "submit" => {
            let result: WorkSubmission =
                serde_json::from_slice(&std::fs::read(options.required("--result")?)?)?;
            WorkCommand::Submit {
                id: options.required("--run")?,
                result,
            }
        }
        "respond" => {
            let action: WorkHumanAction =
                serde_json::from_slice(&std::fs::read(options.required("--result")?)?)?;
            WorkCommand::Respond {
                id: options.required("--run")?,
                action,
            }
        }
        _ => unreachable!(),
    };
    let application = WorkApplication::new(directory).map_err(anyhow::Error::msg)?;
    let value = application
        .execute(command)
        .await
        .map_err(anyhow::Error::msg)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn options_reject_mistakes_instead_of_silently_changing_behavior() {
        for args in [
            vec!["start", "--run", "old"],
            vec!["next", "--issue", "task"],
            vec!["start", "--pipeline", "a", "--pipeline", "b"],
            vec!["submit", "--text"],
        ] {
            assert!(Options::parse(args.into_iter().map(str::to_owned).collect()).is_err());
        }
    }
}
