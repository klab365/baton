---
name: baton
disable-model-invocation: true
description: Execute an issue or task through a configured Baton pipeline in the current Git worktree. Use when asked to start or resume Baton work.
---

# Baton work

The coding-agent TUI is the interface; Baton executes the configured agents via its runtime. Baton owns workflow state, routing, limits and checks. The existing worktree remains externally owned.

## Start or resume

- Check `baton work --help` and ensure the project has a configured pipeline and agent credentials. If several pipelines exist and none was requested, ask which to use.
- Run `baton work status` to find existing sessions; do not silently start a duplicate run. Resume the requested run by ID.
- Before starting, resolve an issue URL or number using available tools, and include its actual requirements and acceptance criteria in the `--issue` text. Baton does not fetch issues. Clarify inaccessible or ambiguous requirements with the user.
- Start with `baton work start --issue '<full task and acceptance criteria>'`, adding `--pipeline ID` if necessary. Pass explicit checks as `--check 'STEP=COMMAND'`; do not invent requirements.

## Execute the pipeline

1. Run `baton work next --run ID`. If it is `waitingHuman`, show the context and ask the user for an explicit decision. Never manufacture approval.
2. If terminal (`succeeded`, `failed`, `cancelled`, `limitExhausted`), report the result and remaining work. Do not automatically restart or raise limits.
3. For an active agent or command step, run `baton work execute --run ID`. Baton runs the configured runtime in the existing workspace, validates the outcome and runs applicable checks. Do not execute the step yourself or submit a fabricated result.
4. Repeat from `next` until terminal, a human decision, or an error. If execution or a check fails, report the error; do not blindly retry. For `progress`, continue the configured pipeline without treating it as completion.
5. For a human decision, write a JSON file with the exact `revision`, `stepId`, `attempt`, `outcome`, and `response` from the user's decision, then run `baton work respond --run ID --result FILE`. Never delegate human decisions.

Use `baton work status --run ID` for persisted history and `baton work cancel --run ID` to cancel. Agent summaries are reports, not independent verification; only claim checks Baton actually ran. Commands and tools run with local user permissions and are not a sandbox. Never include secrets in task text or result files.
