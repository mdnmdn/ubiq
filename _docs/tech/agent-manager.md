---
id: tech-agent-manager
title: The agent-manager boundary
kind: tech
status: draft
summary: What the embedded harness-management library owns, what Ubiq owns, how the application consumes it, and the rule that keeps the two from growing into each other.
read_when: you are about to write code that launches a harness, drives one as a conversation, names a harness config path, or touches accounts, skills or MCP servers
updated: 2026-09-03
verified: 2026-09-03
code_anchors: [crates/ubiq-host/Cargo.toml, crates/ubiq-host/src/agent.rs, crates/ubiq-host/src/conversation.rs, crates/agent-manager/src/lib.rs, crates/agent-manager/src/spec.rs, crates/agent-manager/src/isolate.rs, crates/agent-manager/src/io/mod.rs]
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
| How a harness's I/O is bridged into structured events, and what those events are called | the library |
| The one translation from those events onto the bus | Ubiq |
| What a policy grants, and how the operating system enforces it | the library |
| Whether an agent is confined at all, and where its run directory lives | Ubiq |
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

`crates/ubiq-host/src/agent.rs` is the whole of that consumption, and it is deliberately thin. The
agent-type list is `harness::all()` projected into `AgentTypeInfo`, each row marked with whether the
harness's own binary is on this machine. A spawn naming one of those ids composes a run — a
`RunSpec` with the harness, the project's folder and the policy setting — provisions it, and answers
with what to exec. A spawn naming anything else is a program name, which is what a shell is.

**A workspace has two faces, and `agent.rs` composes both.** `Agents::compose` is the terminal one:
`IoModes::Passthrough`, and a launch to exec under a pseudo-terminal. `Agents::converse` is the
other: `IoModes::Structured`, and a `structured_bridge` over the harness's own JSON instead of a
launch, because a conversation's harness writes frames on a pipe rather than drawing a screen. What
differs between them beyond the mode is the run directory's name and the isolation, both below.

**The bridge is owned by a pump thread, and `crates/ubiq-host/src/conversation.rs` is that thread.**
`IoBridge::next_event` blocks and both its methods take `&mut self`, so whoever reads a bridge
cannot also be handed a prompt; the reader owns it and a turn reaches the harness through the
detached `AgentInputSink` the bridge hands out. A harness that answers `None` there takes no second
turn, which is the honest signal rather than a guess, and `Conversation::accepts_input` is how the
interface asks. Events reach the window on the same unbounded mailbox a pseudo-terminal's reader
uses, so a window behind on drawing never stalls the harness.

**One file knows both vocabularies.** `map_event()` in the same module is the only place that names
`agent_manager::io::AgentEvent` and `ubiq_proto::conversation::ConvUpdate` together. Both are the
Agent Client Protocol's `session/update` vocabulary — `D53` — so the translation is a rename, and
confining it to the host is what keeps the interface free of any dependency on this library. A
second mapping anywhere else is the boundary being crossed.

Three things in these files are Ubiq's rather than the library's, and all three concern ownership
rather than configuration. **A run's configuration directory belongs to whatever owns the run**: it
is `ConfigStrategy::Fixed` under Ubiq's own config root, named by the pane id or by the agent id —
both ULIDs, so neither can be read as the other's — deleted when that pane closes or that
conversation is retired, and swept at startup for whatever a killed process left. **An agent in a
pane is confined unless the host settings say otherwise** — the policy grants the project's folder
and that directory, with an ephemeral `$HOME`. Which harnesses opt out, and under which policy,
stays the library's: it has the layered shape for that, and a second one here would be two places to
look. See `D52`. And **a conversation is confined by nothing**, whatever the setting says, because a
bridge owns its child's descriptors and a sandbox needs them; every bridge answers each tool
approval itself for the same reason. That is `G92` in [`../backlog.md`](../backlog.md), deliberate
for the first end-to-end slice.

Confining a run in a terminal Ubiq owns is macOS-only. isol8 spawns with inherited stdio and keeps
its child handle private, so no host can hand it a pseudo-terminal; `isolate::confined_launch`
renders the policy and execs `sandbox-exec`, which macOS supports and Landlock cannot. The seam that
replaces it is specified in `refs/isol8-pty-seam-update.md`.

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

`crates/ubiq-host/Cargo.toml` declares the dependency and `crates/ubiq/Cargo.toml` does not, which
is where the edge belongs: the host owns configuration and processes, and the interface may not name
either. `just host` and `just ui` are the mechanical checks that this stayed true. What a run is
composed *of* beyond a harness and a folder — skills, MCP servers, an account, a model — needs a
composition on the wire, and is tracked in [`../backlog.md`](../backlog.md).

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
