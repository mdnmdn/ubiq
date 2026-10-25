---
id: tasks-detail
title: A task
summary: The panel that opens when you select a card — its description, steps and the agents working on it.
keywords: [task, card, detail, steps, mission, comments, prerequisites, plan, delete]
context: [panel.ubiq.task]
order: 20
status: current
related: [tasks-overview, agents-starting, guides-tasks-board]
---

Select a card on the [Tasks board](tasks-overview) and the right panel reports that task whole. With
nothing selected it says **No task open**. The popup switch in the board's toolbar draws the same
panel over the columns instead, and the right panel then says where it went.

## The parts

| Part | What it does |
|---|---|
| Title | Click it to edit; a stage tag (such as **IN REVIEW**) sits under it, with **BLOCKED** added when a step is blocked; the stage changes only by moving the card |
| Priority | Three words in a row, one lit |
| Facts | **Key**, **Link**, **Level**, **Parent**, **Kind**, **Complexity**, **Assigned to**, **Labels**, **References**, **Prerequisites**, **Blocks**, **Attachments** and **Colour**; **Level** turns a task into a mission (or back) |
| Hide empty fields button | Hides the facts that have no value; it reads **Show empty fields** while they are hidden |
| **Shape** | How the agents on this task are arranged (direct, chain or coordinated), with a note on what the choice means, and a picker for the workspace session it belongs to |
| **Description** | Markdown, with **Write** and preview, **Save** and **Cancel** |
| **Sub-tasks** | The task's steps, each with its own state; add one in the field under the list |
| **Comments** | What you and the agents have written, each marked `user`, `agent` or `ubiq`; add one in the field under the list |
| **Open** | On the agent working the task: opens its conversation |
| **Assign to an agent** / **Continue with an agent** | Starts an agent on the task, or carries on with the one that was working it, while the task is in backlog, ready or in progress |
| **Complete** / **Feedback to an agent** | On a task in review: accepts it, or sends it back to an agent |
| **Open plan** | On a mission: opens its plan |
| **Delete** | Asks **Delete** or **Keep** before removing the task |

A mission's panel also offers **Open** the mission and **Show only this mission**, and the controls
that move it through its phases: **Confirm**, **Reject…**, **Retry**, **Done** and **Fail…**.

## What you can do here

- Create a task: press **New task** on the board, type a title or a description, and press
  **Create**. A description with no title is named for you from its first line.
- Give a task to an agent, review its result and complete it:
  [Plan work on the Tasks board](guides-tasks-board).
- Choose the agent and its setup in [Starting an agent](agents-starting).

## When something goes wrong

- **Some fields will not edit.** A board bound to another service that only pulls says so at the top
  of the panel; set the binding to two-way in the project's **Task sync** section to change that.
- **The agent buttons and the shape are missing.** A named board has no agents and draws neither.
- **A prerequisite or parent is refused.** A task cannot wait on itself or on a task that would
  close a loop, and only a mission can be a parent.
