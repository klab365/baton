# Work in an existing coding-agent session

`baton work` lets Pi or another host with a separate-session subagent facility orchestrate a configured Baton pipeline in an existing Git worktree. Herdr can create that worktree before the coding agent starts. The Baton CLI does not launch agents or allocate/remove a worktree in this mode; the bundled Pi extension can launch a subagent when a step requests one.

The standalone Work CLI and the managed-runtime/server mode share core pipeline models and transition rules, but have separate composition, state and locks. Work sessions are controlled entirely through the CLI and skill, including human actions via `baton work respond`. They are not exposed in the server API or Runs UI.

## Install

Install a Baton binary built from a revision containing `baton work`; older release binaries do not support this protocol. From source:

```sh
cargo build --release -p baton-cli
# Put target/release/baton on PATH, or invoke it by its absolute path.
```

For Pi, install this repository's skill package (after this change is available on the selected Git ref):

```sh
pi install git:github.com/klab365/baton
```

Restart Pi or reload its resources, then invoke `/skill:baton <issue URL or task>`. The package installs instructions, not the Rust binary. For local development, `pi install /absolute/path/to/baton` loads the local package.

The same `skills/baton` directory can be installed under `~/.agents/skills/baton` for Codex or `~/.claude/skills/baton` for Claude Code. Keep one maintained copy and use your preferred copy/symlink installation. No host-specific frontmatter is required.

Automatic skill selection does not guarantee that every issue uses Baton. Add an explicit project instruction, for example:

> For issue implementation, use the Baton skill and start or resume the configured factory run before changing code. Follow its steps and report the terminal result or pending human decision.

## Configure a pipeline

Use the existing Agents/Pipelines UI or commit a project configuration. `examples/session-factory/.baton/config.json` is an illustrative configurable build/review/human pipeline; copy its contents into a new project's configuration, not over an existing one. Agent system prompts and assigned skills provide step instructions. In external mode, the active session supplies its own model and tools: Baton does not instantiate the configured managed runtime or enforce that runtime's tool allowlist. Instructions and a skill are not a security boundary.

Task acceptance criteria come from the supplied issue or prompt. Baton stores the supplied request verbatim in `task`, without fetching URLs or resolving issue IDs. The planning agent reads referenced issues with its own tools and keeps their requirements in its session context; Work does not repeat them at each step. It does not synthesize or replace acceptance criteria. Process gates such as build/test commands are configured separately.

## CLI

Run these commands inside the existing worktree (nested directories resolve to its Git root), or pass `--workspace PATH`:

```sh
baton work start --issue https://github.com/owner/repo/issues/123 --pipeline factory
baton work start --issue 'Fix login. Acceptance criteria: ...' --pipeline factory
baton work status
baton work next --run RUN_ID
baton work submit --run RUN_ID --result result.json
baton work respond --run RUN_ID --result human-response.json
baton work cancel --run RUN_ID
```

`--issue` always accepts literal task text. For example, `"Löse Issue https://github.com/owner/repo/issues/123"` is passed through unchanged. The planning agent reads the referenced issue using its available tools and authentication. Baton itself performs no issue imports, URL detection or source-specific requests; neither `--text` nor `--repo` is needed.

If exactly one pipeline exists, `--pipeline` may be omitted. Otherwise select explicitly. Start always creates a new run; resume by fetching `next` with the existing ID. Status without an ID lists compact progress for external sessions; status with an ID returns the entire saved run.

All successful commands, including help, print JSON to stdout. Failures always emit a JSON error object to stderr and exit with a nonzero status. `next` is read-only and returns the same attempt until it is submitted. It includes the current step, agent instructions, assigned skill bodies and their supporting-file directory, allowed outcomes, checks and run/attempt/revision identifiers. It does not include the original task, earlier human responses or previous handoffs: the running coding-agent session owns that context.

`start`, `submit`, `respond` and `cancel` return compact progress (`id`, `status`, `revision`, `currentStep`, `attempt`), rather than replaying the run history. The original issue/prompt, decisions, results and check evidence remain persisted and are available through an explicit `status --run ID` read. Use that on demand when entering a genuinely new coding-agent session; it is not a copy of the complete Pi chat. Work does not construct or persist cumulative step-input prompts. The existing history/context feature for Baton-managed agent runtimes is unchanged.

Submission files use camelCase:

