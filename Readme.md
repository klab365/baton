# Baton

Baton orchestrates specialized AI agents through configurable pipelines. The application uses a Rust backend and a SvelteKit web UI.

## Architecture

```text
SvelteKit UI (web/)
      │ HTTP / JSON
      ▼
Axum API + CLI (crates/baton-server, crates/baton-cli)
      │ inbound adapters
      ▼
Application use cases and domain (crates/baton-core)
      │ outbound ports
      ▼
Filesystem configuration (.baton/config.json)
```

The Rust workspace is organized as a modular monolith. The web UI is a separate client; it does not own project persistence or domain rules. Pipeline orchestration is implemented through a replaceable `AgentRuntime` port. Agents select reusable runtime profiles; the embedded Rig profile contract is ready for its adapter integration. The deterministic fake runtime requires explicit selection for offline tests. Setup, breaking migration, and security boundaries are documented in [docs/orchestration.md](docs/orchestration.md).

## Development

Install dependencies and run the API and UI in the demo project:

```bash
mise install
bun install
mise run dev
```

The Axum API listens on `127.0.0.1:5173`; Vite serves the UI at `127.0.0.1:5174`. The demo project configuration is versioned at `examples/demo/.baton/config.json`. See [runtime orchestration](docs/orchestration.md) for runtime-profile setup and the isolated artifact-writing/review pipeline.

## Checks

```bash
mise run check-format
mise run lint
mise run check
mise run test
mise run build
```

## Run against another project

Start the backend for a project directory:

```bash
BATON_PROJECT_DIR=/path/to/project cargo run -p baton-server
```

Then start the UI in another terminal:

```bash
bun run dev -- --port 5174
```

The first UI setup action creates `.baton/config.json` through the backend API. The project name is derived from the directory name.

## Current status

The backend and UI support project configuration, reusable agents/runtime profiles, pipelines, tasks, isolated runs, runtime-profile configuration, isolated runs, and human decisions. Baton never manages provider credentials. Remote execution targets are not implemented.
