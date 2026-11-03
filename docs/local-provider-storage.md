# Provider credentials

Agents select a runtime, provider/model and, where required, a reusable local credential. Project configuration stores credential IDs and non-secret settings, not API keys or refreshable authentication records. Manage credentials through the UI; the Rig adapter resolves them locally at execution time. The fake runtime does not contact a model.

Each project has a vault under `~/.local/share/baton/credentials/projects/<project-id>`. Set `BATON_CREDENTIAL_DIR` on the backend process to override the base directory. The non-secret `projectId` in `.baton/config.json` is initialized or migrated at startup. Clones with the same ID share the vault, and only one Baton process may use that ID at a time. Existing local IDs are retained during migration. Credentials formerly stored in the global vault are not imported or removed; reconnect accounts and remove old secrets deliberately when no longer needed. Protect vault directories and do not commit them or put secrets in project configuration, task prompts, run records or logs.

## GitHub Copilot

A GitHub account with active Copilot access, a supported model, network access to GitHub and Copilot, and a browser for device authorization are required. In **Settings → Add credential**, choose **GitHub Copilot session**, enter a name, and use the displayed link and one-time code. After authorization, select the credential and an available Copilot model for the agent. **Test connection** checks the saved session; model availability can depend on account and organization policies.

Sessions and refreshable tokens are stored in the local project vault, not the project configuration. The vault is local storage rather than an encrypted secret manager; protect the local account and backups. Worktree tools run with the same operating-system permissions and can potentially access the vault. Deleting a credential removes local session files but does not revoke GitHub authorization; revoke access in GitHub account settings when needed. If access is revoked or expires, delete the credential and connect the account again; reassign agents to the new credential. The device and model endpoints depend on GitHub's service and may change.

A Git worktree is not a security sandbox. Agents and their tools run with the permissions of the Baton process; use operating-system isolation when handling untrusted code or sensitive credentials. See [runtime orchestration](orchestration.md).
