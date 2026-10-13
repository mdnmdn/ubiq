---
id: inbox-tui
title: Proposal — a terminal interface, on ratatui
kind: proposal
status: proposal
summary: A second interface crate drawn with ratatui that attaches to a host as an ordinary client over the transport the remote listener already speaks, covering the Control readings, a reduced IDE and the agents screen — three areas of the workbench and not the fourth, because a terminal already has git; what it may share with the GPUI interface, why a pane inside a pane is the expensive part, the ownership rule that decides whether it can watch the window's panes or only its own, the host record that lets either interface attach to whichever one is running, and the three shapes the application builds in.
read_when: you are deciding whether Ubiq gets a second front end, what a non-GPUI client may reuse, how a front end finds a host that is already running, or how a terminal reaches a host on another machine
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
search, a text viewer — and the agents screen, conversations and the panes under them. The host it
attaches to is whichever one is running: the window's, another terminal's, one on a machine reached
over SSH, or one it starts itself because none was. Git is not one of the areas. A terminal already has git, and the Git screen is the one part of the window whose
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
- **It attaches before it starts.** A front end that finds a host already running against its config
  root joins that one instead of raising a second, whichever interface raised it — and the rule runs
  in both directions, so the window obeys it too (§8).
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

## 6. The panes it cannot see, and the conversations either

The finding that most shapes the design: **everything the host streams, it streams to one client.**
The reader thread in `crates/ubiq-host/src/pty/mod.rs` forwards each chunk to the owning client's
mailbox and stops when nobody is listening; ownership is fixed at spawn; and the host keeps no
scrollback, because `D6` keeps the bytes opaque and nothing stores what it does not parse. A
conversation is the same shape for a different reason: its updates are addressed to the client that
started it, and the pump carries a sequence number while the content passes straight through. What
broadcasts is what the host *owns* — the catalogue, the agent roster, the readings.

So a terminal interface that dials a host the window is already driving sees the projects, the
readings, the files and which agents exist — and neither the window's panes nor the window's
transcripts.

Three ways out:

1. **Own its own.** The terminal interface spawns the panes and conversations it draws, and the
   window keeps the ones it spawned. Nothing changes in the host.
2. **Fan the streams out and keep enough history to join one.** A real host change, a real memory
   cost, and it turns the opacity rule into a storage question.
3. **An explicit hand-over** — one client asks to take, or to share, what another owns.

**Take the first for the first release.** The second and third are one design rather than two, and
they are the subject of [a proposal of their own](./multi-attach-proposal.md), which is also where
the question [`detached-panes`](./detached-panes-proposal.md) scoped out for the window lands: a
reattachment that crosses clients. The honest consequence is that the first release is a terminal
interface *to a host*, not a terminal view *of the window* — which is what the SSH case actually
wants.

## 7. How it is built: three shapes

**One crate for the interface**, `crates/ubiq-tui/`, holding its own state, its own frame loop and
its own draw functions. It depends on the contract crate and on ratatui, and on nothing else of
Ubiq's.

**One composition root, still.** The binary crate stays the only place that names both halves. It
grows a second binary target and two features, each interface an optional dependency behind its own,
and every target naming its feature in `required-features` — so a narrowed build simply produces one
fewer binary rather than failing to compile.

| Build | Features | Produces | For |
|---|---|---|---|
| Both | the default | `ubiq` and `ubiq-tui` | The desktop install: a window, and a terminal that attaches to the same host |
| Window only | `--no-default-features --features gui` | `ubiq` | A packaged desktop application that has no business shipping a second command |
| Terminal only | `--no-default-features --features tui` | `ubiq-tui` | A server, a container, a build box: no drawing crate anywhere in the tree, no GPU, no font stack, no display |

The terminal-only build is the one that has to be defended, because it is the reason the features
exist rather than a runtime flag: a machine with no display must be able to compile and run Ubiq
without GPUI in its dependency graph at all. On-device assistance is a default feature and drops out
of a narrowed build alongside the rest; the host answers from its stub backend, which every call
site already reads correctly.

**`just tui` is the check that keeps it true**, mirroring `just ui`: the terminal interface names the
contract and neither the host nor the window's crate. `just check` gains the two narrowed builds, and
the terminal-only one carries the assertion that matters — no drawing crate in the tree of a binary
meant for a machine with none.

## 8. Attach, or start

**A host belongs to a config root, and a front end asks the root before it starts one.**

Two mechanisms come close to answering that today and neither does. The single-instance lock in
`crates/ubiq-app/src/handoff.rs` is per *application*: its socket is named from the hash of the
running executable, so a relaunch of the window hands its paths to the running window and exits,
while a differently-named binary never meets it at all — and it is Unix-only. A served run
advertises to the network, and nothing local is told it exists.

**The host record.** Whichever process owns a coordinator binds the listener that exists on loopback
with a kernel-assigned port — `remote::serve` hands back the address and the token it minted — and
writes a record beside the catalogue in the config root: address, token, the owner's process id, and
which interface owns it. It is written with the same atomic helper the catalogue uses, readable only
by its owner, and removed when the process ends.

Every front end then starts the same way:

1. **Read the record.** None → own a host, write one, draw.
2. **Dial what it names.** The dial is the liveness test, exactly as a handoff socket left by a crash
   is proved stale by connecting to it: a record whose address refuses is removed, and the front end
   owns a host instead.
3. **Attach and draw.** For the window this is the path it has rather than a new one — the bus
   wrapper in `crates/ubiq/src/app/hosts.rs` already multiplexes a local client and dialled ones.

