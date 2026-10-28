# Pipelines

Create agents in the **Agents** UI before building a pipeline. With no agents, pipeline creation is disabled. Agent definitions remain in `.baton/config.json`; agents reference explicitly configured runtime profiles such as `rig.openai.gpt-4.1`. Runtime/provider/model selection never belongs on a step.

A pipeline is an ordered graph of agent and human-decision steps. Each transition has a bounded traversal count. For agent steps with transitions, Rig exposes a `report_outcome` tool limited to that step's configured outcomes. Agents report a concise handoff through the tool while keeping their natural-language response separate; Baton rejects missing or disallowed decisions. Baton records step inputs, outputs, outcomes and handoffs in the run state.

Agent create and update requests reject unknown runtime profile IDs. Runtime configuration is project-owned; credentials are never accepted through pipeline or agent APIs. Deleting an agent referenced by a pipeline returns 409 with the pipeline and step to update.

A project may add a commit agent after review and before human approval. That agent inspects and commits task changes in the task worktree; Baton does not create commits. Keep its instructions explicit about excluding session data and unrelated files, and have it call the configured `report_outcome` control tool after committing.
