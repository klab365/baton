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

The Rust workspace is organized as a modular monolith. The web UI is a separate client; it does not own project persistence or domain rules. Pipeline orchestration is implemented through a replaceable `AgentRuntime` port. The deterministic fake runtime is used for offline development; the opt-in Rig/OpenAI runtime is documented in [docs/orchestration.md](docs/orchestration.md).

## Development

Install dependencies and run the API and UI in the demo project:

```bash
mise install
bun install
mise run dev
```

The Axum API listens on `127.0.0.1:5173`; Vite serves the UI at `127.0.0.1:5174`. The demo project configuration is versioned at `examples/demo/.baton/config.json`.

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

The backend exposes project status and initialization endpoints. The UI includes project setup, an overview, and informational pages for tasks, pipelines, agents, and execution targets. Persistent tasks, CRUD execution UI, external integrations, and authentication are not implemented yet. The CLI supports durable sequential fake and opt-in OpenAI pipeline runs.
