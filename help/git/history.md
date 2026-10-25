---
id: git-history
title: The History panel
summary: The commit graph, its search and filters, and what picking a commit does.
keywords: [history, log, commit graph, search commits, cherry-pick, reset, revert]
context: [panel.ubiq.git.history]
order: 30
status: current
related: [git-overview, git-changes, git-refs, git-compare-commits]
---

History fills the centre of [Git mode](git-overview) with the repository's commit graph, newest at
the top. Each row shows the graph lanes, the **Message**, the **Author**, **When** and **SHA**. While
something is uncommitted, the top row reads **Uncommitted changes** with a count, and Ubiq opens on it.

## The parts

| Part | What it does |
|---|---|
| Search field | Filters the rows by message or SHA as you type |
| Branch chip, **All branches** | Walks one branch instead of the checked-out one |
| **my commits** | Keeps only the commits you wrote |
| `<n>` of `<m>` commits | How many rows the filters leave |
| A commit row | Click to select it; its summary and files appear in [Changes](git-changes) |
| **Load more commits** | At the foot; the list also loads more as you scroll |

Right-click a commit for **Copy SHA**, **Cherry-pick**, **Revert** and **Reset (soft)**,
**Reset (mixed)** or **Reset (hard)**. **Reset (hard)** asks first.

## What you can do here

- Read what a commit changed: select it, then pick one of its files to open the [diff](git-diff).
- Compare two commits: [Comparing two commits](git-compare-commits).
- Jump to a branch or tag's commit: pick it in [Branches](git-refs).

## When something goes wrong

- **A commit you jumped to is missing.** Ubiq clears the filters and loads older commits until it
  finds it.
- **The list is short.** It loads as you scroll; use **Load more commits** if nothing seems to load.
- **Loading history…** shows in the strip while a page is on its way.
- **Cherry-pick, Revert and the soft and mixed resets** run at once and put a commit or staged
  changes on top of what you have; **Reset (hard)** discards the working tree, so it asks.
