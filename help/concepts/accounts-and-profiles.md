---
id: concepts-accounts-and-profiles
title: Accounts and profiles
summary: An account is the identity a harness runs under; a profile is the base configuration a run starts from.
keywords: [login, credentials, identity, preset, client, work, personal]
order: 50
status: current
related: [agents-accounts, agents-starting, guides-accounts-and-profiles, philosophy-your-machine]
---

## What it is

An *account* is a named identity a harness runs under, such as the login Claude Code or Codex uses with
its own service. You can keep several, and choose one each time you start an agent. Two agents of the
same harness can then run side by side under different accounts, a personal one and a work one for
example, and their usage stays apart.

An account points at a credential and never holds it. Ubiq stores no password or token for you: the
harness keeps its own login, and Ubiq only records which identity to run it under. When an agent's
account resolves to nothing, it runs under your own home configuration, and its info panel says so.
[Accounts](agents-accounts) covers choosing one.

A *profile* is a named base configuration a run starts from. What you choose for one run, such as
the model, the reasoning level or the permission mode, is layered on top of it, so a profile saves
you from answering the same questions each time. The New agent form offers your saved setups and a
**Customize** button for changing one for a single start; see [Starting an agent](agents-starting).

## Why it works this way

Keeping identity and configuration separate lets you combine them freely: the same profile under
two accounts, or one account used with several profiles. Keeping credentials out of Ubiq means
signing in, signing out and revoking access all happen where the harness already does them, which
is part of [keeping your machine yours](philosophy-your-machine).
