# Runtime orchestration

Baton owns pipeline routing, worktrees, human decisions and durable state. `AgentRuntime` is the generic outbound port; adapters live in `baton-adapters`. Each agent directly selects its runtime kind, provider/model, and (where required) a credential ID. Pipeline steps select only an agent or a human decision.

## Agent runtime configuration

Agents are configured in the **Agents** UI. Provider/model and credential are separate choices: credentials are reusable local records, while each agent selects the model it uses. Only credential IDs are stored in `.baton/config.json`; secret material and refreshable auth records remain in the local vault. The selected credential type must match the provider integration.

The embedded runtime is `rig`. `fake` is a deterministic offline runtime intended for tests; it is explicitly selected and returns `agentId: input` without contacting a model.

Schema-v6 runtime profiles are migrated to direct agent settings on read. A profile shared by multiple agents is copied to each of them and then removed from the v7 configuration.

## Execution boundaries

Each task owns one worktree at `<absolute Git common directory>/baton-worktrees/<task-id>` on branch `baton/task-<task-id>`. Its sequential runs reuse that workspace; different tasks have independent worktrees and may run in parallel. Legacy run-owned worktrees are promoted when an older task is started again. Completion or deletion of the task explicitly cleans up its worktree. Exclusive reservations and ownership receipts prevent unsafe reuse. Runtime adapters must use that workspace as their working directory and never change the process-global current directory.

A worktree is an orchestration boundary, not an operating-system sandbox. The embedded runtime integration must document and enforce its own tool, process, network and credential boundaries. Use a container, VM or dedicated OS user when stronger isolation is required.
