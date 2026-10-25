---
id: workbench-control
title: The Control screen
summary: Control mode — what Ubiq is holding right now and what your agents have spent in tokens, across every project.
keywords: [control, stats, usage, tokens, spend, memory, uptime, cost, how much]
context: [rail.control]
order: 90
status: current
related: [workbench-modes, workbench-rail, agents-conversation, settings-harnesses]
---

**Control** ![Control](../img/ui/mode-control.png) is the first mode in the **APP** group of the
[rail](workbench-rail). It answers for the whole application rather than one project, so it works
with no project open. It has two pages, **General** and **AI**, in a strip along the top, and a line
beside the strip saying what the page reports.

## The parts

| Part | What it does |
|---|---|
| **General** | Five readings of what Ubiq is doing right now |
| **Memory** | How much memory Ubiq holds, or `unavailable` where the system will not say |
| **Uptime** | How long Ubiq has been running, as `1h 05m` or `04m 12s` |
| **Open projects** | How many projects have at least one pane running |
| **Agents live** | How many agents are alive now |
| **Agents this run** | Every agent started since Ubiq launched, including ones that have ended, so it only rises |
| **AI** | A table of what agents have spent, one row per hour, harness, model and subagent |

The **AI** table has the columns **Hour**, **Harness**, **Model**, **Subagent**, **In**, **Out**,
**Think**, **Other**, **Total**, **Msgs in**, **Msgs out** and **Tools**. The token columns count what
went in, what the harness wrote, its reasoning, and anything else; **Total** adds them. A row for a
harness's own work reads `itself` under **Subagent**, and a model the harness did not name reads
`unknown`.

A reading nobody has reported yet is drawn as an em dash, not as zero. The page refreshes each time
you enter the mode.

## What you can do here

- See which modes the rail offers: [Rail modes](workbench-modes).
- Check one conversation's own usage: [Inside a conversation](agents-conversation).
- Look at what is left of a plan, by account: [Harnesses and isolation](settings-harnesses).

## When something goes wrong

- Every reading is an em dash: the figures have not arrived yet. Leave the mode and come back.
- **AI** says `Nothing recorded`: no agent has reported spend yet. A harness that does not state its
  token counts, Codex among them, adds no rows.
- **Memory** says `unavailable`: the operating system declined to give it, so there is no number to
  show.
- The numbers do not match a harness's own bill: they cover only what Ubiq saw reported, from the
  moment it was recording.
