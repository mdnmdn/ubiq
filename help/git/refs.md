---
id: git-refs
title: Branches, tags and remotes
summary: The Refs panel's sections, and how selecting a ref moves the history graph.
keywords: [branch, tag, remote, ref, checkout]
context: [panel.ubiq.git-refs]
order: 40
status: current
related: [git-overview, git-history]
---

## Five sections

The Refs panel groups a repository's refs into five sections — local branches, remote branches,
tags, and two more for less common cases — with the current branch always sorted first, then the
trunk branches, then everything else alphabetically. Each section can be shut to save space; a
search across all of them bypasses shut sections rather than requiring you to open one first to
search it.

## Selecting a ref

Clicking a ref reveals the [history](git-history) panel if it was hidden, jumps the graph to the
commit that ref points at, and scrolls it into view.

## Not yet built

Creating a branch, renaming one, or checking one out from this panel is not wired up in this build —
refs here are for orientation and jumping through history, not for changing what is checked out.
