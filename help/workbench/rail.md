---
id: workbench-rail
title: The rail
summary: The column down the window's left edge — the project mark, the modes and the project badges — and how it switches what the centre shows.
keywords: [rail, mark, modes, project badges, switch project, switch mode, activity bar]
targets: [ui.rail, ui.rail.mark, ui.rail.modes, ui.rail.projects, ui.rail.mode.teams]
order: 5
status: current
related: [workbench-modes, workbench-titlebar, workbench-panels, projects, git-overview, agents-starting, kb-overview, db-overview, teams-overview, tasks-overview]
---

The rail is the narrow column down the window's left edge. It picks what the centre of the window is
for — files, Git, agents, tasks — and, when the window holds several projects, which of them you
are looking at. Exactly one mode is active at a time, drawn in the accent colour.

## The parts

| Part | What it does |
|---|---|
| The mark | The Ubiq logo above the rail, in the open project's colour; it tells you which project the window is on |
| **APP** modes | **Control** and **All Teams**; they answer for the whole window and work with no project open |
| **PROJECT** modes | **IDE**, **DB**, **Git**, **Agents**, **Teams**, **KB** and **Tasks**; each needs a project, and says so until one is open |
| Project badges | One badge per project in this window, at the foot of the rail; click one to switch to that project |

Each mode is one button, an icon over its label. The modes are described in
[Rail modes](workbench-modes); the project's own pages are [Git](git-overview),
[Agents](agents-starting), [KB](kb-overview), [DB](db-overview), [Teams](teams-overview) and
[Tasks](tasks-overview).

Project badges show a project's initial, or the letters its settings give it, in its colour. The
current project is the filled square, the others are rings, and a badge's tooltip is the project's
name. Badges appear only when the window holds more than one project, newest last, and only as many
as fit under the modes. **⌘1** to **⌘9** jump to the nth badge; **⌃1** to **⌃9** jump to the nth
project mode that is switched on, counting from **IDE**. **Control** and **All Teams** have no digit.

## What you can do here

- Jump to a file, a project or a place from the keyboard: [Use the navigator](how-to-use-the-navigator).
- Switch project from the title bar instead: [The title bar](workbench-titlebar).
- Hide a mode you never use, or show **DB**: [Project settings](settings-project).
- Change where a panel sits once you are in a mode: [Split and move panes](how-to-split-and-move-panes).

Switching project does not change the mode. If you were on **Git**, you are still on **Git**, now
for the other project.

## When something goes wrong

- A project mode shows a page saying no project is open: open a folder with **Add a project** in the
  [title bar](workbench-titlebar).
- **DB** is missing: it is off until you switch it on in project settings, under **General**, then
  **Modes**.
- A mode you expect is missing: it may be hidden in [project settings](settings-project). The rail
  always keeps at least one mode.
- No badges: the window holds one project, the **Open projects in the rail** setting is off, or the
  window is too short for them. **Settings** has the switch.
- **⌃3** lands on the wrong mode: the digits count only the modes that are switched on, so hiding a
  mode shifts the ones after it.
