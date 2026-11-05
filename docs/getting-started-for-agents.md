# Getting started with Baton for coding agents

This guide is for a coding agent helping a user set up Baton in their repository. Read the [README](../README.md) and the relevant linked guides before acting. Ask the user which workflow they want if it is unclear:

- **Managed runtime:** Baton runs agents in task worktrees, with configuration and runs managed in its UI. Follow [Get started](../README.md#get-started) and the [Software Factory guide](getting-started.md).
- **Existing coding-agent session:** use `baton work` with a supported host and an existing worktree. Follow [Work in an existing coding-agent session](session-work.md); its skill package and the Baton binary are installed separately.

## Setup checklist

1. Inspect the repository and environment: check Git, the installed `baton` version, and any existing `.baton/config.json` and project instructions. Do not overwrite existing configuration or discard changes. Ask before installing software or modifying the project.
2. If needed, install the CLI using the current [installation instructions](../README.md#install-the-cli). Check platform support and verify that `baton` is on `PATH`. For `baton work`, ensure the installed binary supports it; follow the source-build instructions in the session guide if necessary.
3. Help the user initialize and configure their chosen workflow. Use the existing configuration if present. For a new pipeline, adapt the appropriate example in the linked guide to the user's project and preferences; do not assume example model or credential IDs work locally. Explain any changes before applying them.
4. For managed agents, have the user add provider credentials through Baton Settings and select available models. Never request secrets in chat or put them in configuration, prompts, logs or version control. See [provider credentials](local-provider-storage.md). The `fake` runtime can be used for offline testing.
5. Verify the setup with the narrowest appropriate checks, such as CLI help and inspecting the configured pipeline. Ask before starting a task, running agents, installing third-party skills or making network requests. Report what was verified, what remains for the user, and any commands or files changed.

A Git worktree is **not a sandbox**: agent tools can access resources with the permissions of the Baton process. Review third-party skills before installing them, and use OS-level isolation where appropriate. Do not commit, push or modify unrelated files without the user's approval.
