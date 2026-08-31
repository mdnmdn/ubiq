---
id: tech-agent-manager
title: The agent-manager boundary
kind: tech
status: draft
summary: What the embedded harness-management library owns, what Ubiq owns, how the application consumes it, and the rule that keeps the two from growing into each other.
read_when: you are about to write code that launches a harness, names a harness config path, or touches accounts, skills or MCP servers
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [crates/ubiq/Cargo.toml, crates/agent-manager/src/lib.rs, crates/agent-manager/src/spec.rs]
depends_on: [tech-structure]
review_cycle: monthly
---

# The agent-manager boundary

## What the library is

`agent-manager` wraps a running harness. Rather than executing `claude` directly, it composes a run
— skills, MCP servers, an account, initial instructions, hooks, all pulled from a catalog — into a
throwaway configuration directory, optionally inside an isolated environment, and launches the real
binary against it. The user's own `~/.claude` and its siblings are read-only for the duration.

It ships two front ends: an `am` CLI for the terminal, and a front-end-agnostic library for
embedding. Ubiq is the embedder.

Its full documentation lives with the crate, starting at `crates/agent-manager/_docs/README.md`.
**That library owns every fact about harness configuration.** This document owns only the boundary.

## The division

| Fact | Owner |
|---|---|
| Where a harness stores its config, and in what format | the library |
| How to launch a harness, and with which arguments | the library |
| What a run is composed of — skills, MCPs, account, instructions, hooks | the library |
| Which accounts exist and how credentials are referenced | the library |
| Session history and resume, as the *harness* understands it | the library |
| How a harness's I/O is bridged into structured events | the library |
| That a harness runs under a pseudo-terminal in a pane | Ubiq |
| Which panes exist, which is focused, how they are laid out | Ubiq |
| A session as a *user's* piece of work, with a home folder | Ubiq |
| The window, the theme, the chrome | Ubiq |

The overlapping word is **session**, and the two meanings are genuinely different. The library's
session is a harness conversation that can be resumed. Ubiq's session is a named grouping of panes
with a folder. A document that means one must say which.

## How Ubiq consumes it

The library exposes a run as a value — a `RunSpec` in `crates/agent-manager/src/spec.rs` — built
from flags, settings and catalog contents, then handed to a provisioner that materialises the
configuration directory and produces a launch. Ubiq's coordinator constructs that value
programmatically instead of parsing command-line flags, and spawns the resulting launch under a
pseudo-terminal it owns.

Everything an embedder can substitute is a trait: the catalog registry, the account store, the
secret store, profiles, templates, session history, and an in-process MCP service. Ubiq supplies its
own implementations where it wants application-specific behaviour and takes the filesystem defaults
elsewhere.

Two feature decisions follow from embedding rather than shelling out:

- **`default-features = false`.** The library's default build pulls in `clap` and `ratatui` for its
  own front ends. An application that has a window needs neither.
- **`inproc-mcp` when Ubiq exposes its own tools.** The library can host an embedder-registered MCP
  service on a loopback endpoint and inject it into the run as an ordinary remote MCP server —
  which is how a hosted agent calls back into Ubiq.

`crates/ubiq/Cargo.toml` declares no dependency on the library, and neither does
`crates/ubiq-host/Cargo.toml`, which is where the edge belongs: the host owns configuration and
processes, and the interface may not name either. Adding that edge, and the agent
registry that reads from the catalog rather than a hard-coded list, is tracked in
[`../backlog.md`](../backlog.md).

## The rules

**1. Ubiq never names a harness configuration path.** Not `~/.claude`, not `CLAUDE_CONFIG_DIR`, not
a settings filename. If Ubiq needs one, the library grows an accessor. A path literal in
`crates/ubiq/src/` is the clearest possible sign the boundary has been crossed.

**2. Ubiq never hard-codes how to launch a harness.** Which binary, which arguments, which
environment — the library answers all three. Ubiq's `agent.rs` holds the *user-facing* agent-type
list and gets its launch facts from the library.

**3. Nothing about windows, panes or terminals goes into the library.** The library builds with no
UI dependency and must keep doing so; that property is what lets it stay embeddable by anything.

**4. New harness support is a library change.** Adding a harness means a new implementation there,
against the runtime contract already written up in `crates/agent-manager/_docs/harness/`. Ubiq gains
the harness with no change of its own — which is the whole point of the split.

**5. A fact stated in the library's documentation is linked, never copied.** Two copies of a harness
launch flag is one copy that goes stale silently.

## Rationale

**Why embed rather than shell out to `am`?** A subprocess boundary would cost a serialisation round
trip on every run, make in-process MCP impossible, and turn every error into parsed text. The
library was built to be embedded, and its core is deliberately free of terminal and CLI types.

**Why keep harness knowledge out of the application at all?** Because it is the fastest-moving,
most-copied knowledge in this domain: config locations move, flags change, and every tool that
wraps agents ends up with a stale table of them. Concentrating it in one crate with its own
documentation means the table is wrong in one place, and fixable in one place.

## Related docs

- [`project-structure.md`](./project-structure.md) — the two crates and their division of labour
- [`architecture.md`](./architecture.md) — where the coordinator sits, and what it is allowed to hold
- `crates/agent-manager/_docs/README.md` — the library's own documentation, starting point
