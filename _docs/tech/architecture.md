---
id: tech-architecture
title: Architecture
kind: tech
status: current
summary: The two halves — coordinator and UI — the single bus between them, the rules neither may break, and why the split is drawn before it is needed.
read_when: you are about to add a capability that crosses the UI/coordinator line, or you want to know why the code is shaped this way
updated: 2026-09-01
verified: 2026-09-01
code_anchors: [crates/ubiq/src/lib.rs, crates/ubiq-app/src/main.rs, crates/ubiq/src/app.rs, crates/ubiq-proto/src/bus.rs, crates/ubiq-host/src/coordinator.rs, crates/ubiq-proto/src/log.rs, crates/ubiq-host/src/lib.rs, crates/ubiq-proto/src/lib.rs]
review_cycle: quarterly
---

# Architecture

## The shape

Ubiq has two halves and one channel between them, and the halves are **crates**, so the boundary is
a compile error rather than a convention.

| Crate | Holds | Depends on |
|---|---|---|
| `crates/ubiq-proto/` | The message set, the ids, the bus, the log sink | Nothing that draws, nothing that touches disk |
| `crates/ubiq-host/` | The coordinator, pseudo-terminals, the project catalogue and its stores | The protocol crate |
| `crates/ubiq/` | Windows, panes, chrome, theme, the projection | The protocol crate — **not** the host crate |
| `crates/ubiq-app/` | The binary | All three |

Only the binary names both halves, and it names the host once: to start it and hand the interface
the other end of the bus. The interface cannot reach around the bus because the host's types are
not in its dependency graph, and `just host` and `just ui` check that mechanically — the first that
no drawing crate reaches the host's tree, the second that the host never reaches the interface's.

A `[[bin]]` inside `crates/ubiq` could not express this: a binary shares its package's
`[dependencies]`, so naming the host there would put it in the library's graph too. That is why the
binary is a crate of its own — see `D27`.

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

`crates/ubiq-proto/src/bus.rs` is that channel in code. `pair()` opens two unbounded queues of messages,
one per direction, and a window keeps one end while the coordinator thread keeps the other. The
same module holds the two byte-stream endpoints a pane's emulator is handed in place of a
pseudo-terminal: a `Write` whose writes leave as input messages, and a blocking `Read` fed by the
output messages the window routes to that pane. Unbounded is deliberate — a window that falls
behind must never stall the reader that is draining a harness.

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
| The config root, and every store under it | `crates/ubiq-host/src/config.rs`, `store/` | Movable by flag, environment or a bootstrap `ubiq.toml` |
| The project catalogue | `crates/ubiq-host/src/projects.rs` | The host acts on it; the interface holds a projection |
| Window, panes, chrome, focus | `crates/ubiq/src/app.rs`, `crates/ubiq/src/ui/` | GPUI. `AppState` is the only view; `ui/` renders it |
| Colour palette | `crates/ubiq/src/theme.rs` | Every colour goes through a token |
| Application and pane state | `crates/ubiq/src/state/` | Pane and app lifecycle, plus the workbench, explorer, editor and chat state. A window holds one tree and one set of open files per project |
| The message set | `crates/ubiq-proto/src/messages.rs` | The contract, serialisable by construction |
| The bus, and a pane's byte streams | `crates/ubiq-proto/src/bus.rs` | The channel pair, and the `Read`/`Write` ends the emulator gets |
| Process and PTY lifecycle | `crates/ubiq-host/src/coordinator.rs` | Spawn, supervise, reap. One coordinator thread, started by the binary before the first window |
| PTY streams and backpressure | `crates/ubiq-host/src/pty/` | `portable-pty` |
| A project's folder, its files and a save | `crates/ubiq-host/src/files/` | The walk, the read and the atomic write, on a worker thread of their own so no listing blocks the coordinator |
| Terminal emulation | `vendor/gpui-terminal/` | Vendored third-party component; the UI's, never the coordinator's |
| Harness definitions | `crates/ubiq-host/src/agent.rs` | Seeded from the embedded library |
| In-process MCP surface | `crates/ubiq-host/src/mcp_server.rs` | Tools Ubiq exposes to the agents it hosts |
| Diagnostics from every subsystem | `crates/ubiq-proto/src/log.rs` | The one sink both halves write to, and the console reads |

