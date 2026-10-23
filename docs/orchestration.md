# Sequential orchestration spike

Run the demo with a deterministic fake runtime (no model or command is executed):

```sh
cargo run -p baton-cli -- run examples/demo draft "Write a release summary"
```

The command takes a pipeline **ID**, not its display name, prints the durable run,
and exits unsuccessfully on failure. The fake adapter returns `agentId: input`.
`AgentRuntime` in `baton-core::orchestration` is the replaceable Rust port; model,
rules, tools and skills are available through the reusable agent configuration.
The fake does not interpret them. There is no production runtime integration.

The control-plane use case validates the configuration and rejects Docker/remote
steps before any execution. Sequential local steps receive the task for the first
input and exactly the preceding output thereafter. Inputs and outputs are limited
to 16 KiB of UTF-8 bytes; oversized tasks are rejected before creating a run and
oversized outputs fail the step without storing the output. Runtime errors are
bounded too. A failed step fails the run and leaves later steps pending.

Run files live under `.baton/runs/<id>.json`, outside project configuration and
Git. Each run records `startedAt` and, after success or failure, `endedAt` as UTC
RFC 3339 timestamps plus `durationMs` measured with a monotonic clock. Interrupted
runs retain their start time and have no end time or duration. The repository creates without overwrite, syncs each snapshot, atomically
publishes it, and syncs the directory. The runner persists before runtime calls
and after every result. Persistence errors stop execution immediately. A crash
leaves the last committed snapshot readable after restart; running records mean
interrupted or in-flight, not success. No automatic recovery, replay or retry is
attempted, since runtime side effects cannot be assumed idempotent. A stale
`<id>.tmp` after a crash must only be removed after confirming its writer is dead.
Each run has a single owner; concurrent execution of the same run is unsupported.

There is no streaming, tool execution, approval, branching, parallelism, CRUD UI,
or standalone execution-target resource in this spike.
