---
id: prod-glossary
title: Glossary
kind: product
status: current
summary: Plain definitions of the recurring terms — harness, agent type, session, workspace, pane, coordinator, bus, catalog — for anyone reading the rest of this documentation.
read_when: you met a term in another document and are not certain what it names here
updated: 2026-08-31
verified: 2026-08-31
review_cycle: quarterly
---

# Glossary

Terms in the order a newcomer meets them. Where a term has an owning document, that document is the
authority and this entry is the one-line version.

### Harness

An interactive AI coding agent that runs as a full-screen terminal program: Claude Code, Codex,
Gemini CLI, opencode, GitHub Copilot CLI. Ubiq hosts harnesses; it does not implement one. The word
covers the program itself, not a particular run of it.

### Agent type

A harness Ubiq knows how to launch: an identifier, the binary to execute, a human label, and the
arguments passed on spawn. The set of agent types is what the user picks from when creating a
workspace.

### Session

A named piece of work that groups the agents serving it, with a home folder and a creation time. A
session outlives any single agent inside it, and the user attaches to and detaches from it rather
than opening and closing it.

### Workspace

One running instance of one agent inside a session: which agent type, which working directory, how
big its terminal is, and whether the process is alive. A session holds many; each owns exactly one
pseudo-terminal and one child process.

### Pane

The visible half of a workspace: a terminal emulator on screen, plus the chrome around it — title,
status, borders. Panes are what the layout arranges and what focus selects between. One workspace,
one pane.

### Focus

The property of being the pane that receives keystrokes. Exactly one pane has it. Everything else
keeps drawing but takes no input.

### Pane ID

The identifier that ties a pane to its workspace, its pseudo-terminal, and every message about
either. Every message on the bus carries one.

### Coordinator

The half of Ubiq that owns processes: it spawns each harness under a pseudo-terminal, reads its
output, writes keystrokes to it, propagates resizes, and reports exits. It renders nothing. Its
counterpart is the UI, which draws panes and owns no process.

### Bus

The single channel the UI and the coordinator use to talk. Neither side calls the other directly.
The message set it carries is the transport contract, owned by
[`../tech/transport-contract.md`](../tech/transport-contract.md).

### PTY (pseudo-terminal)

The kernel object that makes a program believe it is attached to a real terminal: a pair of ends,
one held by the harness as its standard input and output, one held by the coordinator. It is what
makes a harness emit colour and full-screen drawing rather than plain piped output.

### SIGWINCH / TIOCSWINSZ

The two halves of a resize. The coordinator sets the pseudo-terminal's new size (`TIOCSWINSZ`); the
kernel tells the harness its window changed (`SIGWINCH`); the harness redraws. Getting this wrong is
the classic source of corrupted terminal layouts.

### Alternate screen

The second screen buffer a full-screen terminal program switches to, so that quitting restores
whatever was on screen before. Harnesses use it, which is why a pane has to be a real terminal.

### agent-manager (`am`)

The library and CLI that composes a harness run — pulling skills, MCP servers, an account and
instructions from a catalog into a throwaway config directory, then launching the real binary
against it. Ubiq embeds it rather than reimplementing it. Its own documentation lives with the
crate; the boundary is stated in
[`../tech/agent-manager.md`](../tech/agent-manager.md).

### Catalog

agent-manager's store of the skills and MCP servers a run can be composed from. A run names what it
wants; the catalog supplies it.

### Skill

A packaged set of instructions injected into a harness's configuration for one run.

### MCP server

A Model Context Protocol server: an external process or endpoint exposing tools a harness can call.
Composed into a run the same way a skill is.

### Account

A named identity a harness runs under. Accounts carry *references* to credentials, never the
credential material.

### Profile

A persistent named base configuration a run starts from, before the per-run composition is layered
on top.

### Subagent

An agent spawned by another agent rather than by the user. Ubiq's interest in the term is that a
subagent gets its own pane, so the user can watch it.

### Layout mode

How panes are arranged in the window: one pane filling it, a vertical split, a horizontal split, or
a grid.

### Theme token

A named colour in the application's palette — a surface, a text weight, a border, a status — used
instead of a literal colour anywhere in the UI. Owned by
[`../tech/ui-and-design.md`](../tech/ui-and-design.md).

## Related docs

- [`overview.md`](./overview.md) — what these terms are in service of
- [`../INDEX.md`](../INDEX.md) — which document owns which of these facts
