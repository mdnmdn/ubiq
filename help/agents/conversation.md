---
id: agents-conversation
title: Inside a conversation
summary: The parts of a conversation — header, transcript, prompts, queue, activity bar, composer and footer — and the three-dots menu.
keywords: [composer, queue, activity, transcript, permission, three dots, info, fork, abort, enqueue]
order: 45
status: current
related: [agents-chat, agents-columns, agents-permissions]
---

A conversation looks the same in an [agents column](agents-columns), a [chat tab](agents-chat) and the
maximized chat. From top to bottom it is a header, the transcript, anything waiting on you, and the composer
with its footer.

## The parts

| Part | What it does |
|---|---|
| Header | The three-dots menu at the left and, at the right, a chip naming what the agent is doing |
| Transcript | Messages, thinking and tool calls; follows the end while you are there, and a **Go to last message** button appears when you scroll away |
| Tool call | A card with a verb, its target and a status; click to see its output or diff |
| **N earlier calls** | Three or more tool calls of one kind fold into this row; click to open them |
| **THINKING** box | A run of reasoning in one box, open while the agent is still thinking |
| Permission prompt | On the tool call it concerns, with one button per answer the harness offers |
| **NEEDS YOU** strip | Above the footer, with **Yes**, **All** and **No** for the oldest request, each only if offered |
| Queued messages | Above the composer, oldest first, each with **Send ASAP**, edit and delete |
| Activity bar | A subagent chip on the left and a todo chip on the right; each appears only when it has something |
| Composer | The field, with its **+** for attaching files; its button is **Send**, **Stop**, **Enqueue** or **Steer** |
| Footer | **ctx** (how full the context is now), **tot** (all tokens billed, subagents included) and rings for the account's plan |

## What you can do here

- Send with **Enter**; **Shift+Enter** adds a line; **⌘Enter** / **Ctrl+Enter** sends too. **Up** on the first
  line walks back through what you sent, **Down** forward.
- While a turn runs, an empty composer offers **Stop**; with text typed it offers **Enqueue**, and the message goes
  when the turn ends. Native Codex offers **Steer**, which sends into the running turn at once.
- Attach files: the **+**, typing `@` at the start of a word, or pasting a copied file or picture. A tag turns
  yellow past 300 KiB and red past 500 KiB, because a large file costs context.
- Answer a prompt with its buttons, or press **⌘⌥Y** to allow and **⌘⌥N** to reject the oldest one. A long
  request shows an expand button that opens the full text. See [permission modes](agents-permissions).
- Click the subagent chip to read a subagent's own transcript; its first row returns to the main agent. The todo
  chip lists the plan.
- Answer a structured question the agent raises in its dialog; typing in the composer instead closes it.

The three-dots menu has three groups. Each item is dimmed, never removed, when it does not apply.

- **Stop** interrupts the turn and leaves the harness up. **Abort** kills a harness that stopped answering and
  keeps the conversation. **Unload** shuts it down and keeps the conversation. **Resume** starts it again.
- **Info** opens a panel with the harness, model, permission mode, account, token spend, context and the
  conversation's id, with buttons that reveal its folders. **Fork** copies the conversation and launches a second
  agent from this point. **Make persistent** keeps it across restarts, where the harness allows.
  **Accept all** answers every permission for you. **Dump messages** writes its traffic to a file and copies the
  path.
- **Hide** closes this chat tab only. **Close** ends the conversation and deletes its transcript, after a confirm.

See [resume or fork a conversation](how-to-resume-or-fork-a-conversation).

## When something goes wrong

- **The harness failed to start.** The transcript shows the error with a **Retry**, which is the menu's **Resume**.
  Sending a prompt retries too.
- **The agent seems stuck.** Press **Stop**; if nothing answers, use **Abort** and then **Resume**.
- **No prompt appeared for a tool call.** **Accept all** is on for this conversation; the menu reads
  **Stop accepting all**.
- **No ring in the footer.** The harness stated no context window or plan limit, so none is drawn.
