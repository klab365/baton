# Architecture

Baton is a local modular monolith with a separate web client. The Rust workspace separates business rules and use cases from filesystem, Git and model-provider integrations.

```text
Browser → embedded SvelteKit UI → HTTP /api → Axum (baton-server)
                                           ↓ mediator
                              baton-core: features + domain
                                           ↓ ports
                        baton-adapters: filesystem, Git, runtimes
```

## Workspace boundaries

- `baton-core` owns domain models and rules, feature commands and handlers, and outbound ports. `medi-rs` dispatches application commands to handlers. The core does not implement HTTP, Git, filesystem persistence or model-provider integrations.
- `baton-adapters` implements those ports: local repositories, Git worktrees, credentials, skill discovery and agent runtimes. The configured runtime selects Rig or the deterministic fake runtime according to each agent's settings.
- `baton-server` composes adapters and mediator resources, exposes Axum routes and translates between HTTP DTOs and application commands. It also generates the TypeScript DTOs in `packages/baton-web/lib/generated/`.
- `baton-cli` starts the API and embedded web UI for a project and provides direct commands for running pipelines and managing task/run lifecycle.
- `packages/baton-web/` contains the client-side SvelteKit UI. Release builds embed its static assets in the Rust server and serve API requests from the same origin. During development Vite proxies `/api` to `BATON_API_URL` (default `http://127.0.0.1:5173`).

## Request and execution lifecycle

1. The UI sends a same-origin API request. Axum validates and translates it and dispatches a typed command through the mediator.
2. A feature handler coordinates domain rules and ports. The server injects concrete adapter implementations when it constructs application state.
3. Project configuration describes agents, pipelines and transitions. Starting a task run captures the current configuration in the run; later configuration edits affect future runs, not that snapshot.
4. The pipeline engine routes agent outcomes and human decisions. A runtime executes agent steps in the task's Git worktree; Baton persists state and owns transitions, pauses and lifecycle. Baton does not automatically commit changes.
5. A task reuses its worktree across sequential runs. Completing or deleting it removes the worktree, including uncommitted changes. Interrupted in-process runs are recovered when the server starts.

## Persistence and safety

Project configuration lives in `.baton/config.json`; task and run records live under `.baton/tasks/` and `.baton/runs/`. These records can contain prompts and outputs. Credentials are kept separately in a local credential directory; configuration stores credential IDs rather than secret material. Run a single backend per project: independent concurrent backend processes are not supported.

Git worktrees isolate task branches and working directories, but do **not** confine shell commands, network access or credentials. Treat agent execution as code execution by the local user. See [orchestration](orchestration.md), [tasks](tasks.md), and [provider credentials](local-provider-storage.md) for details.
