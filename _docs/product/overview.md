---
id: prod-overview
title: Product overview
kind: product
status: current
summary: What Ubiq is, who runs it, why an agent harness needs a real terminal rather than a chat box, and what the product deliberately refuses to be.
read_when: you are deciding whether something is in scope, or you have never seen this project before
updated: 2026-08-31
verified: 2026-08-31
review_cycle: quarterly
---

# Product overview

## What Ubiq is

Ubiq is a **harness multiplexer**: a desktop application that hosts several interactive AI coding
agents side by side, each in a real terminal pane, under one window and one set of controls.

Think tmux, with the panes specialised for agent harnesses rather than shells. A developer opens a
session for a piece of work, spawns Claude Code in one pane and Codex in another, watches both, types
into whichever has their attention, and keeps the whole arrangement when they come back tomorrow.

## Who it is for

One developer running several agents at once on their own machine. Not a team server, not a hosted
service, not a CI runner. The product optimises for the case where a person is *watching* agents work
and intervening — reading one pane while another thinks, redirecting a run that has gone sideways,
comparing two harnesses on the same task.

That framing sets the bar for everything else: latency matters more than throughput, fidelity of the
terminal matters more than a pretty chat transcript, and the application must survive an agent
crashing without taking its neighbours down.

## Why a pane is a terminal

Every harness Ubiq hosts is a full-screen, interactive TUI. It takes over the alternate screen,
addresses the cursor absolutely, emits the full ANSI escape vocabulary, asks the terminal how big it
is, redraws when that answer changes, and expects keystrokes back byte for byte — arrows, Ctrl and
Alt chords, bracketed paste, mouse reports.

The consequence shapes the whole product: **a pane is a terminal, not a text buffer.** Ubiq cannot
render an agent's output as a scrolling log and get away with it, because the agent is not producing
a log — it is driving a screen. So each pane owns a genuine terminal emulator, each harness runs
under a genuine pseudo-terminal, and Ubiq's job is to shuttle bytes between the two without opinions
about their content.

This is also what separates Ubiq from a chat frontend. A chat frontend owns the conversation and
calls a model. Ubiq owns nothing about the conversation: it hosts whichever harness the user
chose, unchanged, and adds the multiplexing, the configuration, and the window around it.

## What Ubiq adds on top of running the harnesses yourself

Four things a terminal and a shell do not give you:

**Simultaneity with attention.** Several harnesses visible at once, one focused, keystrokes routed
to the focused one only, and pane chrome that says which agent is in which state without the user
reading its output to work it out.

**A session as a unit of work.** A named piece of work with a home folder and the set of agents
attached to it. Closing the window does not lose the arrangement, and reopening does not mean
respawning everything by hand.

**Composed runs.** An agent is launched with a specific set of skills, MCP servers, and an account —
assembled per run, rather than inherited from whatever is installed globally on the machine. Ubiq
does not implement this itself; it embeds a library that does, so the same composition works from
the terminal and from the window.

**One place to configure them all.** Each harness invents its own config location and format. Ubiq
presents one surface over the set.

## Scope

In scope, in the order the product cares about it:

1. Hosting interactive harnesses in real terminal panes, faithfully — colours, resize, raw
   keystrokes, alternate screen.
2. Sessions and workspaces: grouping, naming, persisting and reattaching to a set of running agents.
3. Layout and focus: splitting, arranging, and routing input to one pane.
4. Lifecycle: spawning, watching, surviving and reporting an agent's exit.
5. Composed launches, via the embedded harness-management library.
6. Agent-to-agent orchestration: a main agent that spawns subagents into their own panes.

## Non-goals

- **Writing a terminal emulator or a VT parser.** Ubiq integrates one. Every hour spent on escape
  sequence handling is an hour not spent on the product.
- **Being a chat client.** Ubiq does not talk to a model API, does not own a conversation, and does
  not render messages. It hosts the harness that does.
- **Replacing the harnesses' own configuration.** Ubiq composes a run and leaves the user's real
  config untouched.
- **Being a secrets manager.** Accounts carry references to credentials, never the credential
  material.
- **Multi-user or hosted operation.** One person, one machine, one window.

## The one constraint that outlives every feature

Ubiq is built to be split. The part that owns processes and PTYs and the part that draws panes talk
to each other over a defined contract and nothing else — even though they share a process today.
That discipline buys two futures at almost no present cost: a detached coordinator that keeps agents
alive while the window is closed, and agents running on other machines entirely.

The engineering form of that rule, and what it forbids, is in
[`../tech/architecture.md`](../tech/architecture.md).

## Related docs

- [`glossary.md`](./glossary.md) — the terms this document uses without defining
- [`../tech/architecture.md`](../tech/architecture.md) — how the product shape becomes a code shape
- [`../backlog.md`](../backlog.md) — what is unresolved
