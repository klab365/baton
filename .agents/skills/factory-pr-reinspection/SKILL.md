---
name: factory-pr-reinspection
description: Verify a retained task branch and existing pull request during explicit PR/CI reinspection.
---

Use this only when the task explicitly requests PR/CI reinspection. Treat it as a follow-up on the same task: verify the retained worktree and branch, and do not switch tasks or create speculative changes. Inspect the existing task branch's committed diff against the PR target base even if the worktree is clean; assess new uncommitted edits separately. Verify the intended base is an ancestor of HEAD when applicable.

Keep these identities distinct in the handoff: repository, task branch, PR URL, PR target-base SHA, task-branch HEAD SHA, incremental changed paths and patch fingerprint, and expected remote task-branch SHA (or explicitly ABSENT). Never substitute the target-base SHA for the task-branch SHA. Report actionable changes and verification, or state that no changes are needed while passing the verified identity onward. Block if identity or branch state is missing or ambiguous.
