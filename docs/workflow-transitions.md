# Directed workflows and human decisions

Pipelines remain sequential by default. Each step specifies `type` (`agent`, `human`, or `shell`) and optional `transitions` (default `[]`). The first listed step is the entry point. Only one step executes at a time; there are no parallel branches or free-form orchestrator agents.

```json
{
	"id": "review",
	"name": "Review",
	"type": "agent",
	"agentId": "reviewer",
	"transitions": [
		{ "outcome": "approved", "to": "human" },
		{ "outcome": "revise_build", "to": "build" },
		{ "outcome": "revise_plan", "to": "plan" }
	]
}
```

Each transition is an exact outcome predicate. `to` names an existing step; `null` explicitly ends the run. Outcomes are `completed`, `approved`, `revise_build`, `revise_plan`, `blocked`, `failed`, `rejected`, and `responded`. `progress` is retained only for reading historical runs and cannot be configured as a transition. Duplicate predicates and unknown destinations/outcomes are rejected. Every new run uses the pipeline's global `maxAttempts` (default 50, configurable from 1 to 1000); started agent, shell, and human visits count, including failures and interruptions. An optional step `maxAttempts` independently caps visits to that step. Legacy managed snapshots without a global value remain unlimited.

For agent steps with transitions, Rig and Pi provide the `report_outcome` control tool. The tool exposes the outcomes configured on that step and rejects others. After finishing its work, the agent calls it with an allowed outcome and the complete authoritative handoff in its summary. A `blocked` outcome requires a nonblank `blocker_reason`; `human_action` is optional and, when supplied, is preserved in the handoff. Blocked means the work needs assistance, not necessarily a person: configured transitions alone decide whether it routes to an agent or human step. Existing clients sending an empty `human_action` remain compatible. The Control Plane validates the outcome and chooses the configured edge. Branching agent reports may additionally include `structured_handoff` with `version: 1`, a nonblank `summary`, optional `findings` (`id`, `status`: `open`/`resolved`/`accepted`, and nonblank `description`), optional `verification` entries (`description` and `result`), and optional `nextAction`. IDs must be unique within the report; at most 128 findings and 16384 serialized JSON bytes are accepted. Unknown properties and unsupported versions are rejected. The structured report is stored on its producing step attempt alongside the legacy text handoff and exposed in run inspection. Downstream context merges reports in attempt order across agent and human steps and after reload: later reports update findings with matching IDs, while omitted IDs retain their previous status and description. A finding is resolved only when a later report explicitly gives that ID status `resolved` (or `accepted`). The merged report uses the latest summary and next action; nonempty verification evidence accumulates with its producing attempt number, so omission never clears it. If merged context exceeds 128 findings or 16384 JSON bytes, the prompt includes findings/evidence prioritized by most recent update that fit and an explicit notice/count; all original reports remain persisted on their attempts, and the notice names the attempts to inspect. Managed Rig and Pi agents can retrieve any omitted report with `get_structured_handoff` and its producing attempt number, then report an update using the same stable finding ID. Omission from the bounded prompt is not resolution or verification. For example, if one report gives `F-1` status `open` and the next report omits `F-1`, it remains open; a later report with `F-1` status `resolved` updates it. Agent-provided verification is an assertion, not independent verification; downstream prompts label it unverified. Plain sequential free-text handoffs remain unchanged. A sequential agent can opt into structured submission by allowing the `report_outcome` tool; it may call the tool with outcome `completed` and a `structured_handoff`, while its final response remains the legacy text handoff. The call is optional, and no transition configuration is needed. If the agent reports no valid outcome, Baton asks up to twice in the same conversation; exhaustion fails the attempt without advancing. An empty agent handoff also fails the attempt. Plain sequential agent steps require no control-tool call, use the final response as handoff and produce implicit `completed`. The task, each handoff and outcome summary, and each human response are limited to 16384 UTF-8 bytes; oversized results fail rather than being silently truncated. Shell steps also fail if either stdout or stderr exceeds the 8192-byte capture limit.

## Managed-runtime step context

Each step receives the immediately preceding handoff and nonempty explicit human responses from successful human steps, labeled with step, attempt and outcome in chronological order. Later explicit corrections supersede earlier decisions where applicable; agent summaries are not treated as human decisions. These responses are recovered from persisted attempts, including after restart. Each response is limited to 16384 UTF-8 bytes; there is no separate aggregate context limit, so long or looping runs can exceed a model's context window. Do not enter secrets in tasks or human responses: run state and agent context are persisted without automatic secret redaction. The original task accompanies these human responses even when `includeTask` is false. Otherwise agent steps may set `includeTask: true` to include it; the default is `false`, including for existing configurations and snapshots. The original task remains in the run record. Interrupted-step recovery adds reconciliation instructions without replacing the prior handoff. A reused agent session can still contain earlier conversation regardless of `includeTask`.

