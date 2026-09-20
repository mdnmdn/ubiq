---
id: workbench-rail
title: The rail
summary: The column down the window's left edge — the mark, the modes and the project badges — and how it switches what the centre and the project are.
keywords: [rail, mark, modes, project badges, switch project]
targets: [ui.rail, ui.rail.mark, ui.rail.modes, ui.rail.projects, ui.rail.mode.teams, ui.rail.mode.teams-old, ui.rail.mode.sink]
order: 5
status: current
related: [workbench-modes, workbench-panels, git-overview, agents-starting, kb-overview]
---

## Three parts, stacked

The rail is the narrow column down the window's left edge, and it is always three things stacked in
the same order: the **mark** at the top, the **modes** below it, and the **project badges** at the
bottom.

- **The mark** is the Ubiq logo, drawn in the open project's colour. Click it to pick a different
  project or to open one that is not in the window yet — it is the rail's own project switcher, not
  just a brand.
- **The modes** are every destination this project offers, one button each, grouped into
  application-level modes (that answer for the window, not one project) and project modes (that need
  a folder open). Exactly one is active at a time, and **⌃1** through **⌃9** jump straight to them in
  the order they are drawn.
- **The project badges** are one badge per project open in this window, newest last. They switch
  which project the rail's modes act on, and **⌘1** through **⌘9** switch between them the same way
  the number jumps switch modes.

## The modes, by group

**Application-level** — these answer for the whole window, not one project:

- **Control** — the window's dashboard: memory held, uptime, how many agents are alive against how
  many have run since launch, and what they have spent in tokens. See
  [Rail modes](workbench-modes#control).
- **Kitchen sink** — the developer's own reference screen, every control and colour the interface is
  built from. Left in for anyone extending Ubiq itself; skip it otherwise.

**Project modes** — inert, and say so, until a project is open:

- [**IDE**](workbench-modes#ide) — the files, the editor and the terminal panes.
- [**Git**](git-overview) — status, diffs, history and refs for the project's repository.
- [**Agents**](agents-starting) — every conversation in the project, one column each.
- **Teams** — groups of agents working a shared brief, split across lanes. An earlier **Teams
  (previous)** screen is kept alongside it while the new one settles, and goes away once nothing
  needs it — if both are visible in your build, the newer one is the one to use.
- [**Knowledge**](kb-overview) — the project's documents, read from wherever they actually live.
- [**Tasks**](workbench-modes#tasks) — the project's cards: queued, in flight and done, and which
  agent has each one.

## Switching project versus switching mode

These are two different axes, and the rail keeps them visually separate on purpose: the badges pick
*which project*, the modes pick *what to look at* in whichever project is picked. Switching a badge
does not change which mode is active — if you were looking at Git, you are still looking at Git,
now against the newly-picked project — and a project mode that project has never had a folder open
for simply says it has nothing to show.
