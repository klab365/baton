# Project configuration

`.baton/config.json` is versioned project data. Rust DTOs in
`baton-core::project` define its strict camelCase JSON format. Unknown fields
are rejected at every level. New projects start with:

```json
{
	"schemaVersion": 3,
	"agents": [],
	"pipelines": [],
	"tools": [],
	"skills": []
}
```

Reusable agents have `id`, `systemPrompt`, `model` (`provider`, `model`, optional
positive `maxOutputTokens`), `rules` (an array of nonblank strings), `toolIds`, and
`skillIds`. Pipelines have `id`, `name`, and a nonempty ordered `steps` array.
Each step has `id`, `name`, `agentId`, and `execution`; it cannot duplicate agent
model, tools, or skills. References must resolve in the same project.
Tools have `id` and `name`; skills add `instructions`. `execution.mode` is one of:
- `local`: a nonempty `command` array and optional nonblank `workingDirectory`.
- `docker`: nonblank `image`, nonempty `command`, optional `workingDirectory`.
- `remote`: nonblank `host`, nonempty `command`, optional `workingDirectory`.

This makes the same agent usable in differently configured pipeline steps. No
project-level execution targets or data planes exist. Remote credentials and
connection-specific local configuration are deliberately not represented.

IDs are nonempty ASCII letters/digits/`-`/`_`/`.` and unique within each entity
collection (step IDs within a pipeline). IDs are supplied by callers, never
generated from names or array positions. Preserve them on rename/reorder;
changing an ID requires updating all references in the same configuration.
Duplicate IDs are errors. Names, model identifiers, command values, and
instructions must not be blank.

## Versions

Only the exact v1 bootstrap shape `{"schemaVersion":1}` migrates to an empty
v3 configuration in memory. Reading never writes a migration. The original
bytes remain the revision token, so an explicit update can safely persist v3.
Version 2 is rejected: explicitly move each step's model/tool/skill settings into
an agent, add its system prompt and rules, and replace them with `agentId`.
Unknown legacy fields, missing versions, and unsupported versions fail closed;
no data is silently discarded. Future migrations must be explicit and tested.
All new writes require a validated current-version DTO.

## Repository operations

`get_project_config` returns missing, invalid (with diagnostics), or ready with
a `ConfigSnapshot`. Its opaque `revision` contains the exact source text and
must be retained for `update_project_config`. Initialization never replaces an
existing file. A stale update fails and callers must reload before retrying.

Filesystem writers exclusively create `.baton/config.lock`, compare the
revision under that lock, write and sync a unique same-directory temporary
file, then publish by atomic rename (or no-replace hard link for creation).
Readers therefore see a complete old or new file. Independent repository
instances/processes use the same lock. Failure cleans up owned temporary and
lock files; cancelling the async caller does not cancel an already-started
blocking write, which runs to completion with cleanup. After a process crash,
a stale lock must be removed manually **only after confirming no writer is
active**. A busy lock fails immediately rather than waiting indefinitely.

The concurrency contract requires all writers to honor `config.lock`.
Uncoordinated editors cannot be made compare-and-swap safe by portable
filesystem rename; a final revision check detects changes before publication
but cannot exclude an external edit in the final rename window. Use repository
operations for concurrent updates. Atomic visibility is guaranteed, not full
power-loss durability of the parent directory.

Never store API keys, tokens, secrets, local settings, or run state in project
DTOs. Environment variables or ignored `.baton/secrets.json`, `.baton/local.json`,
and `.baton/runs/` can hold such data; these are not read by this repository.
Free-form instructions and commands must also not contain secrets. No runtime
history or machine-specific execution credentials are serialized into config.
