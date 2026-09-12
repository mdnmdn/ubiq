---
id: inbox-tui
title: Proposal — a terminal interface, on ratatui
kind: proposal
status: proposal
summary: A second interface crate drawn with ratatui that attaches to a host as an ordinary client over the transport the remote listener already speaks, covering the Control readings, a reduced IDE and the agents screen — three areas of the workbench and not the fourth, because a terminal already has git; what it may share with the GPUI interface, why a pane inside a pane is the expensive part, and the ownership rule that decides whether it can watch the window's panes or only its own.
read_when: you are deciding whether Ubiq gets a second front end, what a non-GPUI client may reuse, or how a terminal reaches a host on another machine
updated: 2026-09-12
depends_on: [tech-architecture, tech-transport, tech-structure, tech-decisions, feat-stats, feat-workbench, feat-chat, feat-panes, inbox-drone]
---

# Proposal — a terminal interface, on ratatui

Ubiq's interface is a GPU-drawn desktop window, and the machine a developer's agents run on is
often not the machine in front of them: an SSH session into a build box, a container, a rented GPU
node, a laptop reached from a phone. The host is reachable from any of those — `ubiq-app --serve`
binds a listener and speaks the whole contract over it — but the only thing that can dial it is
another desktop window on another desktop.

This proposes **a second interface, drawn with ratatui**, that attaches to a host as an ordinary
client and covers three of the workbench's areas: the Control readings, a reduced IDE — explorer,
search, a text viewer — and the agents screen, conversations and the panes under them. Git is not
one of them. A terminal already has git, and the Git screen is the one part of the window whose
terminal equivalent is a solved, better product someone else ships.

The argument is not that a terminal is a nicer window. It is that the architecture claims a UI is
replaceable, the claim has never been tested, and the test is worth more than it costs.

## 1. Where it stands

**The bus was built for many clients.** `crates/ubiq-proto/src/bus.rs` mints a `ClientId` per
attachment out of a process-wide counter and routes to `To::Client(id)` or `To::Everyone`; the
window is one client, and each remote session is another. `bus::detached` mints a client with no
hub behind it, for a socket pump to drive — that is the seam a second process attaches through.

**The wire exists and is in use.** `crates/ubiq-proto/src/wire.rs` frames messages as a four-byte
length prefix over MessagePack, chosen for self-description so two differently-built binaries can
disagree about field order and still talk. `crates/ubiq-host/src/remote.rs` binds a listener,
answers `GET /attach?token=` with a protocol upgrade and pumps frames into `Hub::connect`;
`crates/ubiq/src/app/remote_connect.rs` is the dialling half. Neither names a message family: a
remote connection is an ordinary client.

**The coordinator is keyed by client, not by window.** `crates/ubiq-host/src/coordinator.rs` holds
`watchers`, `owners`, `focused` and `conversation_owners` as maps from `ClientId`, and `client_gone`
reaps what one client owned. There is no `WindowId` anywhere in the host, and two attached clients
already have two focus states, neither more focused than the other.

**Both libraries are in the lockfile.** ratatui 0.30.2 and crossterm resolve through the harness
library's own front end, whose TUI is a stub in `crates/agent-manager/src/tui.rs` — the version was
already chosen against the workspace's `unicode-width` constraint. `alacritty_terminal` 0.25.1 sits
under `vendor/gpui-terminal/`, and the grid holder in `vendor/gpui-terminal/src/terminal.rs` names
no drawing crate at all.

**And the interface's state layer is renderer-agnostic by construction.** Across the 46 files of
`crates/ubiq/src/state/` there are nine mentions of the GPUI crate and no rendering context at all;
the exceptions are two widget-state fields and a handful of scroll offsets. That is not an accident
— it is `D17`'s split between state and the functions that draw it, and it is what makes this
proposal a rendering question rather than a rewrite.

## 2. The rule

**A second interface is a second client, and nothing else.**

