---
id: agents-permissions
title: Permission modes
summary: How much an agent may do without asking first, chosen per run.
keywords: [permissions, approval, autonomy, sandbox]
order: 30
status: draft
related: [agents-starting]
---

## Choosing how much an agent may do

Every harness has its own notion of what it may do without stopping to ask — running a command,
writing a file, reaching the network. Ubiq surfaces a permission mode in the new-agent menu so you
pick that stance per run, from asking before anything to letting the agent proceed unattended.

## A conservative default

Starting an agent with no permission mode chosen keeps it on the harness's own most cautious
setting, so a first run never does more than you expect.

This page is a draft: the per-harness mapping from Ubiq's permission modes to each harness's own
flags is still being written up.
