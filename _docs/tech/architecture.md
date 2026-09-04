---
id: tech-architecture
title: Architecture
kind: tech
status: current
summary: The two halves — coordinator and UI — the single bus between them, the rules neither may break, and why the split is drawn before it is needed.
read_when: you are about to add a capability that crosses the UI/coordinator line, or you want to know why the code is shaped this way
updated: 2026-09-04
verified: 2026-09-04
code_anchors: [crates/ubiq/src/lib.rs, crates/ubiq-app/src/main.rs, crates/ubiq/src/app/mod.rs, crates/ubiq/src/app/boot.rs, crates/ubiq/src/app/wire.rs, crates/ubiq-proto/src/bus.rs, crates/ubiq-host/src/coordinator.rs, crates/ubiq-proto/src/log.rs, crates/ubiq-host/src/lib.rs, crates/ubiq-proto/src/lib.rs, crates/ubiq-host/src/work/mod.rs, crates/ubiq-host/src/files/mod.rs, crates/ubiq-host/src/files/diff.rs, crates/ubiq-host/src/git/mod.rs, crates/ubiq-host/src/git/observe.rs, crates/ubiq-host/src/projects.rs, crates/ubiq-host/src/settings.rs, crates/ubiq-host/src/store/mod.rs, crates/ubiq-host/src/store/file.rs, crates/ubiq-host/src/store/memory.rs, crates/ubiq-host/src/watch/mod.rs, crates/ubiq/src/web_export/mod.rs]
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

Six, in descending order of how expensive they are to break.

**1. Neither half may reach around the bus.** Not a direct call, not a shared mutable handle, not a
callback that skips the message set. The two halves share a process, which makes cheating easy and
invisible; the rule is what keeps the split real.

**2. The UI never assumes the pseudo-terminal is local.** No path, no process handle, no file
descriptor crosses into UI code. A pane is an ID plus a byte stream, and where the other end of that
stream lives is not the UI's business. The workarea in rule 6 is the one path the interface is
given, and it is given rather than composed for exactly this reason. A file dropped from outside
every open project is the second exception: the operating system hands the interface an absolute
path with no host round trip available, and it is given rather than composed there too — the
interface reads it with `std::fs` to build a read-only guest tab, and never resolves, writes to, or
sends that path anywhere. `D54` records the decision and its cost. The web-export server
(`crates/ubiq/src/web_export/`) is a third instance of the same reasoning at a larger scale: it reads
a whole project's tree with `std::fs` and the `ignore` crate, off its own thread, using the
project's path from the same `ProjectSnapshot` rather than a path it composed. `D55` records it.

**3. The coordinator renders nothing.** It has no opinion about layout, colour, or what the bytes it
forwards mean. Terminal *emulation* — parsing those bytes into a screen — belongs to the UI's
terminal component.

**4. Every message carries a pane ID.** Output, input, resize, focus, exit. A message that cannot
name its pane is a message that will need reworking the moment a second pane exists.

**5. Terminal bytes stay opaque.** Only control messages are structured. Ubiq writes no VT parser
and no terminal state engine; it shuttles bytes between a pseudo-terminal and an emulator built for
exactly that problem.

