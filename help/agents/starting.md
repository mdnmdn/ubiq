---
id: agents-starting
title: Starting an agent
summary: Agents mode's parallel columns, and what happens when you start a harness in a project.
keywords: [start agent, new agent, agents mode, harness]
context: [rail.agents]
targets: [ui.rail.mode.agents]
order: 10
status: current
related: [panes, sessions, agents-accounts]
---

## Agents mode

**Agents** is one of the rail's project modes, and fills the centre with parallel columns, one per
agent you have started in the project. Each column holds that agent's pane and its status — running,
waiting on a permission, or exited — so watching three agents at once means looking at three columns
rather than hunting through tabs.

## Starting one

The **+** control opens the new-agent menu: pick a harness, an [account](agents-accounts), and the
[permission mode](agents-permissions) it should run under. Ubiq composes the run — skills, MCP
servers, instructions — and launches the real harness binary in a fresh [pane](panes), unchanged
from what you would get running it yourself in a terminal.

## Where else it shows up

The same **+** and the same menu are reachable from the IDE chat strip and the kitchen sink, not
only from Agents mode itself — starting an agent is not tied to one screen.
