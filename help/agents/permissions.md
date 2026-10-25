---
id: agents-permissions
title: Permission modes
summary: Where you set how much an agent may do without asking, and how its permission prompts are answered.
keywords: [permissions, approval, autonomy, sandbox, accept all, allow, reject, plan mode]
order: 30
status: current
related: [how-to-switch-permission-mode, agents-new-agent, agents-conversation]
---

Every harness has its own idea of what it may do without stopping to ask: run a command, write a file, reach
the network. A permission mode is that setting, and Ubiq shows each harness's own labels for it rather than
translating them. You choose one when you start an agent and can change it at any point; when the agent
does need to ask, the question appears in its [conversation](agents-conversation).

## The parts

| Part | What it does |
|---|---|
| **Mode** row in the [new agent dialog](agents-new-agent) | Picks the mode the agent starts with, in the harness's own words |
| Mode picker beside **Send** | The same choice, next to the model and thinking pickers; it stays live for the conversation's whole life |
| Mode in an [agent definition](agents-definitions) | A definition can pin a mode, so every start from it uses the same one |
| Permission prompt | Sits on the tool call it concerns, with one button per answer the harness offers; a lasting answer is marked as lasting |
| **NEEDS YOU** strip | Above the footer; **Yes**, **All** and **No** answer the oldest request, each shown only if the harness offered it |
| Expand button, tooltip **Show full request** | Opens a long request in full, with the same buttons and **Cancel** |
| **Accept all** in the three-dots menu | Answers every permission for this conversation with a plain allow |

## What you can do here

- Choose a mode for a new agent in the dialog, or change a running agent's mode from the picker beside
  **Send**; see [switch a permission mode](how-to-switch-permission-mode).
- Answer a prompt with its buttons, or press **⌘⌥Y** to allow and **⌘⌥N** to reject the oldest request of the
  conversation you are reading. They work while the composer has the keyboard.
- Let one conversation run unattended with **Accept all**; the menu then reads **Stop accepting all**. Turn it
  off the same way.
- Find which mode an agent is on in **Info**, in the three-dots menu.
- Pin a mode in an agent definition for work you always run the same way.

Several requests can wait at once, and each blocks its turn until answered; **Stop** ends the turn and discards
them all. With **Accept all** on, the transcript holds tool calls you were never asked about, which is why the
setting belongs to one conversation, starts off, and is labelled while on.

## When something goes wrong

- **The agent seems stuck.** It may be waiting on a request scrolled out of view. The **NEEDS YOU** strip is
  there to catch that; click its label to jump to the question.
- **The shortcut does nothing.** The conversation has no request waiting, or none offering an allow or reject
  answer.
- **A button you expected is missing.** The harness did not offer that answer, so Ubiq does not draw it.
