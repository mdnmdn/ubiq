---
id: agents-new-agent
title: The new agent dialog
summary: Choosing what a new agent runs as, how it is set up and what it is told first, and how it gets its name.
keywords: [new agent, launch, name, harness, account, agent definition, profile, model, start in terminal]
order: 50
status: current
related: [agents-starting, agents-accounts, agents-definitions, agents-permissions]
---

The dialog opens from **New agent** in [Agents mode](agents-starting) and from the chat strip's **+**. It asks
everything a start needs in one form. Everything below the first row is faint and takes no click until you have
chosen what to run.

## The parts

| Part | What it does |
|---|---|
| **Agents** tab | An **Agent** dropdown of your saved [agent definitions](agents-definitions), one row each, as `name — harness · model · level`; with both kinds, **This project** ones come first, then **Global** ones |
| **Harness** tab | A **Harness** dropdown: a harness and the [account](agents-accounts) it runs as; moving between the tabs starts the answer again |
| **Customize** | Under a chosen definition; turns the harness, **Model**, **Thinking effort** and **Mode** rows on for this one start |
| **Model**, **Thinking effort** | What the harness offers; the dialog opens on the last launch, else the harness's own default |
| **Mode** | The permission mode, in the harness's own labels; a new agent opens on the harness's unattended mode, such as **Bypass permissions** for Claude Code; see [permission modes](agents-permissions) |
| **Persistent** | A checkbox that keeps the conversation across a restart of Ubiq; dimmed, and its note says why, for a harness that keeps its history outside Ubiq |
| **Max subagents** | How many subagents the agent may use; the agent is told it, not given a switch |
| **Ask for feedback**, **Plan mode** | Shown only when the dialog opens from a task's **Assign to an agent**; each changes what the initial prompt tells the agent — to ask when it needs to, or to plan before it acts |
| **Initial prompt** | Sent as the agent's first message, so it starts working without a keystroke |
| **MCPs**, **Skills** | Checklists of what Ubiq can add to the run; ticks apply to this agent |
| **Custom policies** | Extra folders the agent may write; live only while agent isolation is on in settings |
| **Start** | Starts the agent as a conversation in a column or chat tab |
| **Start in terminal** | Starts the same run as a terminal pane; it ignores the initial prompt and **Max subagents** |
| **Save agent** | Saves these answers as an agent definition; offered when the target is a bare harness |

The **Harness** tab lists **Configured** rows (a harness with an account signed in), then **Default** rows, which
use the harness's own configuration in place. Every harness Ubiq supports is listed; one whose program is missing
is drawn dimmed and cannot be picked. The ACP versions of Claude Code
and Codex appear only after you turn on their **Enable … ACP** switch in [Settings](settings-harnesses).

## What you can do here

- Start from your last choice: the dialog opens on it for the project, else on the last start anywhere,
  with the **Mode** you used then.
- Name an agent. Ubiq names it after its harness, with a number from the second one on. After the agent answers its
  first prompt it takes a short title of its own, if a provider is set up in [Settings](settings-assist). Rename it
  from its tab to keep your name for good.
- Choose a model, thinking effort and mode here, and change them later from the pickers beside **Send** in the
  agent's [conversation](agents-conversation); they stay live for the conversation's whole life.
- Hide or end the agent later: see [hide or close a tab](how-to-hide-or-close-a-tab).

Press **Esc** to dismiss; nothing is created, so a dismissed dialog leaves no empty tab or column.

## When something goes wrong

- **No agent definition is listed.** None has been saved, or the ones that exist are switched off. Use the **Harness** tab.
- **A harness cannot be chosen.** Its program is not on this machine. Set its command in
  [Settings](settings-harnesses) and it becomes available.
- **The dialog opens on nothing.** The harness or definition it remembers has been removed.
- **The start is refused.** The harness was uninstalled after the list was drawn. Pick another.
- **The model row says it is asking.** Ubiq is asking the harness for its models; the row fills when it answers.
