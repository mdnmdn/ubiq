---
id: workbench-modes
title: Rail modes
summary: What each destination on the rail fills the window with, from IDE and Git to Agents, Tasks and Control.
keywords: [rail, mode, ide, control, tasks, stats, destination]
context: [rail.ide]
targets: [ui.rail.mode.ide, ui.rail.mode.tasks, ui.rail.mode.control]
order: 10
status: current
related: [workbench-rail, workbench-panels, workbench-control, git-overview, kb-overview, db-overview, agents-starting, tasks-overview, teams-overview]
---

A mode is a destination on the [rail](workbench-rail). Picking one changes what fills the centre of
the window and which panels open beside it. Your arrangement in each mode is remembered, so coming
back to a mode finds it as you left it.

## The parts

| Mode | What it shows |
|---|---|
| **IDE** | The project's files: the [explorer](workbench-explorer) on the left and every open file as a tab in the [editor](workbench-editor) |
| **DB** | The project's [databases](db-overview): connections on the left, table data in the centre, SQL editors below. Off until you switch it on |
| **Git** | The project's [repository](git-overview): branches on the left, history and diffs in the centre, changes on the right |
| **Agents** | Every agent in the project, one [column](agents-columns) each |
| **Teams** | How the project's agents are arranged, and which task each serves; see [Teams](teams-overview) |
| **KB** | The project's [documents](kb-overview), read from wherever they live |
| **Tasks** | A board of cards, one column per status; see [Tasks](tasks-overview) |
| **Control** | What Ubiq is doing and what your agents have spent, across every project; see [The Control screen](workbench-control) |
| **All Teams** | Every open project's agents on one canvas, with the same screen as **Teams** |

**IDE** is the mode a project opens in. While a file is open it fills the centre; with none open, the
centre says so and the explorer is where you start. The status bar under it shows the open file and
its kind, and the project's git state.

**Tasks** opens its right region on the selected task, so a card's report or form sits beside the
board. Its status bar counts cards per column, finished sub-tasks and blocked cards.

**Control** and **All Teams** work with no project open. The others need one, and show a page saying
so until you open a folder.

## What you can do here

- Jump to a mode with **⌃1** / **Ctrl+1** to **⌃9** / **Ctrl+9**, counting the project modes from **IDE**; the rail lists them.
- Switch **DB** on, or hide a mode: [Project settings](settings-project).
- Start an agent from any project mode: [The new agent dialog](agents-new-agent).
- Move or split the panels a mode opens: [Split and move panes](how-to-split-and-move-panes).

## When something goes wrong

- **DB** is not on the rail: switch it on in project settings, under **General**, then **Modes**.
- A mode shows that no project is open: use **Add a project** in the [title bar](workbench-titlebar).
- A panel you expect has vanished: closing a panel's tab removes it, and the title bar's region
  buttons bring the region back with that mode's own panel in it. [Panels and the dock](workbench-panels)
  has the details.
