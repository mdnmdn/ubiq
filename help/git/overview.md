---
id: git-overview
title: The Git screen
summary: Reviewing refs, history and uncommitted changes for a project's repository, on the same window.
keywords: [git, repository, branch, commit]
context: [rail.git]
targets: [ui.rail.mode.git]
order: 10
status: current
related: [git-changes, git-history, git-refs]
---

## What it is

**[Git mode](ubiq://./git)** fills the centre with a project's repository: refs on the left, history
in the centre, and uncommitted changes on the right, on a first visit — every region can be dragged,
hidden or resized like any other. A repository selector in the toolbar lists more than one when a
project holds several.

## The four panels

- [Refs](git-refs) — branches, tags and remotes, grouped in the left region
- [History](git-history) — the commit graph, searchable, in the centre
- [Changes](git-changes) — staged, modified and untracked paths, on the right
- Diff — the comparison for whichever path or commit is selected, under the history

## What is not built yet

Branch creation, stashing and undo are inert in this build — the four panels above are read and
write for changes and commits, and read-only for the rest.
