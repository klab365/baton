use anyhow::{Context, bail};
use baton_core::domain::work::{WorkCheck, WorkHumanAction, WorkSubmission};
pub mod application;
use application::{WorkApplication, WorkCommand};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};

const HELP: &str = "Usage: baton work <start|next|submit|status|respond|cancel> [options]
  start --issue <task text> [--pipeline ID]
        [--max-attempts 50] [--check STEP=COMMAND ...]
  next --run ID
  submit --run ID --result FILE
  status [--run ID]
  respond --run ID --result FILE  (explicit human decision only)
  cancel --run ID
Common: --workspace PATH (current Git worktree by default)
Output: JSON on stdout; errors are JSON on stderr with a nonzero exit status.
Baton never starts a coding agent or creates/deletes a worktree in this mode.";

struct Options {
    action: String,
    values: HashMap<String, String>,
    checks: Vec<String>,
}
impl Options {
    fn parse(args: Vec<String>) -> anyhow::Result<Self> {
        let mut iter = args.into_iter();
        let action = iter.next().context(HELP)?;
        if !["start", "next", "submit", "status", "respond", "cancel"].contains(&action.as_str()) {
            bail!("{HELP}");
        }
        let mut options = Self {
            action,
            values: HashMap::new(),
            checks: vec![],
        };
        while let Some(key) = iter.next() {
            let allowed = match key.as_str() {
                "--workspace" | "--run" => true,
                "--issue" | "--pipeline" | "--max-attempts" | "--check" => {
                    options.action == "start"
                }
                "--result" => matches!(options.action.as_str(), "submit" | "respond"),
                _ => false,
            };
            if !allowed {
                bail!("Unknown option {key}\n{HELP}");
            }
            let value = iter
                .next()
                .with_context(|| format!("Missing value for {key}"))?;
            if key == "--check" {
                options.checks.push(value);
            } else if options.values.insert(key.clone(), value).is_some() {
                bail!("Duplicate option {key}");
            }
        }
        if options.action == "start" && options.values.contains_key("--run") {
            bail!("start creates a new run; use next --run ID to resume");
        }
        Ok(options)
    }
    fn required(&self, key: &str) -> anyhow::Result<String> {
        self.values
            .get(key)
            .cloned()
            .with_context(|| format!("Missing {key}\n{HELP}"))
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
