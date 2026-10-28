# Contribution conventions

- Use Conventional Commits for all commit messages (for example, `feat: add pipeline configuration`, `fix: reject stale config updates`, or `docs: clarify schema migration`).
- When creating a pull request, include a closing reference to its issue (for example, `Closes #8`) in the PR description.
- Run the narrowest relevant tests after changes. For workspace-level Rust or UI changes, use the corresponding `mise` tasks where practical.

# Architecture and implementation guidelines

## Core structure

- Organize `baton-core` by responsibility: `domain/` contains business models and rules, `features/` contains use cases and application commands, and `ports/` contains contracts required by the core.
- Keep the domain independent of adapters and infrastructure. Do not add Rig, Bevy, HTTP, filesystem, or Git implementations to `baton-core`.
- Put adapter implementations in `baton-adapters`. Adapters implement core ports; the server is responsible for composing them.
- Runtime profiles are not part of the project model; agents carry provider/model configuration and a credential ID directly. Keep secret material in the local credential vault, never in project config, snapshots, run data, logs, or version control.

## Mediator and commands

- Use `medi-rs` in `baton-core` to expose feature use cases. Define each command as a typed Rust struct in its feature module, normally deriving `MediCommand` and specifying its return and error types where needed.
- Implement command handlers with `#[medi_handler]`. Register feature resources and command-to-handler mappings in that feature's `medi_module!` manifest.
- Commands represent application use cases, not individual helper functions. Keep domain rules in `domain/`; have feature handlers coordinate ports and domain logic.
- Compose the mediator and inject adapter resources in `baton-server`. HTTP routes should validate/translate HTTP input, dispatch with `state.mediator.send(...)`, and map results/errors to HTTP responses; they should not implement use cases or access repositories directly.

## API and runtime boundaries

- Keep HTTP DTOs and TypeScript generation in `baton-server` (under its API/DTO code), not in `baton-core`.
- Keep `AgentRuntime`, `AgentRuntimeFactory`, repository contracts, and worktree contracts in `baton-core` ports. Implement concrete runtimes, tools, plugins, persistence, and Git worktrees in `baton-adapters`.
- Keep agent and pipeline behavior generic. Do not hard-code roles such as `writer` or `reviewer`; represent workflow behavior through project configuration and transitions.
- Baton controls task/run lifecycle, pipeline transitions, and human pauses. Rig executes agents; it does not own Baton lifecycle policy.
- Task runs share one persistent worktree per task; different tasks use different worktrees. A worktree is not a sandbox: shell execution is not guaranteed to stay within it.
- The configured commit agent owns task-branch commits. Do not add automatic Baton commits or rewrite task-branch history as part of pipeline execution.
