# Runtime orchestration

Baton owns pipeline routing, worktrees, human decisions and durable state. `AgentRuntime` is the generic outbound port; adapters live in `baton-adapters`. Agents select a reusable `runtimeProfileId`. Steps select only an agent (or a human decision); no runtime, model or provider selection is allowed on steps.

## Runtime profiles

Runtime profiles are configured in **Settings → Runtime profiles** and stored in `.baton/config.json`, for example `rig.openai.gpt-4.1`. A profile represents one runtime/provider/model combination and can be assigned to many agents. Profile IDs use safe ASCII segments and must match `runtime.provider.model`.

The current embedded runtime contract is `rig`. Its UI allows a provider and model to be recorded now, but execution intentionally fails until the Rig adapter is integrated. This preserves the project configuration and API shape while preventing a configured profile from silently falling back to another runtime.

`fake` is a deterministic offline runtime intended for tests. It must be explicitly configured and assigned; it returns `agentId: input` and never contacts a model.

Provider credentials are never stored in `.baton/config.json`, agent definitions, task snapshots or run records. The forthcoming Rig adapter will define its supported providers, credential sources and model discovery without changing the profile or orchestration contracts.

## Execution boundaries

Each task owns one worktree at `<absolute Git common directory>/baton-worktrees/<task-id>` on branch `baton/task-<task-id>`. Its sequential runs reuse that workspace; different tasks have independent worktrees and may run in parallel. Legacy run-owned worktrees are promoted when an older task is started again. Completion or deletion of the task explicitly cleans up its worktree. Exclusive reservations and ownership receipts prevent unsafe reuse. Runtime adapters must use that workspace as their working directory and never change the process-global current directory.

A worktree is an orchestration boundary, not an operating-system sandbox. The embedded runtime integration must document and enforce its own tool, process, network and credential boundaries. Use a container, VM or dedicated OS user when stronger isolation is required.
