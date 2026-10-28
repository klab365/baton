# Directed workflows and human decisions

Pipelines remain sequential by default. Schema-v3 steps may additionally specify `human` (default `false`) and `transitions` (default `[]`). The pipeline editor supports both. The first listed step is the entry point. Only one step executes at a time; there are no parallel branches or free-form orchestrator agents.

```json
{
	"id": "review",
	"name": "Review",
	"agentId": "reviewer",
	"transitions": [
		{ "outcome": "approved", "to": "human", "maxTraversals": 1 },
		{ "outcome": "revise_build", "to": "build", "maxTraversals": 3 },
		{ "outcome": "revise_plan", "to": "plan", "maxTraversals": 2 }
	]
}
```

Each transition is an exact outcome predicate. `to` names an existing step; `null` explicitly ends the run. Outcomes are `completed`, `approved`, `revise_build`, `revise_plan`, `blocked`, `failed`, `rejected`, and `responded`. Duplicate predicates, unknown destinations/outcomes, and limits outside 1–1000 are rejected. All explicit edges are bounded, not just backward edges: this also bounds cycles containing implicit sequential edges. Counts are cumulative for the entire run, never reset on re-entry.

For agent steps with transitions, Rig provides the `report_outcome` control tool. Its schema and description expose only the outcomes configured on that step. After finishing its work, the agent calls it with an allowed outcome and a concise summary; the agent's natural-language final response remains separate. The Control Plane validates the reported outcome and chooses the configured edge. An agent cannot supply a destination or add response fields. A missing or disallowed outcome fails the attempt without advancing. Plain sequential agent steps require no control-tool call and produce `completed`. The tool summary becomes the next input handoff. Summaries, handoffs, and runtime inputs are bounded to 16384 UTF-8 bytes.

A configured terminal edge succeeds except for `failed`, `blocked`, or `rejected`, which terminate as failed. Those outcomes can instead route to a configured recovery or human step. An exhausted edge leaves `limitExhausted`, with the selected transition, existing count, and an actionable error. Inspect the history, correct the task/configuration, and explicitly start a new run; there is no limit override or automatic retry.

## Human waits

```json
{
	"id": "human",
	"name": "Human PR decision",
	"human": true,
	"transitions": [
		{ "outcome": "approved", "to": null, "maxTraversals": 1 },
		{ "outcome": "rejected", "to": "build", "maxTraversals": 2 },
		{ "outcome": "responded", "to": "review", "maxTraversals": 2 }
	]
}
```

Human steps have no agent (`agentId` omitted or empty). Their predicates must be `approved`, `rejected`, or `responded`. An empty transition list uses sequential continuation on approval/response and terminal failure on rejection. Reaching the step durably writes `waitingHuman`, its input, attempt number, current step, and `pendingActions`. No agent is invoked. The wait has **no automatic timeout or approval**; explicitly cancel to abort. Cancellation is also the cooperative abort mechanism for in-flight agent work. There is no automatic merge or destructive action attached to approval.

Each run owns a frozen configuration snapshot and references its task's retained Git worktree. Startup recovery fails interrupted `running` runs without retrying, but leaves human waits unchanged. A response explicitly resumes using the saved graph, counts, and same worktree; ownership and branch identity are checked before resumption. Repeated or conflicting actions are rejected. Provider availability is checked before an action is accepted. A human response (including an empty response) becomes the next handoff.

## Run API and UI

- `GET /api/runs` and `GET /api/runs/{id}` expose `configSnapshot` (execution graph), `currentStep`, `pendingActions`, `transitionCounts`, `steps`, and events. Each executed `steps` entry has a global `attempt` number, input, output, outcome, selected transition, traversal count, status, and error. Pending entries have attempt 0; sort executed attempts by attempt number, not vector position. Re-entry creates a new attempt instead of overwriting completed work.
- `POST /api/runs/{id}/actions` accepts `{ "outcome": "approved", "response": "Reviewed PR #123" }`. `response` defaults to empty. Only a waiting run and a configured human action are accepted. Invalid actions do not mutate the wait. The endpoint executes subsequent steps until the next wait or terminal state.
- `POST /api/runs/{id}/cancel` explicitly aborts running or waiting runs and retains their worktree. A cancelled run cannot be resumed; its task worktree remains until the task is done or deleted.

The run detail UI shows the graph, current step, traversal counters, ordered attempts/handoffs, events, and pending human action buttons. Task modification, deletion, duplicate execution, and CLI worktree cleanup are blocked while a run is waiting. Legacy task files may contain an ignored `configSnapshot`; legacy sequential runs remain readable.
