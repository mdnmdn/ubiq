---
id: agents-columns
title: The agents columns and sidebar
summary: What a column shows — header, tabs, thread and footer — and the sidebar that lists every conversation in the project.
keywords: [columns, sidebar, thread, footer, sessions list, session, missions, bench]
context: [panel.ubiq.agents.explorer]
order: 15
status: current
related: [agents-starting, agents-conversation]
---

In [Agents mode](agents-starting) the centre is a row of columns and the left region is the sidebar. A column
is where you talk to one agent; the sidebar is the one place the whole project is visible at once, including
agents that no column is showing.

## The parts

| Part | What it does |
|---|---|
| Column header | The agent's role mark, its name, and under it its identity, branch and worktree; a grouped column also shows its count |
| Status mark | A hexagon on every tab: the outline says whether the agent can still act, the core says what it is doing |
| Column tabs | One per agent; more than one tab makes a group. A long name is cut and shown in full on hover |
| **+** on a column | Groups another agent into this column, free agents first; agents already on screen are listed but dimmed |
| Thread | The [conversation](agents-conversation) itself, the same view the chat tab shows |
| Footer | What the turn has spent, how full the context is, and the rolling five-hour usage window where Claude Code reports one |
| Sidebar session rows | One group per workspace session, with a bar down its left edge; the bar takes the worst state under it: error, then waiting, then running, then ended |
| Sidebar agent rows | Status mark, name, identity, a chip with what the agent is doing, and the three-dots menu on a live agent |
| Sidebar **Missions** | One row per open mission with its phase, a dot while it needs you, and its live agents |

Colours read the same everywhere: yellow needs you, blue is working, green is idle, grey has stopped.

## What you can do here

- Bring an agent on screen: click its row in the sidebar. An agent in a column comes to the front; one that is
  off screen gets a column, or joins the focused column when the row is full.
- Fold a workspace session with its chevron; the header's one control folds or opens every one.
- Rename an agent from its tab's right-click menu. An agent also names itself once it has answered its first
  prompt, if a provider is set up in [Settings](settings-assist); your own name is never overwritten.
- Hide or end an agent: see [hide or close a tab](how-to-hide-or-close-a-tab) and the
  [conversation's menu](agents-conversation).
- Open a mission from the **Missions** section; its chevron expands the planner and workers.

## When something goes wrong

- **A column says "No conversation".** This window holds no conversation for that agent, so there is nothing
  to draw. Pick another agent from the sidebar.
- **A harness that exits stays on screen.** Its transcript stays and it takes no further turn; closing the tab
  takes it off screen.
- **No ring in the footer.** The harness reported no context window, so none is drawn rather than a guess.
- **A workspace session with no agents is missing from the sidebar.** Only workspace sessions with agents
  are listed; see [Workspace sessions](sessions).
