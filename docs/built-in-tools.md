# Built-in tools (v1)

The host owns tool definitions and implementations (`baton_core::tools`). The Tools page enables built-ins project-wide; the agent editor grants each agent a selected subset. Project prompts cannot create tools or widen permissions. Tool CRUD uses the existing versioned project-config repository; disabling a tool assigned to an agent is rejected.

Built-ins: `builtin.list.v1`, `builtin.read.v1`, `builtin.search.v1`, `builtin.write.v1`, `builtin.patch.v1`, and `builtin.delete.v1`. Inputs are typed closed JSON objects; outputs are `{ "content": string }`. Schemas, permission summaries, version, and limits are exposed by the host catalog. Search is literal, bounded search implemented in Rust; no external `rg` or `fd` process runs. `builtin.shell.v1` is tracked separately in #35 and is not enabled yet.

## Worktree boundary

Tools execute only after orchestration validates the run-owned Git worktree. They require an absolute canonical worktree under `baton-worktrees`, use capability-relative filesystem access, never fall back to cwd, and fail closed outside Unix. They operate on relative project paths across the worktree, not just an artifacts directory.

Absolute paths, traversal, hidden path components (including `.git` and `.baton`), `target`, `node_modules`, symlinks and multiply-linked files are rejected. Reads are nonblocking/no-follow and bounded to 8192 bytes. Writes are confined to the worktree. Patch requires a unique expected-text match; delete requires explicit `DELETE` confirmation. Directory listing and search have bounded results. The model cannot use shell syntax, subprocesses, Git, network, credentials or external targets through these tools. No tool can reach the Baton checkout or primary checkout; modifications are only made in the allocated run worktree.

Credential-shaped lines are redacted defensively; pattern matching is not a general secret detector. Avoid exposing secrets in task prompts or agent responses. Tool history stores only the selected ID (or `unknown`) and success/failure, never arguments or file contents. Agent final responses remain part of normal run history.

Limits: eight calls per step, 256-byte paths, 16 path components, 100 directory entries/results, 8192-byte files/content/results, and a 60-second session deadline checked before and after provider calls. Provider requests retain their own timeouts; the session deadline does not preempt an in-flight provider request. Invalid schemas, authorization failures, containment violations, limits and I/O errors stop the step without retry. OS paths and contents are not included in errors.

## Demo

The demo writer has project read/list/search/write/patch/delete grants; the reviewer has read/list/search only. The writer prompt directs edits to the isolated project worktree, and the reviewer inspects the resulting changes before the pipeline waits for human approval. The scripted integration test runs this flow in a real Git worktree without provider credentials. A live run requires a locally configured and verified provider selected for the agents.
