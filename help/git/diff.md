---
id: git-diff
title: The Diff panel
summary: How the diff under the history compares a path or a commit, and the split and unified views.
keywords: [diff, hunk, comparison, side by side, unified, split]
context: [panel.ubiq.git.diff]
order: 50
status: current
related: [git-changes, git-history, git-compare-commits]
---

The Diff panel sits under the history in [Git mode](git-overview) and shows what a changed path
changed. It comes forward when you pick a path in [Changes](git-changes) or in a commit's file list.
Its header names the path and holds the view controls.

## The parts

| Part | What it does |
|---|---|
| The path | Which file the comparison is about |
| **split** | Draws the old and new text side by side |
| **unified** | Draws one column, removed lines then added lines |
| The chevron at the right | Shuts the panel so the history gets the whole height, or opens it again |
| The body | Added, removed and unchanged lines, with line numbers |

## What is compared

What the panel shows depends on the list the path is in:

- A path under **Staged** shows what the next commit would carry.
- A path under **Unstaged** shows what is not staged yet.
- A path under **Conflicted** shows its difference from the last commit.
- A path picked after selecting two commits shows what differs between them; see
  [Comparing two commits](git-compare-commits).

## What you can do here

- Choose the view that reads best for the change: **split** for edits to a line, **unified** for
  added or removed blocks.
- Scroll sideways to reach a long line; lines are not wrapped.
- Decide what to keep: [Review and commit](guides-review-and-commit).

## When something goes wrong

- **Pick a changed path to compare it** — nothing is selected yet; click a row in Changes.
- **Reading…** stays while Ubiq waits for the comparison; the last path's text is never left
  under the new name.
- **The panel asks for another path** when the one you were reading has no changes left, for
  instance after you discard it.
