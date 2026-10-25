---
id: agents-definitions
title: Agent definitions
summary: A saved setup you start agents from, and how to create, edit and clone one.
keywords: [agent definition, profile, preset, settings, harness config, clone, recipe]
order: 60
status: current
related: [agents-accounts, agents-permissions, agents-new-agent, guides-accounts-and-definitions]
---

An agent definition is a saved answer to the questions the [new agent dialog](agents-new-agent) asks: which
harness, as which [account](agents-accounts), with which model, level and permission mode, and what to add to
the run. It is how you start a reviewer or a builder the same way every time. An account says *who* an agent
runs as; a definition says *how* it runs. Settings lists them under **Agent definitions**.

## The parts

Open **Settings**, then **Agent definitions**.

| Part | What it does |
|---|---|
| A definition row | `name — harness · account · model · mode · N MCPs`, showing what the definition pins and leaving out the rest |
| Badges | **Coordinator**, **Worker** or **Off**, as the definition says about itself |
| **Add agent** | Opens the definition form empty; dimmed when no harness is available on this machine |
| **Clone** | Saves a copy under the first free name, `<name> copy`, `<name> copy 2` |
| **Edit**, **Delete** | Open the form, or remove the definition after a confirm |

The definition form is the new agent dialog with a name and **Save** in place of **Start**, and no tabs: a
definition is always written against a harness. It asks the target, model, reasoning level, permission mode,
subagent ceiling, the **MCPs** and **Skills** to add, extra folders, an **Agent prompt** that is given to the
agent as its opening instruction, a description and tags. The form is opened from a project's settings for a
project definition, and from the application's settings for a global one.

## What you can do here

- Create a definition from scratch with **Add agent**, or from a start you just configured with **Save agent
  definition** in the new agent dialog.
- Start from one: pick it on the dialog's **Agents** tab, then use **Customize** for a one-off change.
- Rename a definition by changing its name in the form; the definitions and missions that refer to it follow.
- Keep a definition per client or per role, each naming its own account: see
  [accounts and agent definitions for several clients](guides-accounts-and-definitions).

A definition in a project beats a global one of the same name, in that project only.

## When something goes wrong

- **A rename is refused.** The new name is taken. Choose another.
- **A row is dimmed.** Its harness is not installed on this machine; the definition is kept and works again
  where the harness exists.
- **A definition is missing from the dialog.** It is marked **Off**: it stays listed in Settings but is offered
  nowhere a run begins.
- **Add agent is dimmed.** No harness is available. Install one or set its command in
  [Settings](settings-harnesses).
