# Pipelines

Create agents in the **Agents** UI before building a pipeline. With no agents, pipeline creation is disabled. Agent definitions remain in `.baton/config.json`; agents reference explicitly configured runtime profiles such as `rig.openai.gpt-4.1`. Runtime/provider/model selection never belongs on a step.

A pipeline is an ordered graph of agent and human-decision steps. Each transition has a bounded traversal count. Baton records step inputs, outputs, outcomes and handoffs in the run state.

Agent create and update requests reject unknown runtime profile IDs. Runtime configuration is project-owned; credentials are never accepted through pipeline or agent APIs. Deleting an agent referenced by a pipeline returns 409 with the pipeline and step to update.
