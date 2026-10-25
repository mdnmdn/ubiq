---
id: teams-inspector
title: Selecting a card and the tasks drawer
summary: What clicking a card on the Teams canvas opens, and the drawer of tasks beneath the canvas.
keywords: [inspector, tasks drawer, delegate, selection, card, steps]
order: 20
status: current
related: [teams-overview, tasks-overview, agents-chat]
---

On [the Teams screen](teams-overview), clicking a card, or a delegate inside a card's fence, opens
that agent's conversation in a chat tab in the right region, and opens the region if it was hidden.
Under the canvas, a drawer lists the tasks for what you have selected.

## The parts

| Part | What it does |
|---|---|
| Chat tab on the right | The agent's own conversation, the same transcript and composer as any [chat tab](agents-chat); picking a delegate points it at the delegate |
| **Tasks** drawer | A bar under the canvas reading the selected agent's or workspace session's name, then `<n> tasks · <m> steps`; click it to open or close the list |
| A task in the drawer | Its title, its shape and a `done/total` count, with a `waits on` mark when it cannot start yet |
| A step | A line under its task, muted once done, with a button naming the agent that owns it |

## What you can do here

- Click a step's agent name in the drawer to select that agent's card and open its conversation.
- Use the arrow on a row of the canvas's first picker to make the drawer follow one workspace
  session or project; ticking rows only narrows the canvas.
- Plan, edit and complete the tasks themselves on the [Tasks board](tasks-overview).

## When something goes wrong

- **The drawer says `No task for this selection.`** What is selected has no task; select
  another card or use a picker's arrow.
- **Nothing opens on the right.** Choosing a workspace session with the arrow opens no
  conversation, since it has none; select a card instead.
- **The drawer is closed.** Click its **Tasks** bar to open it.
