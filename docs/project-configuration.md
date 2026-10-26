# Project configuration

Baton stores project-owned configuration in `.baton/config.json`. The configuration is versioned, validated before writes and updated with a revision token to prevent stale overwrites.

## Runtime profiles

Reusable agents have `id`, `systemPrompt`, `runtimeProfileId`, `rules` (nonblank strings), and `skillIds`. Reusable `runtimeProfiles` have `id`, `name`, `runtimeKind` (`rig` or explicitly selected `fake`), and an optional provider/model selection. IDs identify the combination, for example `rig.openai.gpt-4.1`.

Profiles are configured in Settings, saved in project configuration, and assigned to agents. Authentication credentials never belong in configuration. `rig` is the stable embedded-runtime configuration contract; the adapter is not installed yet, so executing a Rig profile fails explicitly. `fake` is available only for deterministic offline tests.

Pipelines have `id`, `name`, and a nonempty ordered `steps` array. Runtime/provider/model selection belongs to agents through their profile, never to individual steps.

## Migration

Schema version 6 is current. The exact v1 bootstrap marker `{"schemaVersion":1}` migrates to an empty v6 configuration. Agent-based v2/v3/v4 configurations migrate old agents to a shared `rig.default` profile and discard retired direct runtime/provider/model/tool selections. The migration never copies credentials.

Pi-backed schema v5 configurations are intentionally unsupported. Replace each old profile with an explicit `rig.*` profile and update its agents before upgrading. This prevents Baton from treating a Pi model identifier or authentication assumption as valid Rig configuration.

Reading migrates in memory only; original bytes remain the revision token. An explicit successful mutation persists the current schema atomically. Missing/unsupported versions and unknown fields fail closed.
