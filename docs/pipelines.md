# Pipeline configuration

Open **Pipelines** in the web UI to create, edit, reorder, or delete ordered chains. Each step selects a reusable agent. The handoff order is task input → first step → subsequent steps → output. This editor does not run pipelines.

Agents must already exist in `.baton/config.json` under `agents`; this feature does not provide agent CRUD. With no agents, pipeline creation is disabled and the page explains how to configure them. Reload after changing configuration externally.

Saving requires a pipeline name, at least one named step, and existing agent references. Validation errors identify configuration paths and retain the draft.

## API

- `GET /api/pipelines`: `{ revision, pipelines, agents }`
- `GET /api/pipelines/{id}`: `{ revision, pipeline }`
- `POST /api/pipelines`: `{ revision, pipeline }` → 201
- `PUT /api/pipelines/{id}`: `{ revision, pipeline }` → 204 (ID is immutable)
- `DELETE /api/pipelines/{id}`: `{ revision }` → 204

Pipeline and step fields follow the existing schema-v3 project configuration. IDs use ASCII letters, digits, `-`, `_`, or `.`. Treat the revision as opaque; send the revision returned when the draft was opened. A stale revision returns 409: reload before editing again. Invalid configuration returns 422; absent pipelines return 404. Mutations validate and atomically replace the existing project config without changing agents, tools, or skills. Existing task configuration snapshots are not modified when pipelines change or are deleted.