**It runs in both directions.** A terminal started while a window runs attaches to the window's host:
the same projects, the same agent roster, the same readings — and its own panes and its own
conversations, because both are streamed to one owner (§6). A window started while a terminal owns
the host attaches to that one. Which interface got there first stops being something anyone has to
know.

**And the other choice stays available.** `--own` starts a host regardless, and refuses unless
`--config-root` names a root nothing serves: two coordinators over one root do not corrupt it — every
store write is temp, fsync, rename — but they do lose each other's updates, because each rewrites the
whole catalogue from its own memory. `--connect <addr>` with a token skips the record and dials what
it is told, which is the SSH case, where the record is on the far machine and is not the near one's
to read.

**The terminal interface claims no single-instance lock.** Handoff exists so that a second launch of a
windowed application is not a second window; a second terminal in a second tmux pane is a second
client on purpose. It reads the record, attaches, and never binds that socket.

The cost is a loopback listener and a token file for every running Ubiq, where a listener exists
today only when asked for. Loopback only, a port the kernel picks, a file inside a root that already
holds the catalogue and the account references — and a preference for anyone who wants no listening
socket at all, which costs them the duality and nothing else.

## 9. Colour, in a place with sixteen of them

`D10` puts every colour behind a token, in one file, in the window's crate. A terminal interface
cannot import that file and must not invent literals of its own, so it carries **the same token
names against a different value table**: a true-colour value per token where the terminal says it
can, and an indexed fallback where it cannot. The names being identical is what makes the two
interfaces reviewable side by side and what stops the terminal one from drifting into its own
vocabulary of greys.

Two consequences: the palette registry's slugs are worth mirroring, so a user's chosen theme means
something in both; and the terminal interface's theme file is the second file in the tree allowed to
name a colour, which is a rule change and therefore a decision row.

## 10. Phases

Each phase ships something usable and proves one risky thing.

| # | Ships | Proves |
|---|---|---|
| 0 | Attach — owned, recorded and dialled — plus the Control readings, on one screen | The client, the record, the frame loop and the token. About a week |
| 1 | Projects, the explorer, search, and a read-only viewer with syntax colour | The file families under a terminal's keyboard model, and the first real layout |
| 2 | Conversations: transcript, tool blocks, composer, permission prompts, delegates | That an agent can be run and answered from a terminal — the release that justifies the rest |
| 3 | Panes: the emulator, the leader key, resize, the redraw budget | The expensive one. Nesting, at the frame rate a harness needs |
| 4 | Remote polish: reconnect, TLS by default on a dialled host, the terminal-only build in CI | That the SSH case is a product rather than a demo |

Phase 0 carries the record from §8, and the window is the second half of it: teaching the window to
read the record and attach is a change to one crate's boot, and it is what makes the duality real in
both directions rather than only from the terminal.

Phase 2 is the one to judge the proposal by. If a developer will run an agent from a terminal
interface over SSH and watch it work, phases 3 and 4 pay for themselves; if they will not, phase 3
should never be built.

## 11. What it costs

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

## 12. What this would record

If accepted, two decision rows and five backlog rows:

- **`D113` — a second interface is a client, not a mode.** The terminal interface is its own crate
  with its own token table and its own binary target, so the application builds in three shapes
  rather than switching interfaces at runtime; the window's crate stays GPUI's, and the shared layer
  is state or nothing. Cost: a second place to add a screen, a second colour table to keep honest,
  and a feature matrix that has to be built in CI or it rots.
- **`D114` — a host advertises itself in its config root, and a front end attaches before it starts
  one.** Owning a coordinator means binding loopback and writing a record; finding a live record
  means attaching to it, whichever interface wrote it. Cost: a listening socket and a token file for
  every running Ubiq, and one more piece of state in the config root that a crash can leave behind.
- **`G250`** — every streamed resource routes to one client and the host keeps no history of either
  kind, so a second interface can watch neither the window's panes nor its transcripts; the fan-out
  and the hand-over are unspecified, and `inbox-multi-attach` is where they are argued.
- **`G251`** — nothing enforces one host per config root. `--own` against a served root is fenced by
  a check at boot and by nothing at all afterwards, and two coordinators that reach one catalogue
  lose each other's updates rather than failing.
- **`G252`** — the conversation fold lives in the window's crate; sharing it with a second interface
  is a crate move that has not been made.
- **`G253`** — the dialled transport is token-only and plaintext by default, and the host record puts
  a token in a file, so anything that can read the config root can attach as a client. Over an
  untrusted network a terminal interface makes that routine rather than exceptional.
- **`G254`** — "one application per config root" and "one host per config root" are two mechanisms
  answering nearly one question: the handoff socket is per executable and Unix-only, the record is
  per root and everywhere. Whether the first should be folded into the second is open.

## Related docs

- [`architecture.md`](../tech/architecture.md) — the two halves, the crate boundary, and the
  detachable coordinator this proposal is the first consumer of
- [`transport-contract.md`](../tech/transport-contract.md) — the message families a second client
  speaks, and the framing under them
- [`workbench.md`](../features/workbench.md) — the screens this proposal takes three of
- [`chat.md`](../features/chat.md) — what a conversation surface owes its reader
- [`stats.md`](../features/stats.md) — the readings phase 0 draws
- [`multi-attach-proposal.md`](./multi-attach-proposal.md) — what a host owes several interfaces at
  once, and the lease that decides who may type
- [`panes-and-terminals.md`](../features/panes-and-terminals.md) — focus, resize and the rules a
  nested pane inherits
