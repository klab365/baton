---
name: rust
description: Use when implementing or reviewing Rust code.
---

# Rust

- Follow the project's `AGENTS.md`, crate conventions and existing architecture.
- Prefer clear ownership and borrowing; avoid unnecessary cloning and unsafe code.
- Handle recoverable errors with `Result` and useful context; avoid unchecked panics.
- Keep async work non-blocking and avoid holding locks across `.await` unless necessary.
- Cover changed behavior and failure paths with focused tests; run the narrowest relevant checks.