It links the contract crate and never the host, exactly as the window does. It renders its own
chrome, owns its own layout, and asks the host for everything it knows. The consequences are worth
stating flat, because each one is a decision someone would otherwise make by accident:

- **No host change buys the reading screens.** Control, projects, the file tree, search and
  conversations are messages that exist, answered to whichever client asked.
- **It gets nothing from the window for free.** No theme, no widget, no layout. `D10` scopes the
  colour rule to one file in one crate, and a terminal has neither that file's palette nor its
  units.
- **What it may share is state, and only state.** The fold from a conversation delta into a
  transcript is a rule, not a picture, and two interfaces that disagree about it are a bug.
- **`just tui` is the check**, beside `just ui` and `just host`: the terminal interface's tree
  contains no drawing crate and no host crate.

## 3. What it covers, and what it drops

| Area | What the terminal interface draws | What it drops |
|---|---|---|
| Control | Both pages: the five host readings and the usage meter's rows. Tables and an em dash for what is unknown, which is what the screen already is | Nothing. This screen is text that happens to be drawn with a GPU |
| IDE | The explorer tree with its filter and its git letter badges, the file picker, project search with its result list, and a read-only text viewer with syntax colour | Every viewer that is not monospace text: Markdown and Mermaid previews, images, the diagram scene, the web panels. Editing, in the first cut |
| Agents | The agents screen as columns, the transcript with its tool blocks and folds, the composer, the permission prompts and the "needs you" strip, the lifecycle glyph, the context and token readouts, the delegate list | Drag-to-group, hover, inline links. Every gesture becomes a key |
| Panes | A real terminal per pane, with focus, resize and a leader key | Nothing, and this is the expensive one — see §5 |
| Git | — | All of it. Refs, history, the change lists, the diff, the lane engine. The terminal the interface runs in already has a git client, and it is better than the one Ubiq would draw in eighty columns |
| Orchestration, tasks, connectors, capture, settings | — | Out of the first three releases. A graph, a board and an OAuth flow each need their own argument |

The drops are the proposal's honest half. A terminal interface that tried to be the window would be
worse than the window at everything; the three areas above are chosen because each is already
text-shaped, and the fourth is dropped because its replacement is already installed.

## 4. What the two interfaces share

The state layer is the only candidate, and it is a real one: the explorer's tree merge, the
conversation fold, the search result model, the picker and the Control state hold about eighteen
thousand lines that name no drawing crate. Two exceptions matter — `crates/ubiq/src/state/editor.rs`
and `crates/ubiq/src/state/search.rs` each hold a widget's own state as the model, deliberately, and
neither travels.

**The recommendation is to share nothing on the first two phases and to extract exactly one module
on the third.** Sharing early means a crate move, a churn of imports across the window, and a
boundary argued before either side knows where it sits. Sharing never means a second implementation
of the conversation fold — two thousand lines of rules about what a delta does to a transcript,
which is precisely the kind of duplication that produces two products that disagree about what an
agent said.

So: phases 0 to 2 duplicate small state (a tree, a table, a query) and answer to the host for
everything else. Phase 3 moves `crates/ubiq/src/state/conversation.rs` into a crate both interfaces
depend on — a move, not a copy — and the window's own tests come with it. Anything else is shared
only when a second consumer has proved the shape.

## 5. A terminal inside a terminal

This is the part that decides whether the proposal is a month or a quarter.

**The emulator is the window's, not a new one.** `D1` says Ubiq integrates a terminal emulator
rather than writing one, and `D2` says a pane is a terminal. Both hold here. The grid holder under
`vendor/gpui-terminal/` wraps `alacritty_terminal` with no drawing crate in its logic, so the
terminal interface feeds bytes into the same emulator the window feeds and walks the resulting grid
into a cell buffer of its own. ratatui's buffer is a grid of cells with a foreground, a background
and attributes, which is the same shape alacritty's grid already has; the walk is mechanical.

The alternative is the ecosystem's own pseudo-terminal widget, which parses with a different VT
crate. It is rejected for one reason: a pane would then behave differently in the two interfaces,
and every emulator bug would have to be reproduced twice. The trick that widget demonstrates —
grid to buffer, once per frame — is worth taking; its parser is not.

