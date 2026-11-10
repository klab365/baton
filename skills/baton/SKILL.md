---

name: baton
disable-model-invocation: true
description: Execute an issue or task through a configured Baton software-factory pipeline in the current Git worktree. Use when asked to process an issue with Baton, start or resume factory work, or follow the project's Baton workflow.
---

# Baton work

Use the current coding-agent session as orchestrator. Baton owns workflow state and routing; separate subagent sessions execute every agent step. The existing worktree stays externally owned.

## Prerequisites

Check `baton work --help`. The installed Baton binary must support the work protocol; this skill alone does not install the binary. If missing, explain that Baton must be installed/upgraded using the repository's installation instructions. Do not fall back to a pretend factory run.

The project must have a configured `.baton/config.json` pipeline. Use an explicitly requested pipeline; when the project has several and none was specified, ask which to use. Never silently change project pipelines, agent instructions, or task acceptance criteria to make a run pass.

## Start or resume

- Run `baton work status` to find existing work sessions. Resume a requested run by its ID; if ambiguous, ask. Do not silently create a second run for an already active task.
- Start a new run with `baton work start --issue '<user request>'`. Baton stores this value unchanged as text, including URLs or issue numbers. Add `--pipeline ID` when needed.
- Pass explicit project/user check commands as `--check 'STEP_ID=COMMAND'`; Baton executes these gates itself on successful submissions for that step. Successful gate results, including command, exit code and output, are recorded in the session; failed gates reject submission and are reported to the caller but are not recorded as accepted submissions. Agent-reported checks are not gate results. Do not invent commands or omit configured requirements. Commands execute with local user permissions.
- Record the returned run ID and always use `--run ID` afterward. If the request references an issue and its body is not already in this session, read it yourself with the available tools (for example `gh issue view <url>`). Do this during planning, before implementation, and retain the issue and its acceptance criteria in the session context. Baton does not fetch or resolve issues. Clarify an ambiguous issue number or inaccessible source with the user.

## Execute the pipeline

1. Fetch `baton work next --run ID`. Use the current step, agent instructions, assigned skills, allowed outcomes and configured checks. The active coding-agent session owns the conversation context: use the issue, acceptance criteria and user answers already present there. `next` does not replay task/history. Resolve supporting skill files under the returned `skillsDirectory/<skill-id>/`. If context is missing in a genuinely new session, explicitly inspect `baton work status --run ID` for the stored task and decisions, or ask the user when those records are insufficient. Do not load full status on every step.
2. If the state is `waitingHuman`, show the question/context and stop worker execution until the user responds. Never manufacture approval. `respond` is a separate explicit human action, not an agent submission.
3. If terminal (`succeeded`, `failed`, `cancelled`, `limitExhausted`), report the result and remaining work. Do not restart automatically or raise attempt limits to bypass a stop.
4. Perform only the current step's work. Follow project instructions. Derive task acceptance criteria from the supplied issue/prompt; clarify material ambiguity instead of silently adding or replacing requirements. Pipeline checks are separate process gates.
   - Delegate every agent step using the host's separate-session subagent facility. Provide the original task, prior step results and human decisions, current work item, and relevant acceptance criteria and decisions. Validate the returned outcome and summary against the current work item, including whether the handoff is self-contained, then submit the result unchanged. Stop rather than submit an incomplete handoff or a reference to missing details. Do not pass the full conversation by default. Only you submit to Baton; never delegate human decisions.
   - If no suitable subagent tool is available, stop and explain the missing capability. Never silently execute the step yourself. If a subagent fails or returns an invalid result, stop without submitting and report the failure. A separate context is not a sandbox; the subagent shares the worktree and its host permissions.
5. Write a JSON result file, then submit it:

   ```json
   {
     "revision": 0,
     "stepId": "build",
     "attempt": 1,
     "outcome": "completed",
     "summary": "What changed and how the issue's acceptance criteria were checked"
   }
   ```

   Use the exact revision, step ID and attempt returned by `next`, and an allowed outcome. Use `baton work submit --run ID --result /absolute/path/result.json`. Keep temporary result files out of commits and never include secrets.
6. When a subagent reports `progress`, submit it unchanged and delegate the next attempt of the same step; do not treat it as completion. The handoff must identify work done, verification, and the next action. Baton enforces the configured overall attempt limit.
7. Fetch the next step and continue until a terminal state, a human decision, or a blocker. If submission is stale, fetch state again and reconcile; do not resubmit old work with a fresh revision blindly. If a configured check fails, delegate the same current attempt again with the exact failure output and request a correction; do not make changes in the orchestrator. Submit the new result only after the delegated agent has addressed the failure. Do not blindly resubmit the unchanged result.

Agent summaries are reports, not independent verification. Baton records the checks it actually executes separately. A review performed in this same context is self-review; only claim independent review when a separate reviewer actually performed it. Use a host's separate reviewer capability only when available and authorized by the task/project workflow; otherwise surface an unmet independent-review requirement.

For a human response, present the current step and collect the user's decision. Submit `baton work respond --run ID --result FILE` with `revision`, `stepId`, `attempt`, `outcome`, and `response` reflecting that explicit decision. Stop on new ambiguity.

Follow the configured pipeline and the user's instructions. Delegate each agent step, submit its result, and continue automatically until Baton reports a terminal state or requires a human decision. Do not pause between configured steps. If a subagent fails or returns an invalid result, stop and report the failure without submitting.
