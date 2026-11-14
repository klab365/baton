# Getting started: build a Software Factory

This guide builds a small pipeline with planning, implementation, independent review, human approval and a commit. The complete example, including system prompts and transitions, is in [`examples/demo/.baton/`](../examples/demo/.baton/); its skills are in [`examples/demo/.agents/skills/`](../examples/demo/.agents/skills/).

The example is a reference, not a portable configuration: its provider, model and credential IDs are local examples. Do not copy its credential IDs into your project. Choose models and credentials available in your own environment.

## 1. Start a project

Install Baton as described in the [README](../Readme.md). Choose a **trusted Git repository** and run:

```sh
baton /path/to/project
```

On first launch, accept the terminal initialization prompt, or open <http://127.0.0.1:5173> and choose **Set up project**. This initializes an empty configuration; Baton does not automatically create the example pipeline or download skills. Set `BATON_PORT` if needed.

In **Settings**, add credentials for the providers you intend to use. Secrets stay in the local credential vault. Create any project-specific instructions in `AGENTS.md` at the project root: Rig loads this file from the task worktree at each step. See [runtime orchestration](orchestration.md) for limits and trust considerations.

## 2. Import skills

In **Skills → Import skill**, paste each GitHub folder URL and import it. These URLs pin the reviewed source to a specific commit:
- [`concise-planning`](https://github.com/sickn33/agentic-awesome-skills/tree/b4bd5a153dd1e0a66789b6c971f731cdcf492a46/skills/concise-planning)
- [`code-review-excellence`](https://github.com/sickn33/agentic-awesome-skills/tree/b4bd5a153dd1e0a66789b6c971f731cdcf492a46/skills/code-review-excellence)
- [`verification-before-completion`](https://github.com/obra/superpowers/tree/2ce8aa15d05bd57213e6f746f953aafb7ded7a8b/skills/verification-before-completion)

The example also assigns `ponytail`, already included at [`examples/demo/.agents/skills/ponytail/SKILL.md`](../examples/demo/.agents/skills/ponytail/SKILL.md). You may copy that skill directory into your project's `.agents/skills/` or omit it from the builder.

Review third-party instructions before using them. Skills are instructions, not permissions or a sandbox. Importing a folder also downloads supporting files, but only the `SKILL.md` body is automatically added to the agent context. See [project skills](skills.md).

## 3. Create agents

In **Agents → Create agent**, create these agents. For each one, select Rig, a compatible credential and a model available to you. Copy the corresponding agent text from the [example agents](../examples/demo/.baton/agents/), adapting it to your project. Instructions define reusable agent behavior; each agent pipeline step can add its own task-specific prompt. Assign the skills below:

| Agent ID            | Responsibility                                     | Skills                                                     |
| ------------------- | -------------------------------------------------- | ---------------------------------------------------------- |
| `factory-planner`   | Inspect the task and provide a focused plan        | `concise-planning`                                         |
| `factory-builder`   | Implement changes and run relevant checks          | `verification-before-completion`; optionally `ponytail`    |
| `factory-reviewer`  | Independently review the worktree changes          | `code-review-excellence`, `verification-before-completion` |
| `factory-committer` | Create a scoped Conventional Commit after approval | `verification-before-completion`                           |

The builder discovers project checks and reports the commands and results. This is agent-driven, not a deterministic test gate; a reviewer should verify important claims.

## 4. Build the pipeline

In **Pipelines → Create pipeline**, add the steps in this order, then configure their transitions. The first step is the entry point; `End` below means a transition with no destination.

| Step ID    | Agent or human      | Transitions                                                                                   |
| ---------- | ------------------- | --------------------------------------------------------------------------------------------- |
| `plan`     | `factory-planner`   | None: continues sequentially to `build`                                                       |
| `build`    | `factory-builder`   | `completed` → `review`; `blocked` → `help`                                                    |
| `review`   | `factory-reviewer`  | `approved` → `approval`; `revise_build` → `build`; `revise_plan` → `plan`; `blocked` → `help` |
| `approval` | Human               | `approved` → `commit`; `rejected` or `responded` → `build`                                    |
| `commit`   | `factory-committer` | `completed` → End; `blocked` → `help`                                                         |
| `help`     | Human               | `responded` or `approved` → `plan`; `rejected` → End                                          |

Compare your configuration with the `software-factory` pipeline in the [example](../examples/demo/.baton/pipelines/software-factory.yml). Agent steps with transitions must call `report_outcome`; the example prompts instruct them to do so. A blocked planner reports its uncertainty in its handoff and routes to human help.

**Review loops have no automatic retry or cost limit.** Monitor the run and cancel it if agents repeat without progress. Human approval does not merge or push the branch. A rejected final human transition ends the run as failed. See [workflow transitions](workflow-transitions.md).

## 5. Run a task

In **Tasks**, create a task with a clear description and acceptance criteria, select the pipeline, save it and start a run. Inspect step outputs and test results under **Runs**. When the run waits for a human, review the changes and approve, reject with feedback, or respond as appropriate. The commit agent runs only after approval.

Each task has its own persistent Git worktree. A worktree is **not a sandbox**: shell commands can affect resources outside it. Use a container, VM or dedicated OS user for stronger isolation. Task completion or deletion removes its worktree and discards any uncommitted changes; review and preserve anything needed first. See [tasks](tasks.md).
