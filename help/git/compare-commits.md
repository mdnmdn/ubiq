---
id: git-compare-commits
title: Comparing two commits
summary: Select two commits in the history and read which files differ between them.
keywords: [compare, range, two commits, diff between, between commits]
order: 60
status: current
related: [git-history, git-changes, git-diff]
---

Use this to see everything that changed between two points in the history, not just one commit.

1. Select the first commit in [History](git-history).
2. Hold **⌘** and click a second commit.
3. Read the list in the panel beside the history: its header says **Comparing** and counts the
   paths that differ, oldest to newest.
4. Click a path to open its [diff](git-diff) between the two commits.

A comparison is read-only, so its rows have no **+** or **-**. The search field above the list
narrows the paths as you type. A plain click on any commit, or on the **Uncommitted changes** row,
ends the comparison and returns [Changes](git-changes) to that row's own view. While the two
commits are being read, the strip shows **Comparing…**.
