# Project configuration

Project configuration lives in `.baton/`:

- `project.yml` holds the schema version and non-secret project ID.
- `agents/<id>.yml` holds one agent per file.
- `pipelines/<id>.yml` holds one pipeline per file.

The filename must match the object's `id`. Files are validated together before changes are saved; the revision token detects changes to any configuration file. Unchanged definitions retain their original formatting. Agent and pipeline files can be copied between projects, provided their references are available in the destination.

Agents contain `id`, `instructions`, `provider` (`rig`, `fake`, or `pi`), optional `model`, optional `credentialId`, `skillIds`, and optional `allowedTools`. Pi executes the installed `pi` CLI in the task worktree using Pi's locally configured authentication; Baton does not read or store Pi credentials. Pi is invoked headlessly (`--print --no-extensions --no-mcp`) in the task worktree, with the selected model and an explicit mapping of Baton tools to Pi built-ins (`shell`→`bash`, `list`→`ls`, `search`→`grep` and `find`). Pi uses its locally configured authentication; Baton does not read or store Pi credentials. Baton stores a separate Pi session per run and agent in the local vault and resumes it on subsequent invocations. Missing, invalid, or interrupted sessions require manual recovery; they are not silently replaced. For steps with transitions, Pi receives a `report_outcome` tool for one configured outcome and a summary. If no valid outcome is reported, Baton asks up to twice in the same session; exhaustion fails the step. Baton records setup metadata, but Pi's internal tool-call trace is not currently exposed. Cancellation kills the Pi process group on Unix, and pipe collection is bounded so a detached descendant cannot block the Baton run indefinitely; a process that deliberately escapes its process group may continue running after Baton reports the pipe timeout. The provider selects the agent runtime; a model is written as `model-provider/model-id`, for example `openai/gpt-4.1`. Credential IDs are references only; secrets and OAuth records remain in the local credential vault. Legacy `runtimeKind` and model objects are not supported; update old configuration and snapshots before loading them.

```yaml
id: builder
instructions: Implement the task.
provider: rig
model: openai/gpt-4.1
credentialId: openai-default
skillIds: []
```

Pipelines contain an `id` and a nonempty ordered `steps` array. Each step has a `type`: `agent` requires `agentId`, and agent steps may define a `prompt` for their specific task, `human` waits for a decision, and `shell` requires `command` and optionally accepts `timeoutSeconds`. Steps do not contain runtime or credential configuration. Transitions refer to step IDs within the pipeline.

```yaml
steps:
  - id: build
    name: Build
    type: agent
    agentId: builder
  - id: approval
    name: Approval
    type: human
  - id: test
    name: Test
    type: shell
    command: cargo test
```

Stored run snapshots use these step types. Older snapshots with implicit step types are converted when read and written in the new format on their next update.

Schema version 7 is current. The previous `.baton/config.json` format is not loaded automatically.
