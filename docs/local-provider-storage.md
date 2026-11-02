# Provider credentials

Agents select a runtime, provider/model and, where required, a reusable local credential. Project configuration stores credential IDs and non-secret settings, not API keys or refreshable authentication records. Manage credentials through the UI; the Rig adapter resolves them locally at execution time. The fake runtime does not contact a model.

The local credential directory defaults to `~/.local/share/baton/credentials`; set `BATON_CREDENTIAL_DIR` on the backend process to override it. Protect this directory and back it up or remove it according to your local security policy. Do not commit it or put secrets in project configuration, task prompts, run records or logs.

A Git worktree is not a security sandbox. Agents and their tools run with the permissions of the Baton process; use operating-system isolation when handling untrusted code or sensitive credentials. See [runtime orchestration](orchestration.md).
