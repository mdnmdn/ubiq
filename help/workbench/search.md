---
id: workbench-search
title: Search
summary: Finding text across a project's files without leaving the window.
keywords: [search, find, grep, content search]
context: [panel.ubiq.search]
order: 40
status: current
related: [workbench-explorer, workbench-editor]
---

## Searching a project

The search panel finds a string or pattern across every file in the open project, outside whatever
an agent's own tools might do in a pane. Results are grouped by file, and picking one opens the
[editor](workbench-editor) at that line.

## Scope

Search respects the same ignore rules the explorer does, so build output and dependency folders do
not drown out a project's own source in the results list.
