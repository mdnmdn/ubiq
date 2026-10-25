---
id: how-to-stage-a-file
title: Stage a file
summary: Add a changed file to the next commit, or take it out again.
keywords: [stage, unstage, add, commit, git add]
order: 60
related: [git-changes, guides-review-and-commit]
---

Use this when you want a file's changes in the next commit. Staging works per file, not per hunk.

1. Pick **Git** on the rail. The Changes panel is on the right.
2. Find the path under **Unstaged**, or type part of its name in the panel's search field.
3. Click the row to read its diff.
4. Press **+** on the row. The path moves to **Staged**.
5. To take a path out again, press **-** on its row under **Staged**.
6. To move every path in a list at once, press **+** on **Unstaged**'s heading or **-** on **Staged**'s.

The path is listed under **Staged**, and the commit button counts it.

A path changed both ways is in both lists. A path under **Conflicted** has no **+** or **-**. Right-click a row for **Discard**, which asks first because an uncommitted change is in no commit to get back. [The Changes panel](git-changes) has the commit box, and [Review and commit](guides-review-and-commit) walks through a whole review.