A configured terminal edge succeeds except for `failed`, `blocked`, or `rejected`, which terminate as failed. Those outcomes can instead route to a configured recovery or human step.

## Human waits

```json
{
	"id": "human",
	"name": "Human PR decision",
	"type": "human",
	"transitions": [
		{ "outcome": "approved", "to": null },
		{ "outcome": "rejected", "to": "build" },
		{ "outcome": "responded", "to": "review" }
	]
}
```

Human steps do not accept `agentId`. Their predicates must be `approved`, `rejected`, or `responded`. An empty transition list uses sequential continuation on approval/response and terminal failure on rejection. Reaching the step durably writes `waitingHuman`, its input, attempt number, current step, and `pendingActions`. No agent is invoked. The wait has **no automatic timeout or approval**; explicitly cancel to abort. Cancellation is also the cooperative abort mechanism for in-flight agent work. There is no automatic merge or destructive action attached to approval.

Managed and external/worktree runs report a warning event when the same step reports an identical blocker after a human response. Comparison ignores case and whitespace; different blockers or repetitions without an intervening human response do not produce this warning. The history and warning are retained with the run, and the configured transition still applies. This narrow warning is not general loop detection or a pause policy; those are tracked in [#231](https://github.com/klab365/baton/issues/231).

Each run owns a frozen configuration snapshot and references its task's retained Git worktree. Startup recovery fails interrupted `running` runs without retrying, but leaves human waits unchanged. A response explicitly resumes using the saved graph and same worktree; ownership and branch identity are checked before resumption. Repeated or conflicting actions are rejected. Provider availability is checked before an action is accepted. A human response (including an empty response) becomes the next handoff. If the human step was reached with a prior handoff, managed human resumption forwards both values, labeled `Previous handoff` and `Human response`; an empty response preserves the prior handoff unchanged. The combined value is bounded to 16384 UTF-8 bytes: when needed, the oldest prior context is truncated at a character boundary with a marker, while preserving the response. If the response plus fixed labels and truncation marker cannot fit, the response is forwarded alone (this can happen below the full limit); if the response itself uses the entire limit, it is likewise forwarded without labels or prior context. Responses over 16384 bytes are rejected without consuming the wait. Use separate human steps when different blocker classes need different continuations; route each incoming `blocked` edge to its dedicated human step and configure that step's `responded` destination. The project software-factory pipeline routes planning blockers to `plan-help`, implementation blockers to `build-help`, review blockers to `review-help`, and commit blockers to `commit-help`; each response resumes its originating step and rejection is terminal. Review handoffs identify the reviewed diff/base and head SHA; the committer must verify staged changes still match before publishing.

The repository's Pi/Work software-factory pipeline no longer includes an automatic CI agent; it ends after approved publication. Use `/baton refine [RUN_ID]` in Pi to start a follow-up run in the same worktree with an additional instruction. The new run reuses the completed run's per-agent conversation identity, so the selected runtime (Pi or Rig) can continue its saved conversation. You can explicitly ask for CI inspection or follow-up fixes in the refinement request; see [session work](session-work.md).

## Run API and UI

- `GET /api/runs` and `GET /api/runs/{id}` expose `configSnapshot` (execution graph), `currentStep`, `pendingActions`, `steps`, and events. Each executed `steps` entry has a global `attempt` number, input, output, outcome, selected transition, status, and error. Pending entries have attempt 0; sort executed attempts by attempt number, not vector position. Re-entry creates a new attempt instead of overwriting completed work.
- `POST /api/runs/{id}/actions` accepts `{ "outcome": "approved", "response": "Reviewed PR #123" }`. `response` defaults to empty. Only a waiting run and a configured human action are accepted. Invalid actions do not mutate the wait. The endpoint executes subsequent steps until the next wait or terminal state.
- `POST /api/runs/{id}/cancel` explicitly aborts running or waiting runs and retains their worktree. A cancelled run cannot be resumed; its task worktree remains until the task is done or deleted.

The run detail UI shows the graph, current step, ordered attempts/handoffs, events, and pending human action buttons. Task modification, deletion, duplicate execution, and CLI worktree cleanup are blocked while a run is waiting. Older run snapshots with implicit step types are converted when read.
