# Local provider storage

Provider metadata is stored outside every project in `~/.local/share/baton/providers/settings.json` (via Rig's `FileTokenStore`). On Unix Rig creates the directory and files owner-only (`0700` directories, `0600` files). OpenAI API keys and ChatGPT OAuth/session records are stored in separate owner-only directories below this location.

`.baton/config.json`, API responses, run records, browser storage, and logs never contain credentials, OAuth tokens, or device codes. The Settings API returns only provider metadata and temporary device-code prompts; responses use `Cache-Control: no-store`. Deleting or disconnecting a provider removes its private credential/session directory.

This is a permission-restricted local secret store rather than an OS keychain because the runtime's supported ChatGPT device-code implementation uses Rig's private file token store. It is intentionally local to the operating-system user and is not portable with a project.
