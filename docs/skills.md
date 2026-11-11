# Project skills

Skills are stored in `.agents/skills/<id>/SKILL.md` and versioned with the project.
Agent files under `.baton/agents/` contain `skillIds`, not skill instruction bodies. Global skill directories are not searched.

Use **Skills** in the sidebar to create, edit, import, or delete a skill. The skill ID is also its display name. The optional multiline description is stored
in YAML block-scalar form (`description: |`) in `SKILL.md` frontmatter. GitHub import downloads the skill folder, including supporting files. Use a GitHub folder URL with a commit SHA to select a specific revision; automatic updates and rollback are not implemented.

Skill use cases and validation live in `baton-core/features/skills.rs`, storage is
contracted through `baton-core/ports/skills.rs`, and the filesystem adapter lives
in `baton-adapters/src/skills.rs`. HTTP routes dispatch mediator commands.

Agent saves reject unknown skill IDs. Assigned skills cannot be deleted. The
configured runtime loads skills when it is created, strips YAML frontmatter from
assigned `SKILL.md` content, and appends only the remaining instructions in
`skillIds` order after the system prompt. The source file and metadata are kept
intact when editing a skill. Missing assigned
skills fail execution. Instructions from multiple skills are combined, not
merged or deduplicated; avoid contradictory instructions. Skills do not grant
permissions, change tool assignments, or automatically execute bundled scripts.
Supporting files are not automatically injected into the runtime context.

Skill paths reject traversal and symlinks. Writes replace `SKILL.md` atomically.
Project directories must still be trusted: filesystem checks are not a sandbox
against another process concurrently replacing directory components.
