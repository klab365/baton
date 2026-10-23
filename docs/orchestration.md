# Sequential orchestration and runtimes

Baton runs a configured pipeline through the `AgentRuntime` port in
`baton-core::orchestration`. Agents remain reusable configuration: a step names
an `agentId`; its model, prompt, rules, tools, and skills are never duplicated
on the step. There are no execution-target or data-plane resources.

## Explicit runtime selection

The CLI always requires a runtime; there is no fallback between real and fake
execution:

```sh
cargo run -p baton-cli -- run --runtime fake examples/demo draft "Write a release summary"
```

`fake` requires every referenced agent to have `model.provider: "fake"`. It is
deterministic (`agentId: input`), does not execute a command or contact a
model, and is the default for offline tests. `openai` likewise requires every
referenced agent to declare `model.provider: "openai"`. A mismatch fails before
a run record is created, so choosing fake cannot silently replace a real model.

Only `execution.mode: "local"` is accepted. The local `command` remains part
of the step configuration required by the existing schema, but the Rig adapter
does not execute it: Rig makes the provider request. Docker and remote steps
are rejected before execution.

## Opt-in OpenAI smoke test

The Rig 0.45 OpenAI adapter is a text-completion implementation of the existing
port. It reads **only** `OPENAI_API_KEY` from the process environment (Rig also
recognizes `OPENAI_BASE_URL`, which is useful only for a deliberately chosen
OpenAI-compatible local endpoint). Keep these local shell variables or an
ignored `.env.local`; never put them in `.baton/config.json`, run records, or
logs.

An OpenAI API key with active API billing is required. A ChatGPT subscription,
ChatGPT/Codex login, or Codex CLI credentials do **not** imply OpenAI API access
or billing. Create and export a separate API key in your own local environment:

```sh
export OPENAI_API_KEY='...'
# Do not paste this value into files, tickets, or terminal recordings.
cargo run -p baton-cli -- run --runtime openai examples/openai-smoke release-note \
  "Version 1.2 adds CSV export and fixes the empty dashboard."
```

`examples/openai-smoke/.baton/config.json` is a two-step configured pipeline.
It uses `gpt-4.1-mini`, a supported OpenAI text model selected for a small smoke
test. The adapter forwards any configured model ID to OpenAI's completion API;
use only IDs enabled for your API project and compatible with that endpoint.
The first agent receives the task, and the reviewer receives the writer's exact
output. The command prints the final durable run; snapshots are stored at
`examples/openai-smoke/.baton/runs/<run-id>.json` and are ignored by Git.

This is intentionally opt-in: normal tests use no credentials and make no
network requests. The adapter has a fixed 60-second request timeout and no
implicit retry or fake fallback. Missing credentials, authentication failures,
unavailable models, rate limits, network failures, and timeouts fail the active
step and run; subsequent steps remain pending. Provider error text is not
persisted, avoiding accidental secret disclosure.

## Request mapping and current capabilities

For each OpenAI request, Baton sends the reusable agent's `systemPrompt` as the
Rig preamble, appends every configured rule as a bullet under `Rules`, sends the
step input as the user prompt, selects `model.model`, and maps
`maxOutputTokens` to Rig's maximum-token setting. The response text becomes the
bounded handoff to the next step.

The evaluated Rig version natively offers structured-output schemas, tool
request/response types, streaming, and async cancellation primitives. This
adapter deliberately uses none of those advanced features:

- **Structured output:** Rig supports schemas, but Baton has no output schema
  configuration or validation contract yet.
- **Tools/MCP/approvals:** Rig can describe and drive tools, but Baton has no
  tool implementation, approval policy, or MCP lifecycle. Any agent with
  `toolIds` fails instead of silently ignoring tools.
- **Skills:** Baton skill instructions have no Rig mapping yet. Any agent with
  `skillIds` fails explicitly; they are not injected as prompt text.
- **Streaming/cancellation/pause-resume:** Rig can stream and Rust tasks can be
  cancelled, but the synchronous `AgentRuntime` port persists only completed
  text. Baton exposes no cancellation, pause, or resume endpoint. Interrupting
  a process leaves its already-persisted running snapshot; there is no automatic
  resume.
- **Context, retries, and durable state:** each step is a fresh request with
  only its bounded preceding handoff. Baton owns durable run snapshots and
  failure short-circuiting. There is no conversation memory, retry policy,
  provider state persistence, or replay because model side effects are not
  assumed idempotent.

The minimal text pipeline is suitable to ship behind explicit `openai`
selection: it preserves ordering, bounded handoff, and durable records without
pretending to support tools or resumable execution. Advanced Rig facilities
should be added only with corresponding Baton contracts and persistence rules.

## Durability and failure semantics

The control-plane use case validates configuration and rejects unsupported modes
before any execution. Sequential local steps receive the task for the first
input and exactly the preceding output thereafter. Inputs and outputs are
limited to 16 KiB of UTF-8 bytes; oversized tasks are rejected before creating a
run and oversized outputs fail the step without storing the output. Runtime
errors are bounded too. A failed step fails the run and leaves later steps
pending.

Run files live under `.baton/runs/<id>.json`, outside project configuration and
Git. Each run records `startedAt` and, after success or failure, `endedAt` as UTC
RFC 3339 timestamps plus `durationMs` measured with a monotonic clock. The
runner persists before runtime calls and after every result. Persistence errors
stop execution immediately. Interrupted runs retain their start time and have
no end time or duration. No automatic recovery, replay, or retry is attempted.