**Four things then have to be got right, and each is a known failure mode.**

*The resize has to reach the harness.* A pane's rows and columns are the widget's inner area minus
its border, and every layout change sends a resize so the kernel signals the process. A pane that
redraws at a size its harness does not know is the classic corruption bug, and nesting doubles the
number of moments it can happen: the outer terminal is itself resizable.

*The keyboard belongs to the harness.* A harness consumes nearly every chord, so the interface
cannot claim any. It needs a leader key — the multiplexer answer — and `D45`'s closed set of
intercepted gestures becomes a closed set of leader-prefixed ones. Anything not behind the leader
goes to the pane as bytes.

*The outer terminal's capabilities are not the window's.* Colour depth, double-width characters,
bracketed paste, mouse reporting and the clipboard are all properties of a terminal Ubiq does not
control. The clipboard in particular has one answer over SSH — the escape sequence that asks the
outer terminal to set it — and no other.

*Redrawing costs.* Bytes arrive in chunks, and repainting the inner grid into the outer one on every
chunk is how a terminal multiplexer becomes slow. The frame loop coalesces: a dirty flag per pane, a
capped redraw rate, and the reader thread never blocked by the draw — the rule that a slow interface
must not stall the harness applies here exactly as it does in the window.

## 6. The panes it cannot see

The finding that most shapes the design: **pane output is routed to one client.** The reader thread
in `crates/ubiq-host/src/pty/mod.rs` forwards each chunk to the owning client's mailbox and stops
when nobody is listening; ownership is fixed at spawn; and the host keeps no scrollback, because
`D6` keeps the bytes opaque and nothing stores what it does not parse. A terminal interface that
dials a host the window is already driving therefore sees the projects, the readings, the files and
the conversations — and none of the window's panes.

Three ways out:

1. **Own its own panes.** The terminal interface spawns the panes it draws, and the window keeps
   drawing the panes it spawned. Nothing changes in the host.
2. **Broadcast pane output and keep a scrollback ring per pane in the host.** A real host change,
   a real memory cost, and it turns the opacity rule into a storage question.
3. **An explicit hand-over message** — one client asks to take, or to share, a pane another owns.

**Take the first for the first release, and file the third.** It is the same question
[`detached-panes`](./detached-panes-proposal.md) scoped out for the window: reattachment is
client-scoped today, and making it cross-client is one design, not two. The honest consequence is
that the first release is a terminal interface *to a host*, not a terminal view *of the window* —
which is what the SSH case actually wants.

## 7. How it is built, and how it starts

**One crate for the interface**, `crates/ubiq-tui/`, holding its own state, its own frame loop and
its own draw functions. It depends on the contract crate and on ratatui, and on nothing else of
Ubiq's.

**One composition root, still.** The binary crate is the only place that names both halves, and that
rule is worth keeping: it grows a second binary target behind a feature, with the GPUI interface an
optional dependency, so that a build with the window switched off produces a terminal binary with no
drawing crate anywhere in its tree. That is the build a headless server wants, and it is a manifest
change rather than a fifth crate.

**Two ways to start, and the second is the point.**

- *Owning a host.* The binary starts a coordinator exactly as the window's does and connects to it
  in-process. One host per process still holds.
- *Dialling one.* It attaches to a running host over the listener that exists, with an address and a
  token. This is the SSH case, the container case and the "my window is on the other desk" case.

The trap between them is the single-instance lock in `crates/ubiq-app/src/handoff.rs`: its socket is
named from the hash of the running executable's path, so two binaries never hand off to each other,
and two coordinators over one config root would race the catalogue. The rule that avoids it:
**a terminal interface that finds a host running against its config root dials it, and starts one
only when none does.** Finding one needs a way to ask — the handoff socket is the obvious place to
answer "who serves, and where" — and that is a gap, not a design.

## 8. Colour, in a place with sixteen of them

