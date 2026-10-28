# Project configuration

Baton stores project-owned configuration in `.baton/config.json`. The configuration is versioned, validated before writes and updated with a revision token to prevent stale overwrites.

## Agents and runtime selection

Agents contain `id`, `systemPrompt`, `runtimeKind` (`rig` or explicitly selected `fake`), optional `model` (`provider` and `model`), optional `credentialId`, `rules`, and `skillIds`. Provider/model and credential selection live directly on each agent; there is no runtime-profile layer. Credential IDs are references only. Secret values and OAuth records are held in the local credential vault, never in project configuration or run snapshots.

Pipelines have `id`, `name`, and a nonempty ordered `steps` array. Steps select an agent (or a human decision) and do not contain runtime, model, provider, or credential configuration.

## Migration

Schema version 7 is current. Schema-v6 `runtimeProfiles` are migrated into each referencing agent's `runtimeKind`, `model`, and `credentialId`, then removed. Shared profiles are copied to every referencing agent. The schema-v1 bootstrap marker `{"schemaVersion":1}` migrates to an empty v7 configuration. Agent-based v2/v3/v4 configurations migrate to direct agent runtime settings without copying credentials.

Schema-v5 Pi configurations remain unsupported because Pi runtime selections and authentication assumptions cannot safely be inferred as Rig settings.

Reading migrates in memory only; original bytes remain the revision token. An explicit successful mutation persists the current schema atomically. Missing/unsupported versions and unknown fields fail closed.
