---
id: agents-starting
title: Agents mode
summary: The Agents screen — a row of columns, one per conversation — and the ways to start an agent from it.
keywords: [start agent, new agent, agents mode, harness, columns]
context: [rail.agents]
targets: [ui.rail.mode.agents]
order: 10
status: current
related: [agents-columns, agents-new-agent, panes, sessions]
---

**Agents** ![Agents](../img/ui/mode-agents.png) is a rail mode for talking to your agents. It fills the centre with
a row of columns side by side, each one a transcript and a composer over one conversation, so you can
follow several agents at once without switching tabs. The [sidebar](agents-columns) on the left lists
every conversation in the project.

## The parts

| Part | What it does |
|---|---|
| The strip over the columns | Counts the columns, the agents they hold and how many are grouped; holds **New agent** and **Close all** |
| A column | One conversation: a tab strip, its own **+**, the transcript, and the composer at the foot |
| **New agent** | Opens the [new agent dialog](agents-new-agent); a second row, **Attach existing agent**, picks a conversation the project already has |
| **Close all** | Takes every agent off the screen without ending it; it appears only while something is on screen |
| The drop strip at the end of the row | Drop a tab past the last column to give it a column of its own |

Eight columns fit the row. Grouping into a column that is already open always works.

## What you can do here

- Start an agent: **New agent**, then fill in the [dialog](agents-new-agent).
- Put two agents in one column by dragging one tab onto another column, to read a hand-off side by side.
- Follow one agent closely: see [two agents side by side](guides-two-agents-side-by-side).
- Bring back an agent you took off screen: pick it in the [sidebar](agents-columns), or in **Attach existing agent**.
- Hide a tab without ending the agent: see [hide or close a tab](how-to-hide-or-close-a-tab).
- Change how much an agent may do unasked: see [permission modes](agents-permissions).

The same **New agent** menu is on the chat strip in IDE mode and in the kitchen sink, so starting an agent is
not tied to this screen. Closing a column's tab hides the agent; the agent keeps running and the sidebar still
lists it. Ending an agent for good is the three-dots menu in [the conversation](agents-conversation).

## When something goes wrong

- **A split is refused.** The row already holds eight columns. Group the tab into an open column instead.
- **The screen says which control brings an agent back.** Every agent is off screen. Use the sidebar or
  **Attach existing agent**.
- **Attach existing agent is dimmed.** The project has no conversation to attach.
- **A tab drops somewhere that is not a column.** It returns to the column it came from.
