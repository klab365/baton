# Provider credentials

Baton project configuration contains provider and model identifiers only. It never stores, reads, extracts or logs provider credentials.

The embedded Rig runtime is not integrated yet; its supported credential sources will be documented with that adapter. Until then, a `rig.*` profile is configuration-only and execution fails explicitly rather than using an implicit provider or credential fallback.
