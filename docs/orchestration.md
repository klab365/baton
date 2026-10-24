# Sequential orchestration and runtimes

Baton runs a configured pipeline through the `AgentRuntime` port in
`baton-core::orchestration`. Agents remain reusable configuration: a step names
an `agentId`; its model, prompt, rules, tools, and skills are never duplicated
on the step. There are no execution-target or data-plane resources.

The port, run types, and durable orchestration live in `baton-core` without
Rig dependencies. Outbound implementations live in `baton-adapters`:
`adapters/mod.rs` exports the separate `fake.rs`, `openai.rs`, and `chatgpt.rs`
adapters. Private `rig_support.rs` shares request helpers. The CLI selects and
wires the concrete adapter; domain code never selects a provider.

## Explicit runtime selection

The CLI always requires a runtime; there is no fallback between fake, OpenAI
API, or ChatGPT-subscription execution:

```sh
cargo run -p baton-cli -- run --runtime fake examples/demo draft "Write a release summary"
```

`fake` requires every referenced agent to have `model.provider: "fake"`. It is
deterministic (`agentId: input`), does not execute a command or contact a
model, and is the default for offline tests. `openai` and `chatgpt` likewise
require the matching `model.provider`. A mismatch fails before a run record is
created, so choosing fake cannot silently replace a real model.

A step invokes its referenced agent through the selected runtime adapter. The
runtime receives only the run's allocated worktree as its project working
directory; there is no per-step directory override. Current Rig adapters are
text-only and do not access project files or change the process directory.
Future filesystem/process adapters must use the supplied directory, never the
server's current directory.

## Isolated Git worktrees

Both CLI and Control Plane execution allocate through the `WorktreeManager`
port before invoking any runtime. The Git adapter uses the configured project
directory (`BATON_PROJECT_DIR` for the server, or the CLI project argument) and
`BATON_BASE_REVISION` (default `HEAD`), resolved to a commit before allocation.
Only committed content is included; uncommitted primary-checkout changes are
not copied. Projects must belong to a Git repository with a resolvable commit.
There is no non-Git or primary-checkout fallback.

For run `<id>`, the deterministic location is
`<absolute Git common directory>/baton-worktrees/<id>` and the new branch is
`baton/run-<id>`. This location is separate from both the primary checkout and
Baton's source checkout, including when Baton itself is the configured project.
All Git invocations use argument arrays, not shell commands. Worktrees are Git
isolation, **not a security sandbox**: agents with filesystem capabilities must
still obey their runtime policy.

Exclusive reservations reject duplicate owners; existing paths or branches
are never reused, reset, or force-removed. Concurrent runs with distinct IDs
get distinct directories and branches. Setup failure produces a failed durable
run with `setupError` and an event, and invokes no agent. A failed reservation
is retained to prevent accidental reuse; inspect the reported path and retry
with a new run ID rather than deleting unknown directories.

Run snapshots persist `worktree` ownership, path, branch, resolved base commit,
lifecycle, and cleanup outcome. Completing a run does not close its task:
worktrees are retained after successful, failed, cancelled, and interrupted runs.
Marking a task `done` or deleting it explicitly attempts to clean up all of its
finished run worktrees; the status change/deletion is refused if a workspace
cannot be safely removed. These lifecycle rules are core use cases, so the CLI
and server behave consistently:

```sh
cargo run -p baton-cli -- task-done <project-directory> <task-id>
cargo run -p baton-cli -- task-delete <project-directory> <task-id>
```

After stopping the server/executor (cancellation is cooperative), explicitly
remove a retained finished run's clean worktree with:

```sh
cargo run -p baton-cli -- cleanup <project-directory> <run-id>
```

Cleanup verifies the allocation receipt, run owner, expected path and branch;
then Git verifies that the path is a registered, clean worktree. Dirty worktrees
are refused without `--force`; inspect and commit or move valuable files first.
The branch and reservation remain for inspection, and the cleanup result is
persisted on the run even when removal is refused. Never manually repurpose a
run's worktree or ownership files for another run. Legacy runs with no worktree
metadata cannot be cleaned through this command. No Docker or remote target is
implemented; the domain port is independent of Git command execution.

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

## ChatGPT subscription device-code login

`chatgpt` is a separate, explicit Rig runtime: it uses Rig's ChatGPT/Codex
OAuth device-code flow, **not** `OPENAI_API_KEY` and not API billing. By default
tokens are stored under `$HOME/.local/share/baton/chatgpt`, outside the project.
Set `BATON_CHATGPT_TOKEN_DIR` only if you want to override that private local
location, then run the opt-in example:

```sh
cargo run -p baton-cli -- run --runtime chatgpt examples/chatgpt-smoke release-note \
  "Version 1.2 adds CSV export and fixes the empty dashboard."
```

On the first request Baton prints the verification URL and code to stderr.
Open that URL, complete the sign-in with the account that has your ChatGPT
subscription, and wait for the command to continue. Rig stores refresh tokens
in `<token-dir>/chatgpt.json`; on Unix Rig creates that directory with mode
0700 and its token files with mode 0600. Never commit, copy into the
project, or share this directory. Subsequent runs refresh the local token when
needed. `gpt-5.3-codex` is the example model; availability and subscription
limits are determined by ChatGPT, not Baton. This is an experimental
subscription-backend integration and has no API-key fallback.

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
