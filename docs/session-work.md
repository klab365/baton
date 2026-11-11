# Work in an existing coding-agent session

`baton work` runs a configured Baton pipeline in an existing Git worktree without a server. A coding-agent TUI can use the CLI as an interface; Pi has a dedicated extension. The CLI executes configured agents through Baton adapters and the configured runtime. Baton does not allocate or remove this worktree.

Work and server runs use shared pipeline models and runtime adapters but have separate records, composition and workspace ownership. Work sessions are not shown in the server Runs UI.

## Install and configure

For Pi, install an official Baton release and Pi, then run `baton install pi` and restart or reload Pi. The package installs the UI extension from the CLI's release tag; repeat after upgrading Baton. For a development build use `pi install /absolute/path/to/baton` instead. Use `/baton` to enter a task and start a run, or `/baton RUN_ID` to resume. The extension shows steps and asks for an explicit decision and confirmation at human pauses. It invokes the CLI; it does not start Pi subagents.

The Pi extension shows the current step and completed results, not live Rig tool activity. It stops on a CLI error; inspect the run before retrying. To cancel a running step, invoke `baton work cancel --run RUN_ID` from another terminal.

To test the local checkout, run `mise run dev-pi` from the repository root. It builds the CLI and starts Pi with the local Baton extension; other Pi extensions are disabled for that session.

Other coding-agent TUIs, such as Claude Code and Codex, can invoke the CLI directly. They do not yet have a dedicated Baton UI integration.

Configure agents and a pipeline in `.baton/config.json`. Rig agents use their configured provider, model, tool permissions and credential ID; credentials must exist in Baton's local credential vault. Assigned project skills and project instructions are provided by the runtime. Agent skills and tools are not a security boundary: commands run with local user permissions and a worktree is not a sandbox.

Baton stores the task text literally and does not fetch issue URLs. Resolve an issue and include its requirements and acceptance criteria in the task text before starting.

## CLI

Run commands in an existing Git worktree, or supply `--workspace PATH`:

```sh
baton work start --issue 'Task and acceptance criteria' --pipeline factory
baton work status
baton work next --run RUN_ID
baton work execute --run RUN_ID
baton work respond --run RUN_ID --result human-response.json
baton work cancel --run RUN_ID
```

If only one pipeline exists, `--pipeline` may be omitted. Start creates a new run; use its ID to resume. `next` returns the current step, attempt, revision, allowed outcomes, checks and latest handoff without replaying the full trace. `execute` runs one agent or shell step and submits its result through Baton. Repeat until completion, an error, or a human pause. Outputs are JSON; errors are JSON on stderr with a nonzero exit code. `status --run ID` returns the persisted run.

Human steps require an explicit user decision via `respond`. Supply a JSON file containing the current `revision`, `stepId`, `attempt`, `outcome`, and `response`. This is a protocol distinction, not identity authentication: a process with shell access can invoke `respond`.

`submit` remains available for explicit structured submissions. A result contains `revision`, `stepId`, `attempt`, `outcome`, and a nonempty `summary`. Stale or invalid results are rejected. Integrations must not fabricate a result or human decision.

For routed agent steps, the configured runtime must report one of that step's configured outcomes. Baton does not accept `progress` or automatically repeat a step. Explicit workflow loops count toward the Work attempt limit, which defaults to 50 attempts including human steps and can be configured from 1 to 1000 via `--max-attempts`. A missing or invalid agent outcome stops the run instead of requesting another decision or retrying automatically. Historical runs containing `progress` remain readable, but unfinished runs no longer accept it.

Rig persists each agent's conversation under `runtime-sessions/<run-id>/agent-<agent-id>/` in the project's local credential-vault directory, outside the workspace. Subsequent attempts of the same agent in that run restore its conversation and receive the current task and handoffs; different agents and new runs do not share conversations. These files can contain conversation and tool output; keep secrets out of them and out of version control. Sessions from the former workspace-wide layout are not automatically imported, because their run ownership cannot be verified.

Rig's compaction plugin manages model context near the provider's window. Before starting a turn, Baton additionally rejects a restored conversation larger than 4 MiB of serialized JSON; reconcile the workspace and start a new run if this limit is reached. Baton does not silently replace a missing or unreadable expected session with empty history.

## Checks and workspace ownership

Add explicit process gates at start, for example `--check 'build=cargo test'`. Baton runs applicable checks in the worktree before accepting successful outcomes. Failed checks reject the submission; successful results are persisted separately from agent summaries. Checks time out after 300 seconds and their recorded output is bounded. Do not include secrets in task text, checks or summaries.

Work records are stored under `.baton/work/`, independently of server records. The workspace remains externally owned; completion or cancellation never removes it. Configuration is snapshotted at start. Concurrent changes to the same workspace remain the user's responsibility.

While an agent or command runs, status and cancellation commands may run concurrently. Only one Work execution can use the workspace at a time. Cancellation is cooperative and does not undo changes already made. A failed or interrupted Rig execution leaves `baton-inflight` in its agent session directory and cannot be retried automatically. Inspect the run, session and workspace and reconcile possible side effects (including commits and network actions) before manually removing that marker and retrying. If the expected session is missing, corrupt or repaired during restoration, stop and reconcile rather than assuming the prior context survived. In server mode an interrupted in-flight execution fails on restart instead of replaying automatically; after reconciliation, start a new run. There are no automatic commits, merges or issue updates.
