---
id: agents-new-agent
title: The new agent dialog
summary: Choosing what a new agent runs as, how it is set up and what it is told first, and how it gets its name.
keywords: [new agent, launch, name, harness, account, agent definition, profile, model, start in terminal]
order: 50
status: current
related: [agents-starting, agents-accounts, agents-definitions, agents-permissions]
---

The dialog opens from **New agent** in [Agents mode](agents-starting), from the chat strip's **+** and from the
kitchen sink. It asks everything a start needs in one form. Everything below the first row is faint and takes no
click until you have chosen what to run.

## The parts

| Part | What it does |
|---|---|
| **Agents** tab | Your saved [agent definitions](agents-definitions), one row each, as `name — harness · model · level`; project ones first, then global ones |
| **Harness** tab | A harness and the [account](agents-accounts) it runs as; moving between the tabs starts the answer again |
| **Customize** | Under a chosen definition; turns the harness, model, level and mode rows on for this one start |
| Model, reasoning level | What the harness offers; the dialog opens on the last launch, else the harness's own default |
| Permission mode | The harness's own labels; see [permission modes](agents-permissions) |
| Subagent ceiling | How many subagents the agent may use; the agent is told it, not given a switch |
| **Initial prompt** | Sent as the agent's first message, so it starts working without a keystroke |
| **MCPs**, **Skills** | Checklists of what Ubiq can add to the run; ticks apply to this agent |
| **Custom policies** | Extra folders the agent may write; live only while agent isolation is on in settings |
| **Start** | Starts the agent as a conversation in a column or chat tab |
| **Start in terminal** | Starts the same run as a terminal pane; it ignores the initial prompt and the ceiling |
| **Save agent definition** | Saves these answers as an agent definition; offered when the target is a bare harness |

The **Harness** tab lists **Configured** rows (a harness with an account signed in), then **Default** rows, which
use the harness's own configuration in place. A harness whose program is missing is drawn dimmed. A harness with
no conversation view, such as Grok, is not offered here. The Claude Code and Codex
adapter variants appear only after you switch them on in [Settings](settings-harnesses).

## What you can do here

- Start from your last choice: the dialog opens on it for the project, else on the last start anywhere.
- Name an agent. Ubiq names it after its harness, with a number from the second one on. After the agent answers its
  first prompt it takes a short title of its own, if a provider is set up in [Settings](settings-assist). Rename it
  from its tab to keep your name for good.
- Choose a model and level before the harness launches; the agent's composer offers the same pickers until it
  has.
- Hide or end the agent later: see [hide or close a tab](how-to-hide-or-close-a-tab).

Press **Esc** to dismiss; nothing is created, so a dismissed dialog leaves no empty tab or column.

## When something goes wrong

- **No agent definition is listed.** None has been saved, or the ones that exist are switched off. Use the **Harness** tab.
- **A harness cannot be chosen.** Its program is not on this machine. Set its command in
  [Settings](settings-harnesses) and it becomes available.
- **The dialog opens on nothing.** The harness or definition it remembers has been removed.
- **The start is refused.** The harness was uninstalled after the list was drawn. Pick another.
- **The model row says it is asking.** Ubiq is asking the harness for its models; the row fills when it answers.
