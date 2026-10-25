---
id: git-changes
title: The Changes panel
summary: The conflicted, staged and unstaged lists in Git mode, and the commit box under them.
keywords: [stage, unstage, discard, commit, amend, revert, conflict, untracked]
context: [panel.ubiq.git.changes]
order: 20
status: current
related: [git-overview, git-history, git-diff, guides-review-and-commit]
redirects: [git/staging.md]
---

The Changes panel sits on the right of [Git mode](git-overview) and lists what differs from the last
commit, with a box to commit it. Its header reads **Uncommitted changes** and counts the changed
paths. Picking a commit in [History](git-history) points the panel at that commit instead, and a
click on the history's **Uncommitted changes** row brings it back.

## The parts

| Part | What it does |
|---|---|
| Search field | Narrows all three lists to the paths that match what you type |
| **Conflicted**, **Staged**, **Unstaged** | One list each, with a count; click a heading to fold it. **Conflicted** appears only when something is in it |
| **+** on **Unstaged**'s heading | Stages every path in the list |
| **-** on **Staged**'s heading | Unstages every path in the list |
| **+** / **-** on a row | Stages or unstages that path; a path changed both ways is in both lists |
| The letter on a row | **A** added, **M** modified, **D** deleted, **R** renamed, **T** type changed, **U** untracked, **!** conflicted |
| Commit message box | Subject, a blank line, then a body |
| **amend** | Makes the commit replace the previous one |
| **Commit** `<n>` **files** | Commits the staged paths with your message |

Click a row to read its [diff](git-diff). Right-click it for **Stage**, **Unstage**, **Discard**,
**Open**, **Copy path** and, once a commit is selected in History, **Revert to** that commit's short
id, which overwrites the file with its content there.

## What you can do here

- Stage or unstage one file: [Stage a file](how-to-stage-a-file). Staging is per path, not per hunk.
- Review an agent's work and commit it: [Review and commit](guides-review-and-commit).
- Compare two commits instead of the working tree: [Comparing two commits](git-compare-commits).

## When something goes wrong

- **Commit is greyed.** It needs a message and at least one staged path, or **amend** ticked.
- **A conflicted path has no + or -.** Only a path in **Staged** or **Unstaged** can be staged or unstaged from its row.
- **Nothing to commit** means the working tree is clean; **Not a repository** means the folder has no repository.
- **A refused write** prints its reason in red under the box.
- **Discard and Revert to ask first**, because an uncommitted change is in no commit to get back.
- **After a commit** the box empties and the history and branches refresh.
