---
id: tech-architecture
title: Architecture
kind: tech
status: current
summary: The two halves — coordinator and UI — the single bus between them, the rules neither may break, and why the split is drawn before it is needed.
read_when: you are about to add a capability that crosses the UI/coordinator line, or you want to know why the code is shaped this way
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [crates/ubiq/src/lib.rs, crates/ubiq/src/main.rs, crates/ubiq/src/app.rs]
review_cycle: quarterly
---

# Architecture

## The shape

Ubiq has two halves and one channel between them.

The **coordinator** owns everything process-related: it spawns each harness under a pseudo-terminal,
reads that terminal's output, writes keystrokes into it, propagates geometry changes, tracks each
pane's lifecycle, and reports exits. It renders nothing. It is a process supervisor plus an I/O
router.

The **UI** is a stream-attach client: one terminal emulator per pane, fed the bytes that arrive for
that pane, sending back the keystrokes of the focused one. It handles layout, focus and chrome. It
touches no process and no pseudo-terminal, and it knows a pane only by its ID.

Between them is the **bus** — a single channel carrying a small, closed set of messages. The message
set is the load-bearing decision and is owned by
[`transport-contract.md`](./transport-contract.md); this document covers the structure around it.

```
  ┌──────────────────────── one process ─────────────────────────┐
  │                                                               │
  │  ┌──────────────┐      the bus       ┌────────────────────┐   │
  │  │ coordinator  │ ◄── contract msgs ──► │ UI (GPUI panes)  │   │
  │  │ portable-pty │                    └────────────────────┘   │
  │  └──┬────┬───┬──┘                                             │
  │  ┌──▼─┐┌─▼──┐┌▼───┐  one harness each                         │
  │  │PTY ││PTY ││PTY │                                           │
  │  └────┘└────┘└────┘                                           │
  └───────────────────────────────────────────────────────────────┘
```

## The rules

Five, in descending order of how expensive they are to break.

**1. Neither half may reach around the bus.** Not a direct call, not a shared mutable handle, not a
callback that skips the message set. The two halves share a process, which makes cheating easy and
invisible; the rule is what keeps the split real.

**2. The UI never assumes the pseudo-terminal is local.** No path, no process handle, no file
descriptor crosses into UI code. A pane is an ID plus a byte stream, and where the other end of that
stream lives is not the UI's business.

**3. The coordinator renders nothing.** It has no opinion about layout, colour, or what the bytes it
forwards mean. Terminal *emulation* — parsing those bytes into a screen — belongs to the UI's
terminal component.

**4. Every message carries a pane ID.** Output, input, resize, focus, exit. A message that cannot
name its pane is a message that will need reworking the moment a second pane exists.

**5. Terminal bytes stay opaque.** Only control messages are structured. Ubiq writes no VT parser
and no terminal state engine; it shuttles bytes between a pseudo-terminal and an emulator built for
exactly that problem.

## Why the split is drawn before it is needed

Both halves live in one process and the bus is an in-memory channel. Drawing the boundary anyway
costs a little indirection and buys two things.

**A detachable coordinator.** Split the halves into two processes and the bus becomes the same
message set serialised over a local socket. Because neither side speaks anything but the contract,
the change is confined to the channel: add framing and serialisation, swap the implementation.
Coordinator and UI logic go untouched — and that is what unlocks tmux-style detach and reattach,
where the window can die while the agents keep running.

**Remote harnesses.** A harness running on another host or in a container is structurally the same
problem as a terminal stream crossing a machine boundary. The coordinator stops assuming the
pseudo-terminal is local; the per-pane stream arrives over a network transport. The contract is
identical, because a pane was always a tagged bidirectional byte stream plus control messages.

Honouring rule 1 is what lets Ubiq go in-process → two processes → distributed by only ever changing
the transport beneath the contract.

## What each half is made of

| Concern | Where it lives | Notes |
|---|---|---|
| Window, panes, chrome, focus | `crates/ubiq/src/app.rs`, `crates/ubiq/src/ui/` | GPUI |
| Colour palette | `crates/ubiq/src/theme.rs` | Every colour goes through a token |
| Application and pane state | `crates/ubiq/src/state/` | State machines for pane and app lifecycle |
| The message set | `crates/ubiq/src/messages.rs` | The contract, serialisable by construction |
| Process and PTY lifecycle | `crates/ubiq/src/orchestrator.rs` | Spawn, supervise, reap |
| PTY streams and backpressure | `crates/ubiq/src/pty/` | `portable-pty` |
| Harness definitions | `crates/ubiq/src/agent.rs` | Seeded from the embedded library |
| In-process MCP surface | `crates/ubiq/src/mcp_server.rs` | Tools Ubiq exposes to the agents it hosts |

`crates/ubiq/src/main.rs` does nothing but open the window: install the GPUI component library, set
the theme, bind the quit action, and construct `AppState`. All real logic sits in the library root,
`crates/ubiq/src/lib.rs`.

## State ownership

The coordinator is the single source of truth. The UI holds a projection of it — enough to draw —
and never a fact the coordinator does not also hold. When the two disagree, the coordinator is
right, and the repair is a message, not a reach-around.

Inside the UI, `AppState` owns the pane map, the focused pane, and the layout mode, and mutates them
only through methods that end in a redraw request: `spawn_pane()`, `close_pane()`, `resize_pane()`,
`focus_pane()`.

## The dependency direction

```
main → app → ui → state → messages
                     ↑
      orchestrator → pty
```

Arrows point at what a module may name. `messages` sits at the bottom and depends on nothing, which
is what lets both halves share it without either becoming the other's dependency. Nothing under
`ui/` may name `orchestrator` or `pty`; nothing under `orchestrator` or `pty` may name `ui`.

Two dependencies come from outside the crate. `portable-pty` gives the coordinator cross-platform
pseudo-terminals. GPUI, with the `gpui-component` widget set, gives the UI its rendering. Neither is
allowed to leak across the bus: a GPUI type in a message, or a `portable-pty` handle in the UI, is
the same violation as breaking rule 1.

## The harness library

Ubiq does not know how to compose a harness run — which skills to inject, where a harness keeps its
configuration, how to launch it against a throwaway config directory. The `agent-manager` crate in
this workspace knows all of that, and Ubiq embeds it rather than reimplementing it. The boundary,
and which side owns which fact, is in [`agent-manager.md`](./agent-manager.md).

## Rationale

The two choices most likely to be re-argued:

**Why not let the UI hold the PTY directly?** It is fewer moving parts today and forecloses both
futures above. The moment a UI function takes a file descriptor, detach and remote harnesses stop
being a transport change and become a rewrite.

**Why an in-memory bus rather than a socket from the start?** A socket adds framing, serialisation
and a daemon lifecycle to manage before anything works. The contract is what makes the later socket
cheap; paying for the socket early buys nothing the contract does not buy on its own.

## Related docs

- [`transport-contract.md`](./transport-contract.md) — the message set, in full
- [`project-structure.md`](./project-structure.md) — where every file lives and what belongs where
- [`agent-manager.md`](./agent-manager.md) — the embedded library and the boundary with it
- [`decisions.md`](./decisions.md) — why these choices, and what they cost
