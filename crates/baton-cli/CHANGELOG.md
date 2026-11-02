# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/klab365/baton/releases/tag/v0.1.0) - 2026-11-02

### Added

- bootstrap Baton projects from CLI
- manage project skills from agent UI
- add local credential vault
- run configured pipelines in persistent task worktrees
- add bounded workflow transitions and human waits
- isolate task runs in Git worktrees
- extend control plane run management
- add ChatGPT device-code runtime
- add Rig OpenAI runtime adapter
- implement durable pipeline orchestration spike
- initialize Baton Rust and SvelteKit workspace

### Fixed

- remove task acceptance criteria field
- default ChatGPT token storage path

### Other

- prepare embedded rig runtime profiles
- flatten adapter crate modules
- rename runtime integrations as adapters
- isolate runtime integration adapters from core
