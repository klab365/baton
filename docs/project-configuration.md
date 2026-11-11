# Project configuration

Project configuration lives in `.baton/`:

- `project.yml` holds the schema version and non-secret project ID.
- `agents/<id>.yml` holds one agent per file.
- `pipelines/<id>.yml` holds one pipeline per file.

The filename must match the object's `id`. Files are validated together before changes are saved; the revision token detects changes to any configuration file. Unchanged definitions retain their original formatting. Agent and pipeline files can be copied between projects, provided their references are available in the destination.

Agents contain `id`, `systemPrompt`, `runtimeKind`, optional `model` (`provider` and `model`), optional `credentialId`, `skillIds`, and optional `allowedTools`. Credential IDs are references only; secrets and OAuth records remain in the local credential vault.

Pipelines contain an `id` and a nonempty ordered `steps` array. Each step has a `type`: `agent` requires `agentId`, `human` waits for a decision, and `shell` requires `command` and optionally accepts `timeoutSeconds`. Steps do not contain runtime or credential configuration. Transitions refer to step IDs within the pipeline.

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
