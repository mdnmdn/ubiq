---
id: agents-chat
title: The chat tab
summary: A chat tab shows one agent's conversation beside your code, and can be pointed at any agent.
keywords: [chat, conversation, transcript, tab, focus, attach]
context: [view.chat]
order: 40
status: current
related: [agents-conversation, agents-starting, workbench-panels]
---

A harness in a terminal shows what an agent is doing; a chat tab shows what it was asked and what it
concluded, beside the code. It is available in IDE mode, in Tasks and on the Teams screens. A chat tab is a
view of an agent's [conversation](agents-conversation), not a conversation of its own: you can open several,
each pointed at a different agent or at none, and a tab's **×** hides it without ending the agent.

## The parts

| Part | What it does |
|---|---|
| Play button on an empty tab | Tooltip **start a new agent**; opens the [new agent dialog](agents-new-agent) |
| Change agent chevron | Tooltip **change agent**; its first row is **New agent**, the others are agents to move this tab to, with a filter field |
| Three-dots menu | The agent's lifecycle verbs, described in [the conversation](agents-conversation) |
| Status chip | At the right of the header, what the agent is doing now |
| Maximize button | Shows this chat near full window; the same button then reads Minimize |
| Tab name | The agent's name, cut to fifteen characters; hover for its summary, identity, task and handle. An empty tab reads **New chat** |
| **+** on the tab strip | Opens **New agent** and **Attach existing agent** and adds a chat tab |

A tab's hexagon mark is the same one the agent wears in its column, so the two always agree.

## What you can do here

- Start an agent into a tab: the play button, the chevron's **New agent** row, or the **+** on the tab strip.
- Point a tab at another agent from the chevron. Agents already shown in another panel of this mode are not
  listed; the tab's own agent is selected at the bottom of the list.
- Show a chat near full window with the Maximize button, or press **⌘⇧Enter** / **Ctrl+Shift+Enter** for the chat
  whose composer has the keyboard. Your draft, attachments and scroll position carry over. **Esc**, the
  button, the shortcut or a click outside puts the chat back.
- Move a chat tab to any region or tab it with others: see [split and move panes](how-to-split-and-move-panes).
- Pin a tab from its right-click menu. A pinned tab has no **×** and no **Hide**; **Rename**, **Close** and
  **Unpin** still work.
- Hide a tab without ending the agent: see [hide or close a tab](how-to-hide-or-close-a-tab).
- Answer an agent's question or permission prompt right in the transcript: see [permission modes](agents-permissions).

## When something goes wrong

- **A tab came back empty after a restart.** Only an agent marked persistent is reattached on its own; a
  conversation a harness can restore reopens by itself when restoring is switched on, and otherwise waits for
  you to restore it.
- **The conversation's tab disappeared.** Closing a conversation for good closes every tab looking at it.
- **The + does nothing.** All the chat tabs the window allows are open. Close a tab you no longer use.
- **Start is refused.** The harness was uninstalled after the list was drawn. Pick another harness.
