---

name: baton
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
- Pass explicit project/user check commands as `--check 'STEP_ID=COMMAND'`; Baton executes these gates itself on successful submissions for that step. Do not invent commands or omit configured requirements. Commands execute with local user permissions.
- Record the returned run ID and always use `--run ID` afterward. If the request references an issue and its body is not already in this session, read it yourself with the available tools (for example `gh issue view <url>`). Do this during planning, before implementation, and retain the issue and its acceptance criteria in the session context. Baton does not fetch or resolve issues. Clarify an ambiguous issue number or inaccessible source with the user.

## Execute the pipeline

1. Fetch `baton work next --run ID`. Use the current step, agent instructions, assigned skills, allowed outcomes and configured checks. The active Pi/Claude Code/Codex session owns the conversation context: use the issue, acceptance criteria and user answers already present there. `next` does not replay task/history. Resolve supporting skill files under the returned `skillsDirectory/<skill-id>/`. If context is missing in a genuinely new session, explicitly inspect `baton work status --run ID` for the stored task and decisions, or ask the user when those records are insufficient. Do not load full status on every step.
2. If the state is `waitingHuman`, show the question/context and stop worker execution until the user responds. Never manufacture approval. `respond` is a separate explicit human action, not an agent submission.
3. If terminal (`succeeded`, `failed`, `cancelled`, `limitExhausted`), report the result and remaining work. Do not restart automatically or raise attempt limits to bypass a stop.
4. Perform only the current step's work. Follow project instructions. Derive task acceptance criteria from the supplied issue/prompt; clarify material ambiguity instead of silently adding or replacing requirements. Pipeline checks are separate process gates.
   - Delegate every agent step. In Pi, call the bundled `baton_subagent` tool with the run ID and relevant acceptance criteria and decisions as `context`. It fetches the original task, prior step results and human decisions, and the current work item, starts a separate Pi process, and returns a validated submission. Save that result as JSON and submit it unchanged. Do not pass the full conversation by default. Only you submit to Baton; never delegate human decisions. In other hosts, delegate using their separate-session subagent facility with equivalent context and result validation.
   - If no suitable subagent tool is available, stop and explain the missing capability. Never silently execute the step yourself. If a subagent fails or returns an invalid result, stop without submitting and report the failure. A separate context is not a sandbox; the subagent shares the worktree and its host permissions.
5. Write a JSON result file, then submit it:

   ```json
   {
     "revision": 0,
     "stepId": "build",
     "attempt": 1,
     "outcome": "completed",
     "summary": "What changed and how the issue's acceptance criteria were checked",
     "evidence": ["Relevant files, actual test commands/results, or artifact references"]
   }
   ```

   Use the exact revision, step ID and attempt returned by `next`, and an allowed outcome. Use `baton work submit --run ID --result /absolute/path/result.json`. Keep temporary result files out of commits and never include secrets.
6. Fetch the next step and continue until a terminal state, a human decision, or a blocker. If submission is stale, fetch state again and reconcile; do not resubmit old work with a fresh revision blindly. If a configured check fails, fix the problem and resubmit the same current attempt when appropriate.

Evidence references are agent reports, not independent verification. Baton records the checks it actually executes separately. A review performed in this same context is self-review; only claim independent review when a separate reviewer actually performed it. Use a host's separate reviewer capability only when available and authorized by the task/project workflow; otherwise surface an unmet independent-review requirement.

For a human response, present the current step and collect the user's decision. Submit `baton work respond --run ID --result FILE` with `revision`, `stepId`, `attempt`, `outcome`, and `response` reflecting that explicit decision. Stop on new ambiguity.

The skill does not grant permission to comment on issues, commit, push, merge, or delete worktrees. Follow the user's authorization for those actions.
