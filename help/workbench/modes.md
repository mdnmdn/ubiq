---
id: workbench-modes
title: Rail modes
summary: What the rail's IDE, Tasks and Control destinations fill the centre of the window with.
keywords: [rail, mode, ide, control, tasks, stats]
context: [rail.ide, rail.control, rail.tasks]
order: 10
status: current
related: [workbench-panels, git-overview, kb-overview, agents-starting]
---

## The rail selects what the centre is for

The rail on the left switches between destinations, grouped in two: application-level modes
(**Control**, and the kitchen sink used to test Ubiq against itself) and project modes (**IDE**,
**Git**, **Agents**, Teams, the knowledge base and **Tasks**). Exactly one is active at a time, and a
project mode with no folder open simply says so.

## IDE

**IDE** is the default: it fills the centre with the files you have open, one panel per file, and
steps aside entirely while any is open. The [explorer](workbench-explorer) and the
[editor](workbench-editor) both belong to it.

## Tasks

**Tasks** fills the centre with a board — the columns and cards a project's outstanding work is
tracked as, independent of any one agent's run.

## Control

**Control** is application-level rather than project-scoped: it fills the centre with the stats
screen — how much memory the host holds, how long it has been up, how many agents are alive now
against how many have run since launch, and what those agents have spent in tokens. It is the one
place that answers "how much is all of this costing," across every project open in the window.
