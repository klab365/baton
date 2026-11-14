# Pipelines

Create agents in the **Agents** UI before building a pipeline. With no agents, pipeline creation is disabled. Agent definitions are stored in `.baton/agents/`; each agent directly selects its provider/model and credential. Runtime/provider/model selection never belongs on a step.

A pipeline is an ordered graph of agent and human-decision steps. Transitions have no traversal limit; repeated cycles continue until an outcome takes a different edge or the run is cancelled. For agent steps with transitions, Rig and Pi expose a `report_outcome` tool limited to that step's configured outcomes. The tool summary is the authoritative complete handoff; missing decisions trigger up to two same-session follow-ups and then fail the attempt. Sequential agent steps use the final response as handoff and imply `completed`. Existing saved runs keep their recorded handoffs and are not rewritten. Baton records step inputs, outputs, outcomes and handoffs in the run state.

Agent create and update requests validate direct runtime configuration. Credential references are IDs only; secret values are never accepted through pipeline or run APIs. Deleting an agent referenced by a pipeline returns 409 with the pipeline and step to update.

A project may add a commit agent after review and before human approval. That agent inspects and commits task changes in the task worktree; Baton does not create commits. Keep its instructions explicit about excluding session data and unrelated files, and have it call the configured `report_outcome` control tool after committing.