`D10` puts every colour behind a token, in one file, in the window's crate. A terminal interface
cannot import that file and must not invent literals of its own, so it carries **the same token
names against a different value table**: a true-colour value per token where the terminal says it
can, and an indexed fallback where it cannot. The names being identical is what makes the two
interfaces reviewable side by side and what stops the terminal one from drifting into its own
vocabulary of greys.

Two consequences: the palette registry's slugs are worth mirroring, so a user's chosen theme means
something in both; and the terminal interface's theme file is the second file in the tree allowed to
name a colour, which is a rule change and therefore a decision row.

## 9. Phases

Each phase ships something usable and proves one risky thing.

| # | Ships | Proves |
|---|---|---|
| 0 | Attach — in-process and dialled — plus the Control readings, on one screen | The client, the dial, the frame loop and the token. About a week |
| 1 | Projects, the explorer, search, and a read-only viewer with syntax colour | The file families under a terminal's keyboard model, and the first real layout |
| 2 | Conversations: transcript, tool blocks, composer, permission prompts, delegates | That an agent can be run and answered from a terminal — the release that justifies the rest |
| 3 | Panes: the emulator, the leader key, resize, the redraw budget | The expensive one. Nesting, at the frame rate a harness needs |
| 4 | Remote polish: discovery, reconnect, TLS by default on a dialled host | That the SSH case is a product rather than a demo |

Phase 2 is the one to judge the proposal by. If a developer will run an agent from a terminal
interface over SSH and watch it work, phases 3 and 4 pay for themselves; if they will not, phase 3
should never be built.

## 10. What it costs

**Every message becomes two front ends' work.** The contract grows, and a second consumer means a
second place to add an arm. The mitigation is that the terminal interface is allowed to be behind:
it renders the families it knows and ignores the rest, and no phase of it blocks a change to the
window.

**A half-built interface nobody uses is worse than none.** It holds a crate, a check, a share of
every review, and a claim on the reader's attention in this library. The stopping rule is written
above: judge at the end of phase 2, and delete the crate rather than carry it.

**It competes for the same need as the drone.** [The drone](./backlog/remote-drone-proposal.md)
brings a remote machine's files and shells to the local window; this brings the local window's
capabilities to the remote machine's terminal. They are not substitutes — one needs a desktop and
the other does not — but a person with an SSH session and a laptop can be served by either, and
building both first is how neither gets finished.

**And the product's own boundary holds.** One person, one machine, one window — a second interface
for the same person on the same host is inside that line; a shared terminal server for a team is
not, and no phase here moves toward one.

## 11. What this would record

If accepted, one decision row and four backlog rows:

- **`D113` — a second interface is a client, not a mode.** The terminal interface is its own crate
  with its own token table; the window's crate stays GPUI's, and the shared layer is state or
  nothing. Cost: a second place to add a screen, and a second colour table to keep honest.
- **`G250`** — pane output routes to one client and the host holds no scrollback, so a second
  interface cannot watch the window's panes; the hand-over message is unspecified.
- **`G251`** — a running host has no discovery: a terminal interface cannot tell whether to dial one
  or start one.
- **`G252`** — the conversation fold lives in the window's crate; sharing it with a second interface
  is a crate move that has not been made.
- **`G253`** — the dialled transport is token-only and plaintext by default, which a terminal
  interface over an untrusted network makes routine rather than exceptional.

## Related docs

- [`architecture.md`](../tech/architecture.md) — the two halves, the crate boundary, and the
  detachable coordinator this proposal is the first consumer of
- [`transport-contract.md`](../tech/transport-contract.md) — the message families a second client
  speaks, and the framing under them
- [`workbench.md`](../features/workbench.md) — the screens this proposal takes three of
- [`chat.md`](../features/chat.md) — what a conversation surface owes its reader
- [`stats.md`](../features/stats.md) — the readings phase 0 draws
- [`panes-and-terminals.md`](../features/panes-and-terminals.md) — focus, resize and the rules a
  nested pane inherits
