# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.11.0](https://github.com/klab365/baton/compare/v0.10.0...v0.11.0) - 2026-11-14

### Added

- persist authoritative pipeline handoffs
- reuse agent sessions across pipeline steps
- pass handoffs along pipeline transitions
- add pipeline listing and selection
- simplify agent provider and model configuration
- add reusable agent instructions and step prompts
- add bounded run observability
- store agents and pipelines in yaml
- add Pi agent runtime
- add prompts to factory pipeline steps

### Fixed

- install Pi extension from release tag
- stop implicit progress retries
- serialize agent instructions as YAML literal blocks
- remove fixed rig agent timeout

### Other

- stabilize Pi installer process tests
- stabilize local Ollama observability assertion
- move factory prompt policy into configuration
- format orchestration assertion

## [0.10.0](https://github.com/klab365/baton/compare/v0.9.0...v0.10.0) - 2026-11-11

### Added

- persist agent context per pipeline run

## [0.9.0](https://github.com/klab365/baton/compare/v0.8.1...v0.9.0) - 2026-11-11

### Added

- run work sessions with configured agents

### Other

- remove run markdown working notes
- remove fixed rig behavior instructions

## [0.8.1](https://github.com/klab365/baton/compare/v0.8.0...v0.8.1) - 2026-11-10

### Added

- add per-run markdown working notes

## [0.8.0](https://github.com/klab365/baton/compare/v0.7.3...v0.8.0) - 2026-11-10

### Added

- continue interrupted agent workflows

## [0.7.3](https://github.com/klab365/baton/compare/v0.7.2...v0.7.3) - 2026-11-10

### Fixed

- release workspace lock despite inherited descriptors

## [0.7.2](https://github.com/klab365/baton/compare/v0.7.1...v0.7.2) - 2026-11-10

### Other

- add browser coverage for Markdown inputs

## [0.7.1](https://github.com/klab365/baton/compare/v0.7.0...v0.7.1) - 2026-11-10

### Fixed

- require actionable blocker details

## [0.7.0](https://github.com/klab365/baton/compare/v0.6.3...v0.7.0) - 2026-11-09

### Added

- align external agent outcome reporting

### Fixed

- update Ctrl+C behavior for graceful shutdown in server

## [0.6.3](https://github.com/klab365/baton/compare/v0.6.2...v0.6.3) - 2026-11-08

### Fixed

- release inherited configuration locks
- prevent orphaned configuration locks
- recover runs despite orphaned temporary files

## [0.6.2](https://github.com/klab365/baton/compare/v0.6.1...v0.6.2) - 2026-11-08

### Fixed

- secure local API against cross-origin requests

## [0.6.1](https://github.com/klab365/baton/compare/v0.6.0...v0.6.1) - 2026-11-08

### Changed

- reorganize the Rust packages and web application under `packages/`

## [0.6.0](https://github.com/klab365/baton/compare/v0.5.1...v0.6.0) - 2026-11-08

### Added

- use shared credential IDs for agents
- support markdown in multiline fields
- reopen completed tasks for follow-up work

### Fixed

- format credential error message

## [0.5.1](https://github.com/klab365/baton/compare/v0.5.0...v0.5.1) - 2026-11-08

### Added

- initialize project repository on first run
- install matching Pi package from Baton

## [0.5.0](https://github.com/klab365/baton/compare/v0.4.0...v0.5.0) - 2026-11-06

### Added

- clarify pipeline run replay
- add configurable shell pipeline steps
- improve run context and live observability

### Fixed

- use available port when starting server

## [0.4.0](https://github.com/klab365/baton/compare/v0.3.0...v0.4.0) - 2026-11-05

### Added

- run pipelines in external agent sessions

### Fixed

- route and filter authenticated provider models
- preserve pipeline feedback across agent steps

## [0.3.0](https://github.com/klab365/baton/compare/v0.2.0...v0.3.0) - 2026-11-04

### Added

- configure agent tool access
- integrate GitHub Copilot provider
- show version in CLI and web app

## [0.2.0](https://github.com/klab365/baton/compare/v0.1.1...v0.2.0) - 2026-11-03

### Fixed

- use pipeline ID as display name
- update task test for pipeline model
- store run traces as readable JSON

### Other

- format pipeline editor
- expect pipeline IDs in agent conflicts

## [0.1.1](https://github.com/klab365/baton/compare/v0.1.0...v0.1.1) - 2026-11-02

### Added

- add favicon and animated workflow graphic
- add project instructions and software factory guide

### Fixed

- include workspace and web changes in releases
- satisfy formatting and update factory test
