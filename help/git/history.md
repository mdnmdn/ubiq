---
id: git-history
title: Browsing history
summary: The commit graph, its lanes, and searching commits by message or author.
keywords: [history, log, commit graph, search commits]
context: [panel.ubiq.git-history]
order: 30
status: current
related: [git-overview, git-changes, git-refs]
---

## The graph

History draws the repository's commit graph in lanes, newest at the top, with the column header and
each commit's message, author and time beside it. Selecting a commit shows its [diff](git-overview)
underneath.

## Searching

The search field above the graph filters visible rows by message or author as you type — search
hides rows without recomputing the graph's lanes, so filtering a long history stays responsive.

## Jumping from a ref

Selecting a [ref](git-refs) in the left region jumps history to the commit it points at and scrolls
it into view, without leaving the screen you were on.
