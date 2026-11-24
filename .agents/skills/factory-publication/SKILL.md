---
name: factory-publication
description: Safely publish reviewer-approved task-branch changes and recover from partial publication.
---

Publish only reviewer-approved changes. Before side effects, verify repository, task branch, remote branch and existing PR against the review handoff. Keep distinct: PR target-base SHA, pre-commit task HEAD SHA, incremental reviewed paths/fingerprint, expected remote task-branch SHA or ABSENT, repository, branch and PR URL. A new commit's parent must be the pre-commit task HEAD, never the PR target base.

If HEAD and worktree are unchanged, verify the remote branch also matches and do not commit or push. If HEAD is exactly one new local commit on the pre-commit task HEAD, the worktree is clean, and its patch matches this run's reviewed fingerprint, treat it as already committed. Compare the remote with the expected remote state: push without force if unchanged; if expected ABSENT, verify that exact branch is still absent and create it with a normal push; if already at this commit, do not push again. Create a PR only if none exists; otherwise retain or update it. CI inspection is user-triggered, not automatic. Stop on unexpected history, patch, remote state, unrelated changes, or ambiguity.

For uncommitted work, stage only reviewed changes and compare the staged diff to the reviewed diff. Never make an empty/duplicate commit, rewrite history, merge, force-push, or push a protected branch. Use a Conventional Commit and include actual verification results and a closing issue reference when applicable.

If publication partially fails, preserve the review facts verbatim and report the new commit SHA/parent, actual remote SHA or ABSENT, push result, PR state, side effects and exact remaining operation. Never report a partial publication as having no side effects.
