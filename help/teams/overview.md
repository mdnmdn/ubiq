---
id: teams-overview
title: The Teams screen
summary: Teams mode — a canvas with one card per working agent, over one project or over every project.
keywords: [teams, graph, canvas, cards, delegates, all teams, arrange, zoom, orchestration]
context: [rail.teams, rail.teamsall]
order: 10
status: current
related: [teams-inspector, agents-starting, tasks-overview]
---

**Teams** on the rail draws the agents running in the open project as cards on a canvas, with a
dashed outline around the cards that serve the same task. **All Teams** is the same screen over every
project the window holds, each project's cards and outlines in its own colour. Use it to see who is
doing what, and to hand an agent to another task by moving its card. Only agents that are running
appear; a project with none says **No agent is running in this project.**

## The parts

| Part | What it does |
|---|---|
| A card | One agent: its status mark and title, its harness and model, the title of its task, a corner chip with its state, and its token count |
| Delegates | Subagents the agent started, one card each under it inside a fence; a card that is waiting on you says `need you` |
| Task outline | A dashed box round the cards serving one task, with the task's shape written on it |
| First picker | **all sessions** on Teams, **all projects** on All Teams; tick the ones to draw, or use the arrow on a row to look at that one only |
| **all states** | Ticks which states to draw: running, waiting, ended, error |
| **Hide done** | Hides delegates that have finished cleanly; failed ones stay |
| **New agent** | Opens the new agent dialog, asking which project first when more than one is drawn |
| Zoom and layout | A **-** and **+** with the zoom percentage, and a picker of five arrangements: Radial, Multiradial, Spider, Hex and Islands |
| Reset zoom button | Puts the zoom back to 100% |
| **Rearrange** | Lays the graph out again in the chosen arrangement, throwing away positions you set by hand |
| **Open the agent panel** | Opens the right panel's chat with nothing attached, to start or attach an agent |

## What you can do here

- Click a card or a delegate to read and answer its conversation: [Selecting a card and the tasks
  drawer](teams-inspector).
- Drag a card into another task's outline to hand the agent that task; dropped on open ground it
  stays where you let go. Drag the empty ground inside an outline to move the whole task.
- Pan with the wheel or two fingers (**Shift** turns a vertical wheel sideways), and zoom with
  **⌘+scroll** / **Ctrl+scroll** or a pinch.
- Start an agent for a task: [The new agent dialog](agents-new-agent).
- Choose the arrangement new canvases open in: [Appearance and size](settings-appearance).

## When something goes wrong

- **No agent matches the filters.** A picker is narrowing the canvas; untick its rows, since an
  empty picker draws everything.
- **A card will not go into a task.** Under All Teams a card only enters a task of its own project,
  so other projects' outlines do not light up.
- **Cards sit in a tangle after hiding or ending agents.** Press **Rearrange**.
- **The canvas is empty.** Teams draws running agents only; start one, or look in
  [The agents columns and sidebar](agents-columns) for the conversations that ended.