```json
{
	"revision": 0,
	"stepId": "build",
	"attempt": 1,
	"outcome": "completed",
	"summary": "Implemented the change; acceptance criteria checked as follows: ...",
	"evidence": [
		"src/example.rs",
		"cargo test: actual result or artifact reference"
	]
}
```

Use the exact revision, step ID and attempt from `next`. Revisions advance only when a submission or human response is accepted. Unknown fields/outcomes, stale attempts, empty evidence, or outcomes absent from the configured step are rejected without advancing. Successful un-routed agent steps use `completed`; routed steps use their configured outcomes.

A human-response file uses the same revision/step/attempt identifiers, plus `outcome` and `response` instead of `summary`/`evidence`. It is accepted only at a human step. `submit` cannot complete a human step. This is a protocol distinction, not identity authentication: an agent with shell access can technically invoke `respond`; the skill must wait for an actual user decision.

## Verifiable process gates and bounded loops

Explicitly attach a check to a step at run creation:

```sh
baton work start --issue 'Fix login; acceptance criteria: ...' \
  --pipeline factory --check 'build=cargo test' --check 'review=cargo clippy -- -D warnings' \
  --max-attempts 50
```

Checks are snapshotted with the session and executed by Baton in the external workspace when that step submits `completed`, `approved`, or `responded`, or any outcome that terminates the run successfully. A nonzero exit, signal, or timeout rejects the submission without advancing; fix the issue and retry. Successful check results (command, exit status, bounded combined output) are persisted separately from agent-reported evidence. Checks time out after 300 seconds, with process-group termination on Unix. Output recorded in the run is limited to 16 KiB per check; stdout is recorded before stderr, not necessarily in chronological order. Commands execute with Baton user permissions and are not sandboxed. Do not include credentials in check commands or output.

A Work invocation holds the workspace Work lock until its command and checks finish. Concurrent Work invocations fail with a busy-lock error; retry after the active command finishes. If Pi or a Work process is interrupted, inspect `status`/`next` before resubmitting. Accepted steps remain persisted; unfinished checks or changes require reconciliation by the worker.

The default limit is 50 total attempts (including human steps), configurable from 1 to 1000 with `--max-attempts`. Rework follows existing transitions; reaching the limit stops with `limitExhausted`. No implicit retries, cost-limit increases, automatic commits, merges or issue updates are performed.

## Persistence and workspace ownership

Sessions live under the workspace's `.baton/work/`, independently of managed-runtime run records. Work has its own OS lock at `.baton/work/process.lock`; the lock is released when the command exits or the process crashes. This directory is ignored through its own `.gitignore` without changing existing ignore rules.

Each invocation composes only the Work mediator, project configuration reader, Work repository, assigned skills and check runner. There is no HTTP request, server detection, port, credential vault or server-runtime initialization. `--server` is not supported. A running Baton server does not block Work, and Work records are not scanned or recovered by server startup. Restarting Pi or the Work CLI leaves the last accepted step and the open attempt intact.

Both modes may read the project's pipeline configuration, which is snapshotted when Work starts. They do not share execution state or task/worktree ownership. Concurrent code changes in the same workspace remain the user's responsibility.

The external path is stored in `session.workspace`; `worktree` and `taskId` remain empty. Completion/cancellation do not clean up that path. Existing task/worktree cleanup cannot claim ownership of it. The user/Herdr retains responsibility for workspace lifecycle.

Every external Work agent step is delegated to a separate subagent session by the orchestrator. The Baton skill stops rather than executing the step itself when delegation is unavailable. The Pi tool passes the original task, prior step outputs and human decisions, current agent instructions and skills, plus relevant context supplied by the orchestrator. It rejects context over 256 KiB instead of silently omitting decisions. For Pi, the Baton package includes the `baton_subagent` extension; installing the package loads it alongside the skill. The extension launches a separate `pi` process using the current model and thinking level, and requires `pi` and `baton` on PATH plus working Pi model credentials. The subagent runs in an ephemeral session; the Pi tool displays live tool names and completion status, without recording the subagent transcript. Its final output is validated before the parent submits it. The extension does not prove reviewer identity. The same pipeline configuration works with managed-runtime runs, where Baton uses the configured runtime instead of the external host.

Review in the same worker context is self-review. A separate context provides a more independent review, but shares the worktree and host permissions and is not a sandbox. Human decisions remain explicit and are never delegated.
