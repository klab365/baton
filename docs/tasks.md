# Tasks

Tasks are runtime data stored as individual JSON files in `.baton/tasks/`, not in the versioned `.baton/config.json`. Keep this directory out of version control: new projects ignore it automatically; existing projects should add `tasks/` to `.baton/.gitignore`. Task snapshots may contain sensitive prompts and configuration; treat runtime data as private local data.

The API exposes `GET /api/tasks`, `POST /api/tasks`, and `GET`, `PUT`, `DELETE /api/tasks/{id}`. POST and PUT accept `title`, `description`, `acceptanceCriteria`, and `pipelineId`. The title and description must be nonblank; acceptance criteria are optional. The selected pipeline must exist in the current valid project configuration. Missing tasks return 404; invalid selections return 400. PUT replaces the editable fields. Status and timestamps are server-owned.

Tasks begin in `draft`; saving never starts execution. Creation captures the complete project configuration, including the selected pipeline and its referenced agents, tools and skills. Editing task text retains that immutable snapshot; selecting a different pipeline captures a new snapshot. Existing details remain readable if the pipeline is later removed. Updating such a task requires selecting a currently valid pipeline. Timestamps are UTC RFC 3339 strings with millisecond precision (for example, `2025-01-02T03:04:05.678Z`).

Writes use a synced temporary file and atomic rename. The backend serializes task operations within its process. Run one backend per project directory; concurrent independent backend processes are not supported. No execution-target resources or run-start endpoints are introduced.
