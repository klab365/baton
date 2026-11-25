# Baton user guide

Baton coordinates local AI agents through configurable pipelines. You create agents and optional reusable skills, arrange them into a pipeline, and start tasks against that pipeline. Baton manages run state, transitions, human decisions and Git worktrees; the selected runtime executes agent steps.
- [Get started](#get-started)
- [Credentials and models](#credentials-and-models)
- [Agents and project instructions](#agents-and-project-instructions)
- [Skills](#skills)
- [Pipelines and routing](#pipelines-and-routing)
- [Tasks and worktrees](#tasks-and-worktrees)
- [Runs and human decisions](#runs-and-human-decisions)
- [Files, configuration and security](#files-configuration-and-security)
- [CLI reference](#cli-reference)
- [Troubleshooting and limitations](#troubleshooting-and-limitations)

For a complete worked pipeline, follow [Getting started: build a Software Factory](getting-started.md) and inspect the [demo configuration](../examples/demo/.baton/).

## Get started

1. Install the CLI using the [installation instructions](../Readme.md#install-the-cli). Release binaries include the API and web UI; Node and Bun are not required to run them.
2. In a trusted Git project, run `baton .` or `baton /path/to/project`. With no path, Baton uses the current directory. The project directory must exist.
3. On the first launch, accept the terminal initialization prompt. Alternatively, open the UI and choose **Set up project**. This creates an empty configuration, not a default pipeline. Open <http://127.0.0.1:5173>; set `BATON_PORT` to use a different port.
4. Under **Settings**, add a provider credential. Under **Skills**, optionally import reusable instructions. Create one or more **Agents**, then a **Pipeline**. Finally, create and start a **Task**.
5. Use **Runs** to inspect execution and respond when a human step is waiting.

A Git worktree is **not a sandbox**. Agents can execute shell commands with the Baton process's permissions, including access outside the worktree. Use trusted projects and a container, VM or dedicated OS user when stronger isolation is needed. Never put secrets in task prompts, project instructions, skill files or run data.

## Credentials and models

In **Settings → Add credential**, select a credential type, give it a name and enter the requested provider settings. Secret-input credentials require their secret; ChatGPT account credentials use a device-code sign-in flow. Follow the displayed link and code, and wait for the status to become available. The available credential types include OpenAI, Ollama and ChatGPT; provider availability and model listings depend on the provider and local configuration.

Credentials can be reused by multiple agents. The agent editor loads available models based on the selected credential. Deleting a credential leaves agents referencing it unable to start new runs until another available credential is assigned.

Baton stores secret material in a project-specific local credential vault, not project configuration. The default base directory is `~/.local/share/baton/credentials`; `BATON_CREDENTIAL_DIR` overrides it. Protect this directory and do not commit it. See [provider credentials](local-provider-storage.md).

## Agents and project instructions

In **Agents → Create agent**, set an ID, system prompt, runtime and any assigned skills. For a Rig agent, select a compatible credential and provider model. Edit or delete agents from the same page. A referenced agent cannot be deleted until pipelines using it are updated.
- **Rig** executes the agent with tools, including workspace file operations and shell commands. The system prompt should define its role, boundaries and expected handoff. An individual Rig step currently times out after 600 seconds.
- **Fake** is a deterministic offline runtime for tests; it does not contact a model and is not a substitute for real agent behavior.
- Provider/model and credential configuration belong to the **agent**, not the pipeline step. Avoid embedding secrets in prompts.

Baton does not automatically read or inject `AGENTS.md` from the task worktree. An agent may inspect it only when the task or an explicit prompt asks. See [runtime orchestration](orchestration.md).

## Skills

Skills are reusable instructions stored in `.agents/skills/<id>/SKILL.md`. Use **Skills** to create, edit, import or delete them. To import, select **Import skill** and provide a GitHub skill-folder URL such as `https://github.com/owner/repository/tree/<ref>/skills/example`. A commit SHA pins the chosen import revision; updates are not automatic. Review third-party instructions before trusting them.

Assign skills to an agent in the agent editor. Baton appends assigned `SKILL.md` bodies in the configured order to the agent prompt. Supporting files in the skill folder are not automatically injected into the prompt; skills do not grant permissions or automatically execute scripts. An assigned skill cannot be deleted until it is removed from agents. Missing assigned skills cause execution to fail. See [project skills](skills.md).

## Pipelines and routing

In **Pipelines → Create pipeline**, give the pipeline an ID (also used as its display name) and add agent or human-wait steps. Create agents first: a pipeline cannot be built without them. The first listed step starts the run. Only one step executes at a time; a pipeline can loop, but does not execute parallel branches.

### Sequential steps

With no conditional routes, an agent step continues to the next listed step after successful execution. Its response becomes the next handoff. A human step without routes continues on approval or response; rejection ends the run as failed.

### Conditional routes and outcomes

In a step's **Routing** section, choose **Add conditional route**, select an outcome and choose the destination step or **End run**. Outcomes are configured per **pipeline step**, not per agent. Current agent outcomes are `completed`, `approved`, `revise_build`, `revise_plan`, `blocked`, `failed`, `rejected` and `responded`; custom outcome names are not supported. Human steps allow `approved`, `rejected` and `responded`. Each outcome may appear only once per step, and destinations must exist.

When an agent step has routes, Rig and Pi expose `report_outcome` with only that step's configured outcomes. Its summary is the authoritative handoff to the next step and must include all needed context. A `blocked` outcome requires a nonblank `blocker_reason`; `human_action` is optional (and can be empty for compatibility with existing clients). Blocked means assistance is needed, not necessarily human assistance: the configured transition determines whether the next step is an agent or a person. Any supplied nonblank human action is included in the handoff. Update custom agent instructions that previously put essential context only in the final response. Baton routes using the tool decision, not JSON in the final response. For a missing or invalid decision, Baton asks up to twice in the same session; if no valid decision is reported, the attempt fails. Without routes, the final response remains the handoff and the outcome is implicitly `completed`. Existing persisted runs retain their recorded handoffs; this change does not rewrite them.

A route to **End run** ends successfully except for `failed`, `blocked` or `rejected`, which end as failed. New runs have a global limit of 50 started step visits by default, configurable as pipeline `maxAttempts` (1–1000); optional step `maxAttempts` sets an independent per-step cap. Failed and interrupted visits count, including human steps. Monitor runs and cancel one that is not making progress. The task, individual agent outputs and outcome summaries, and individual human responses are limited to 16,384 UTF-8 bytes each. The assembled step context can be larger. See [workflow transitions](workflow-transitions.md).

### Ask a person for clarification

A planner can route `blocked` to a human step with its question in its final response; configure the human step's `responded` route back to the planner. This is a new agent attempt, not an in-turn chat. The response becomes the next handoff. Explicit human responses remain available to later steps with their source; other earlier results are not automatically included. There is currently no custom `needs_input` outcome or dedicated ask-user tool.

## Tasks and worktrees

In **Tasks**, create a task with a nonempty title and description, including acceptance criteria, and select a pipeline. Saving creates a draft; it does not start execution. Open the task and choose **Start task** to create a run. Task details also allow editing, deletion and inspection of run history.

Each task has a separate Git worktree and task branch; subsequent runs for the same task reuse its worktree. Different tasks may run concurrently. Each run snapshots the current project configuration at start; later configuration changes affect future runs, not that graph. Files an agent explicitly reads from the worktree can nevertheless change.

**Marking a task done or deleting it force-removes its worktree**, including uncommitted changes and task-local Rig session data. Review and preserve anything needed beforehand. Task modification, deletion and duplicate execution are blocked while a run is waiting for a human. See [tasks](tasks.md).

Baton does not automatically commit, merge or push. A pipeline may configure an agent to create commits; give that agent explicit instructions about intended files, unrelated changes and credentials. Human approval by itself does not merge or push.

## Runs and human decisions

Open **Runs** and select a run to see its status, configured steps, ordered attempts, inputs and handoffs, execution trace and events. A step can have multiple attempts when a pipeline loops. Each step receives the immediately preceding handoff and explicit human responses from earlier human steps, with their provenance. Later corrections take precedence where applicable. Agent steps include the original task when there are explicit human responses; otherwise they omit it by default. Set `includeTask: true` to always include it in newly supplied context. Other earlier attempts remain available for inspection but are not automatically included. A reused agent session may still contain earlier conversation. Run state persists across a human pause and backend restart. Trace availability depends on the runtime; older runs may not contain a trace. Treat run data and transcripts as potentially sensitive.

When a run reaches a human step, it enters `waitingHuman`. In **Action required**, read the step context, optionally enter a response and select one of the configured outcomes. The chosen outcome determines the route; the response is the next handoff. Waiting has no automatic timeout or approval. A run can be cancelled while running or waiting; cancellation is cooperative for in-flight work, and a cancelled run cannot be resumed. Its task worktree remains until the task is done or deleted.

After a backend restart, a previously waiting run remains available for a human response. Baton attempts to resume interrupted managed agent steps with the saved pipeline snapshot and retained worktree; the previous attempt is marked failed and a fresh attempt is started. The agent is instructed to reconcile possible external side effects, but this is not an exactly-once guarantee. Interrupted shell steps, missing or invalid worktrees, and unavailable runtime configuration are not retried automatically and remain failed. See [directed workflows and human decisions](workflow-transitions.md).

## Files, configuration and security

| Location               | Purpose                                                                                  |
| ---------------------- | ---------------------------------------------------------------------------------------- |
| `.baton/project.yml`, `.baton/agents/`, `.baton/pipelines/` | Versioned project metadata, agents and pipelines; credential IDs, not secret values |
| `.baton/tasks/`        | Task data, including descriptions                                                        |
| `.baton/runs/`         | Run state, snapshots, handoffs, events and available traces                              |
| `.agents/skills/`      | Project-owned skill instructions and supporting files                                    |
| `AGENTS.md`            | Not loaded automatically; agents may inspect it when explicitly asked                    |
| Local credential vault | Secret material, outside project configuration                                           |

Baton creates `.baton/.gitignore` for task and run directories if missing; it does not replace an existing one. Ensure your own ignore rules protect private task and run data. Project configuration is validated and uses revision checks to reject stale edits. See [project configuration](project-configuration.md) for migrations and storage details.

Use one backend per project directory. Restrict access to the local UI, project files, worktrees and credential vault. Neither a worktree nor prompt instructions are security boundaries.

## Work in an existing coding-agent session

Use `baton run start` (and the other `baton run` stepwise subcommands; `baton work` remains an alias) to run pipeline steps through an already-open Pi, Claude Code or Codex session in the current Git worktree. This mode preserves externally owned workspaces, persists step handoffs, and supports explicit checks and human decisions. See [session work](session-work.md) for CLI usage and skill installation.

## CLI reference

```sh
baton [project-directory]                          # start local UI and API
baton run <project-directory> <pipeline-id> <task-prompt>
baton task-done <project-directory> <task-id>
baton task-delete <project-directory> <task-id>
baton cleanup <project-directory> <run-id>
```

`baton run` starts a pipeline directly and requires an existing configuration. `task-done` marks a task done; `task-delete` deletes it. Both can remove the task's worktree and uncommitted data. `cleanup` applies to eligible legacy run-owned worktrees, not task-owned worktrees; it retains the branch. Active runs cannot be cleaned up this way.

`BATON_PORT` changes the CLI's local server port. `BATON_CREDENTIAL_DIR` changes local credential storage. `BATON_BASE_REVISION` changes the Git base revision used for newly allocated worktrees; its default is `HEAD`. Installer-only settings are documented in the [README](../Readme.md#install-the-cli).

## Troubleshooting and limitations

- **Project setup screen:** initialize with **Set up project**. An invalid existing config is not silently replaced; inspect the displayed error and [configuration documentation](project-configuration.md).
- **Cannot select a model or start an agent:** check credential status, provider compatibility, network/provider availability and selected model. Deleting a credential invalidates agents that refer to it.
- **Skill unavailable:** check the assigned ID and `.agents/skills/<id>/SKILL.md`. Import is a one-time copy, not automatic synchronization. Existing skills may require explicit replacement confirmation.
- **Run fails at a routed agent step:** verify the agent called `report_outcome` with an outcome configured on that step. Check the step error and trace.
- **Run remains in a loop or waits:** Baton pauses before the next step when an agent reports the same `blocked`, `rejected`, `revise_build`, or `revise_plan` outcome and summary three times for the same step, ignoring case and whitespace. Intervening steps do not reset this count; a different outcome or summary from that step, or a human response at a configured human step, does. After continuing an escalation, another three identical findings trigger another pause. Inspect the recorded attempts and worktree, optionally enter guidance, and send a response to continue; cancellation is also available. The pause and worktree survive a server restart. Exact text matching can miss reworded or partially repeated findings and can flag a real, still-relevant blocker; it does not infer progress from file changes or stop ordinary successful build/review repetitions. Step and global `maxAttempts` limits remain independent hard limits (global default 50); reaching one ends the run as limit-exhausted rather than creating a resumable loop pause. Human waits never auto-approve.
- **Run interrupted by restart:** managed agent steps are retried in the retained worktree when possible. If recovery fails or a shell step was interrupted, inspect the failed run and worktree before starting another run.
- **Tests appear successful:** agent-reported checks are not a deterministic Baton quality gate. Inspect actual commands/results and rely on independent CI as appropriate.
- **Execution boundaries:** only local execution is supported; no remote execution targets or parallel branches. Native command/test pipeline steps, automatic merges and automatic Baton commits are not provided.

For a complete walkthrough see [Getting started](getting-started.md); for details see [Pipelines](pipelines.md), [Tasks](tasks.md), [Skills](skills.md) and [Runtime orchestration](orchestration.md).
