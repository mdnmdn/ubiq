---
id: philosophy-what-ubiq-is-not
title: What Ubiq is not
summary: What Ubiq deliberately does not try to be, so your expectations match what it does and you know where to look instead.
keywords: [not, limits, scope, alternative, ci, server, hosted, team, password manager]
order: 50
status: current
related: [philosophy-why-ubiq, philosophy-your-machine, glossary]
---

## What it is

Ubiq hosts the coding agents you choose and gives you one window to watch and steer them. Some
things sit outside that on purpose:

- **Not a model of its own.** Ubiq does not do your coding work itself. Every agent is a
  [harness](philosophy-any-harness) you picked, running unchanged, and the work and the model are
  the harness's.
- **Not a replacement for a harness's own settings.** Ubiq puts a run together for the task and
  leaves your real harness configuration untouched. To change how a harness behaves everywhere,
  use that harness's own settings and documentation.
- **Not a secrets manager.** An account points at your credentials and never holds them. Your
  keychain and your harnesses keep the secrets — see
  [Your machine, your credentials](philosophy-your-machine).
- **Not a team server, a hosted service or a CI runner.** Ubiq is for one person at their own
  machine, watching agents work. It does not run jobs unattended for a team.

## Why it works this way

Each of these is a job other tools already do well, and doing it again inside Ubiq would make it
worse at its own job: showing you several agents at once, faithfully, and letting you step in.

Keeping the harness unchanged means what you learn about it holds inside Ubiq and outside it.
Keeping your settings and secrets where they already live means leaving Ubiq takes nothing with it.

The words Ubiq uses for what it does are defined in the [Glossary](glossary).
