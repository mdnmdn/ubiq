---
id: tasks-overview
title: The Tasks board
summary: The Tasks mode — the board of columns and cards a project's work is planned on, and its toolbar.
keywords: [tasks, board, kanban, cards, labels, columns, backlog, archive, mission]
context: [rail.tasks]
order: 10
status: current
related: [tasks-detail, workbench-modes, teams-overview]
---

Pick **Tasks** on the rail to see what there is to do in the open project and how far each piece has
got. The board is a row of columns, one per stage, with one card per task; the panel on the right
reports the card you select (see [A task](tasks-detail)). Agents that were given the task tools write
to the same board, so a card an agent creates or moves appears here.

## The parts

| Part | What it does |
|---|---|
| Columns | One per stage: backlog, ready, blocked, in progress, in review, done and abandoned, each with a count; click a column's heading to shut it to a strip |
| A card | A task's title, with its key, kind, labels and priority; its left edge shows the worst thing happening in it, unless you gave the card a colour |
| Filter field | Narrows the cards to those matching what you type in their title, description, key, kind, labels or workspace session; `key:` matches only keys and `#` only labels |
| **all tags** | Picks labels; a card must carry every one you tick |
| **all missions** | Shows one mission and its subtasks, or every task; not drawn on a named board |
| **Ready only** | Hides every task that is waiting on a prerequisite |
| Popup switch | Draws the selected task over the columns instead of in the right panel |
| Archive button | Files every done and abandoned task away at once, **Archive done and abandoned tasks**; a finished mission stays |
| **New agent** | Opens the new agent dialog, so the agent starts as a chat tab beside the board; not drawn on a named board |
| **New mission** | Starts a mission, named with your project's own word for it; not drawn on a named board |
| **New task** | Opens a form in the right panel; nothing is created until you give it a title or a description |

## What you can do here

- Plan work and hand it to agents end to end: [Plan work on the Tasks board](guides-tasks-board).
- Drag a card to another column, or in front of another card, to change where the work has got to.
- Select a card to read it, edit it and give it to an agent: [A task](tasks-detail).
- See the agents themselves arranged by task: [The Teams screen](teams-overview).
- Rename the word **mission**, choose which columns show and add named boards in
  [Project settings](settings-project).

## When something goes wrong

- **The board looks empty.** A filter is on: clear the field, untick the tags, turn off **Ready only**
  and set the mission picker back to all tasks.
- **A mission's card will not move to another column.** Its column is its phase, so a drop in
  another column is refused; only the mission's own controls change it.
- **A task disappeared after archiving.** Archived tasks leave the board, and the board has no way
  to bring them back.