`crates/ubiq-app/src/main.rs` does nothing but start the application: resolve the config root, start
the one host, install the GPUI component library, set the palette, bind the quit action and the
interface's own, and ask for the first window. Opening a window is
`app::open_project_window`, the single place one is created, so the first window and "open in a new
window" cannot drift apart. `main.rs` consumes the crate as a library rather than redeclaring its
modules, so the tree is compiled once. All real logic sits in the library root,
`crates/ubiq/src/lib.rs`.

**A window is one `AppState`.** Several may be open, each pointed at its own project. They share the
palette, the window registry and the bus's hub, all of which are process-wide, and nothing else —
so any state that ought to be global needs a home outside `AppState` before it can be shared.

**There is one host per process**, started by the binary before the first window and outliving every
one of them. A window attaches to it and gets a `Client`; the host reads every client through one
`HostEnd` and addresses each answer to somebody. Pane-family messages route to the window that owns
the pane, recorded when it was spawned; project-family messages are broadcast, which is what makes
every window's picker agree by construction. Attaching and leaving are transport facts rather than
things either half says, so they are `FromClient` variants and not messages.

The catalogue is why it is singular: two hosts would race the store file and disagree about what
exists. It also means nothing drops when a window closes, so the host reaps that window's
pseudo-terminals deliberately — without that, every closed window would leave a live harness.

## Diagnostics

One thing crosses the line without the bus: the log sink in `crates/ubiq-proto/src/log.rs`. Both halves
write to it, the window's console reads it, and it is process-wide rather than per window.

It is not an exception to rule 1, because nothing about it is communication. Records travel one way,
a producer never reads, and no record carries a pane's state, a path or a handle — neither half
learns anything from the sink, and removing it would change nothing either half does. What it buys
is a subsystem that logs with `tracing::info!` and no plumbing, including the crates that have never
heard of Ubiq. The trade, and the shape a detached coordinator forces, are `D24` and a row in
[`../backlog.md`](../backlog.md); what the sink holds and what the console does with it is
[`../features/logs.md`](../features/logs.md).

## State ownership

The coordinator is the single source of truth. The UI holds a projection of it — enough to draw —
and never a fact the coordinator does not also hold. When the two disagree, the coordinator is
right, and the repair is a message, not a reach-around.

Inside the UI, `AppState` owns the panes, the focused pane, and the layout mode, and mutates them
only through methods that end in a redraw request: `spawn_pane()`, `close_pane()`, `resize_pane()`,
`focus_pane()`. It owns the workbench's own state on the same terms. A pane is drawn when the
coordinator answers with the workspace it started, not when the UI asked for one — asking is
`spawn_pane()`, and the answer arrives, with everything else the coordinator says, at `receive()`,
through a task draining the bus.

## The dependency direction

```
main → app → ui → state → messages
        ↘                 ↑
          bus ────────────┤
        ↗                 │
      orchestrator → pty ─┘
```

Arrows point at what a module may name. `messages` sits at the bottom and depends on nothing, which
is what lets both halves share it without either becoming the other's dependency; `bus` sits just
above it, naming the contract and nothing else, which is why both halves may name `bus`. Nothing
under `ui/` may name `orchestrator` or `pty`; nothing under `orchestrator` or `pty` may name `ui`.

Three dependencies come from outside the crate. `portable-pty` gives the coordinator cross-platform
pseudo-terminals. GPUI, with the `gpui-component` widget set, gives the UI its rendering, and
`gpui-terminal` — vendored in this workspace — gives it the emulator a pane is drawn by. None may
leak across the bus: a GPUI type in a message, or a `portable-pty` handle in the UI, is the same
violation as breaking rule 1.

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