**6. The host reserves the interface's workarea and never reads inside it.** Every project's
`ProjectSnapshot` carries a `workarea` — one directory, under that project's own folder in the
config root, that belongs to the interface. The host makes it and names it, and that is the end of
the host's interest: nothing on this side lists it, reads it or writes to it. The interface uses it
directly, off the bus, for caches and for anything else that is its business and not the project's,
and what it keeps there is **disposable** — deleting the directory loses a cache and nothing else,
because anything worth keeping goes over the bus as a preference blob. It is **not the project's
folder**, so nothing the interface writes lands in the user's repository. And the interface **never
composes the path** out of `HostInfo.config_root`; it uses the string it was handed, which is what
makes a host on another machine a change of value rather than a change of code. `projects.rs` is
where it is reserved; [`transport-contract.md`](./transport-contract.md) owns the field.

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
| The config root, and every store under it | `crates/ubiq-host/src/config.rs`, `store/` | Movable by flag, environment or a bootstrap `ubiq.toml`. Four traits: the catalogue, a project's tasks, the interface's opaque view state, and settings (Ui opaque, Host parsed) |
| The project catalogue | `crates/ubiq-host/src/projects.rs` | The host acts on it; the interface holds a projection |
| A project's tasks, and the sessions and agents over them | `crates/ubiq-host/src/work/` | Tasks are the user's data, written down per project; sessions and agents are the host's mocks, minted per project and never written |
| Window, panes, chrome, focus | `crates/ubiq/src/app/`, `crates/ubiq/src/ui/` | GPUI. `AppState` is the only view; `ui/` renders it |
| Colour palette | `crates/ubiq/src/theme.rs` | Every colour goes through a token |
| Application and pane state | `crates/ubiq/src/state/` | Pane and app lifecycle, plus the workbench, explorer, editor, chat, agents, orchestration and board state, and the projection of a project's work. A window holds one tree, one set of open files and one projection of the work per project |
| The message set | `crates/ubiq-proto/src/messages.rs` | The contract, serialisable by construction |
| The bus, and a pane's byte streams | `crates/ubiq-proto/src/bus.rs` | The channel pair, and the `Read`/`Write` ends the emulator gets |
| Process and PTY lifecycle | `crates/ubiq-host/src/coordinator.rs` | Spawn, supervise, reap. One coordinator thread, started by the binary before the first window |
| PTY streams and backpressure | `crates/ubiq-host/src/pty/` | `portable-pty` |
| A project's folder, its files, a save and a diff | `crates/ubiq-host/src/files/` | The walk, the read, the atomic write and one file's comparison with version control, on a worker thread of their own so no listing blocks the coordinator |
| A project's repository | `crates/ubiq-host/src/git/` | The overview the status bar reads and the working-tree map the explorer's badges read, on a worker thread of their own so a cold status does not stall every pane |
| What changed in a project's folder | `crates/ubiq-host/src/watch/` | One recursive `notify` watch and one debounce thread per open project, per window. The only thing in the host that speaks without being asked |
| Terminal emulation | `vendor/gpui-terminal/` | Vendored third-party component; the UI's, never the coordinator's |
| Harness definitions | `crates/ubiq-host/src/agent.rs` | Seeded from the embedded library |
| In-process MCP surface | `crates/ubiq-host/src/mcp_server.rs` | Tools Ubiq exposes to the agents it hosts |
| Diagnostics from every subsystem | `crates/ubiq-proto/src/log.rs` | The one sink both halves write to, and the console reads |

**Version control is read in two places, both in the host, both through `git2`.** A one-file diff
lives in `crates/ubiq-host/src/files/diff.rs` and rides the files worker: the host opens the
repository, takes the blob at `HEAD` or the one staged in the index, works the hunks and their line
numbers out, and sends them — which is what keeps a diff library out of the interface, on the
discipline that keeps a VT parser out of the host. The overview and the working-tree map live in
`crates/ubiq-host/src/git/` on a worker of their own, because a cold status on a large repository is
seconds and seconds on the files worker would stall every expand behind it. Neither half writes
into a repository: status walks with the index-stat refresh turned off, and the git directory is
inside the project's folder, so `D30` covers it.

Four behaviours follow, and each is a thing the interface must not have to guess. The repository is
looked for **upward from the project's root**, so a project that is a folder inside one is compared
against that repository; a project with none above it is `Refused` in the words the transport
contract fixes, rather than answered with an empty diff that would draw as a file with no changes. A
file version control has never seen — untracked, ignored, or any file in a repository whose first
commit has not been made — has no blob on either base and comes back **wholly added**, which is what
the working tree actually adds. Both sides are compared **as they are stored**, without git's clean
and smudge filters, because running those means running programs configured by a folder the user
merely opened. And the comparison carries the family's ceilings — two megabytes a side, four hundred
hunks, ten thousand rows — past which it comes back `truncated` rather than as a change smaller than
the one on disk.

**One thing in the host speaks without being asked: the filesystem watch.** `watch::start` takes a
project's root, the merged excludes and a mailbox, and holds a recursive `notify` watch plus a
debounce thread; the coordinator keys one per `(client, project)` and drops it when that window
opens another project or leaves, so a dropped handle is the whole of stopping a watch. What crosses
is `ProjectFilesChanged` — project-relative paths and a flag for the git directory, never content
and never an absolute path, on the same rule a search hit follows. A watch that will not start is
logged and the project simply has none.

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
write to it, the window's console reads it, and it is process-wide rather than per window. The
workarea of rule 6 is the other, and neither weakens rule 1: the sink is written by both halves and
read by both, and the workarea is written by neither half but one — the host says the name once and
looks no further.

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

Inside the UI, `AppState` owns the panes, the focused pane, and the dock they are panels in, and
mutates them only through methods that end in a redraw request: `spawn_pane()`, `close_pane()`, `resize_pane()`,
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
