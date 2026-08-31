---
id: tech-decisions
title: Decision register
kind: tech
status: current
summary: One entry per structural decision — what was chosen, why, and what it costs — cited as `Dnn` across this library.
read_when: you are about to argue with a rule, reverse a design choice, or make one a reasonable person might later reverse
updated: 2026-08-31
verified: 2026-08-31
depends_on: [tech-architecture]
review_cycle: quarterly
---

# Decision register

One entry per structural choice. Every entry names its cost, because a decision recorded without its
cost reads as a law rather than a trade, and the next person cannot tell whether the trade still
holds.

This is the one place in the library where a document may name what a choice replaced: rationale
stays true even when the alternative is gone. Cite an entry as `D7`, never by its row position.

**Appending a row is part of making the decision**, not follow-up work — see `_docs/_meta/authoring.md`.

---

### D1 — Ubiq integrates a terminal emulator rather than writing one

Harnesses use the full ANSI vocabulary, the alternate screen and absolute cursor addressing. Writing
a VT parser and terminal state engine is a project in itself, and a solved one.

**Cost:** the emulator's fidelity, performance and bugs are the product's. Working around one means
working around someone else's design.

### D2 — A pane is a terminal, not a text buffer

Every pane owns a real terminal emulator and every harness runs under a real pseudo-terminal. The
alternative — rendering agent output as a scrolling log — was rejected because the agent is driving
a screen, not producing a log.

**Cost:** far more machinery than a text view, and every pane carries the memory of a terminal.

### D3 — One transport contract, and neither half may bypass it

The UI and the coordinator communicate only through a closed message set. No direct calls, no shared
mutable handles, no callbacks that skip the bus.

**Cost:** indirection on every interaction, including ones that share a process and could be a
function call. This is the single most expensive rule to hold and the one that buys the most.

### D4 — The two halves share a process, over an in-memory bus

The contract is implemented as an in-memory channel rather than a socket. A socket adds framing,
serialisation and a daemon lifecycle before anything works.

**Cost:** the discipline of `D3` is unenforced by the compiler. Reaching around the bus compiles
cleanly, which is why the rule has to be written down.

### D5 — Every message carries a pane ID

Including in the single-pane case, where it is redundant.

**Cost:** a field nothing reads until the second pane exists. The alternative was reworking every
message once multiplexing arrived.

### D6 — Terminal bytes stay opaque

Only control messages are structured. Neither half parses harness output.

**Cost:** the coordinator cannot make decisions based on what a harness is doing. Anything that
needs to — detecting an idle agent, reading a model name — has to come from a structured channel
rather than the terminal stream.

### D7 — GPUI, replacing the Tauri and web-view frontend

The application is a Rust GPUI program. It supersedes a design where the UI was JavaScript and
`xterm.js` inside a Tauri web view.

**Why:** several full-refresh terminals under a stream of escape sequences is the load the UI has to
survive, and a GPU-drawn native tree carries it without a serialisation boundary in the render path.
It also collapses two languages, a bundler and a second runtime into one toolchain.

**Cost:** GPUI is consumed from git rather than a published crate, so the build tracks an upstream
that moves. First builds are long, and the component ecosystem is small compared to the web's.

### D8 — A Cargo workspace, with the harness library as a sibling crate

`agent-manager` is a crate beside the application, with its own documentation, tests and CLI, rather
than a module inside it.

**Cost:** a boundary to maintain and one more manifest. It buys a library that other tools can
embed, and that builds with no UI dependency at all.

### D9 — Ubiq embeds the harness library rather than shelling out to `am`

The application constructs a run programmatically and launches it in-process.

**Why:** a subprocess boundary would cost a serialisation round trip per run, make in-process MCP
impossible, and turn structured errors into parsed text.

**Cost:** the application is coupled to the library's Rust API, so a breaking change there is a
change here.

### D10 — Every colour goes through a theme token

No literal colour outside `crates/ubiq/src/theme.rs`, and every token has a value in both palettes.

**Cost:** a thread-local read per colour, and a token to name before any new shade can be used.

### D11 — A session groups workspaces, and the user attaches to it

Sessions are named units of work that outlive the agents inside them; the user attaches and detaches
rather than opening and closing.

**Cost:** two layers of identity — session and workspace — where one would do for a single-agent
tool, and lifecycle rules for each.

### D12 — Documentation is a linted library, not a folder of Markdown

Frontmatter on every document, one owner per fact, generated index, mechanical checks, and a
same-commit update duty.

**Why:** the readers are agents arriving with no memory of previous work. A document that is wrong
is worse than no document, because it is trusted.

**Cost:** frontmatter to maintain, a linter to keep green, and a real constraint on where new
material may go.

### D13 — `just` is the only command surface

Every command anyone runs is a recipe, including the ones that are a single `cargo` invocation.

**Cost:** a dependency on `just` to do anything, and a second place to update when a command
changes.

### D14 — Diagrams are authored in a compact YAML form

Wireframes are written in a hand-authorable format and converted, rather than edited as native
diagram JSON.

**Why:** the native format carries around twenty-five bookkeeping fields per element, which is noise
for a human author and worse for an agent one.

**Cost:** a converter to maintain, and a format that supports a subset of what the diagram tool can
express.

### D15 — `_docs/design/` is assets, exempt from every documentation check

Captured prototypes and wireframes keep no frontmatter and are skipped by the linter.

**Cost:** that material is invisible to the catalogue and the drift queue, so a wireframe can go
stale silently. `ui-and-design.md` carries the pointer into it, and the reconciliation rule.

### D16 — `ubiq-layout.png` is the target shell, and the earlier prototypes are superseded

The workbench is an IDE-shaped window — activity rail, explorer, tabbed editor, bottom terminal
dock, right-hand chat — built against `_docs/design/ubiq-layout.png`. The `wireframe-opus` screens
and the captured HTML prototypes under `_docs/design/output/` describe an earlier shape and no
longer describe the shell.

**Cost:** two sets of design assets that disagree, and the older set stays in the tree because it
still records intent for screens the shell has not reached. Nothing automated will flag the
divergence — `D15` exempts that folder from every check.

### D17 — A thin kit over the component library, and screen areas as functions rather than views

`gpui-component` is used directly wherever it has a widget. `crates/ubiq/src/ui/kit/` holds only
what it lacks, is generic over no view, and never names `AppState`. Each screen area is a free
function over the root view rather than a view of its own, so `AppState` is the only `Render` in the
application.

**Why:** one view means one owner of state and one place that requests redraws, which is the whole
of the "mutation ends in a redraw request" rule. A window of independent panel views would mean
reconciling several projections of the same coordinator state.

**Cost:** the root view grows as the shell does, and a panel cannot hold private state without going
through it. If a panel ever needs its own focus and key handling, that is the point to reverse this.

### D18 — Surfaces are square, and a coloured left edge identifies them

No rounded corners anywhere except state dots. A surface is a fill plus one coloured border on its
left, and that border's colour is the whole signal: accent for what the user is acting in, a status
colour for something being reported, the project's colour for the window.

**Why:** a GPUI element has a single `border_color` for all four sides, so a neutral box with one
coloured edge costs two elements where one edge costs one — and at the sizes this UI uses, the edge
reads faster than the box.

**Cost:** it is a house style, not a convention anyone arrives knowing, so every new surface
has to be told about it. `ui::kit::slab` exists so that telling is cheap.

### D19 — A project is a colour, and a window belongs to one project

Each project owns a swatch from the theme's project group and wears it in the picker, the titlebar,
the mark and the window's left edge. A second window is a second `AppState` pointed at a different
project; windows share nothing but the process-wide palette.

**Cost:** a group of colours that carry no role, which is the one exception to how tokens are named.
And per-window state means anything that should be global — an open project set, a session list —
needs somewhere else to live before it can be shared.

### D20 — The emulator is `gpui-terminal`, vendored into the workspace

`D1` says Ubiq integrates an emulator rather than writing one; this names it. `gpui-terminal` is a
GPUI component that parses VT with `alacritty_terminal` and accepts any `Read`/`Write` pair, which
is exactly the shape a pane needs. It sits in `vendor/` as a workspace member rather than being
pulled from crates.io or git, because upstream builds against the published `gpui` and Ubiq builds
against Zed's `main`, where two calls have a different shape.

**Cost:** a copy of somebody else's crate in the tree, with the divergence recorded by hand and
reapplied on every rebase. Upstream fixes do not arrive on their own, and the patch list is only as
honest as the README that holds it.

### D21 — The UI is handed bus endpoints, not a pseudo-terminal

The emulator wants something to read and something to write. It is given a `Write` that turns a
keystroke into an input message and a blocking `Read` fed by the output messages routed to that
pane, both from `crates/ubiq/src/bus.rs`. The obvious alternative — handing the emulator the
pseudo-terminal directly, which is what its own example does — would have been fewer moving parts.

**Why:** it is what makes rule 2 structural rather than aspirational. The UI cannot reach a
descriptor it was never given, so a remote or detached harness is a change to what fills the stream
and to nothing else.

**Cost:** every byte is copied into a message and out again, and the emulator reads on a thread per
pane. A pane's stream also has to be closed explicitly when its harness exits, because nothing else
tells a blocking reader the process is gone.

### D22 — Closing a pane kills its harness

Closing the tab signals the child and reaps it. The alternative was to let the harness run on
unattended, or to keep the pane alive until the agent finished.

**Why:** a pane is the only handle the user has on a harness. An agent with no pane is an agent
nobody can see, stop, or answer — and one that keeps burning tokens.

**Cost:** a long-running agent cannot be parked by closing its pane, and a stray click ends work. A
detach that keeps harnesses alive without a window is the shape that reverses this, and it is
tracked in [`../backlog.md`](../backlog.md).

### D23 — Which window holds which project is process-wide, not per window

The project catalogue and the window-to-project map live in one GPUI global,
`WindowRegistry`, and every window reads it and redraws when it changes. Each window keeps only its
`WindowId`. The alternative — the copy each window used to keep — was simpler and needed no shared
mutable state at all.

**Why:** the picker has to answer "where is this project open?", and no window can answer that from
a copy of its own. The copies also disagreed: closing a project in one window left it open in the
other. Making a project open in exactly one window is what turns the picker into a view of the whole
desktop rather than of one window's guesses.

**Cost:** shared mutable state between windows, with the reader's discipline that comes with it —
reads go through `WindowRegistry::read`, because the `default_global` that would seed it on demand
notifies the observers on a plain read and spins the frame. And a window's lifetime is no longer its
own: emptying it closes it, from whichever window the user was clicking in.

### D24 — Diagnostics go to one process-wide sink, not over the bus

Every subsystem logs with `tracing`, a layer in `crates/ubiq/src/log.rs` pushes each event into one
ring the whole process shares, and the window's console reads that ring directly. The alternative
that would have honoured `D3` literally was a log message on the transport, with the coordinator
forwarding its records to the window that owns it.

**Why:** collection has to cost a subsystem nothing, or it does not happen. A `tracing::info!` with
no sink to acquire and no handle to thread through a signature is what makes the harness library, the
emulator and the framework collectable on the same terms as Ubiq's own modules — and none of them
can be taught to send a message on Ubiq's bus. The sink stays outside `D3` on its own terms: records
travel one way, a producer never reads, nothing in a record is a pane's state, a path or a handle,
and neither half learns anything from it. A message-based log would also have made the console blind
to everything emitted before the first window and after the last one.

**Cost:** one shared mutable structure both halves touch, which is exactly the shape `D4` warns
about — the discipline that keeps it one-way is written down rather than compiled. It is also the one
part of the system a detached coordinator does not carry across for free: its records would need the
transport, which is filed in [`../backlog.md`](../backlog.md). And a ring in memory means diagnostics
die with the process.

### D25 — The log console is a dock tab, not a panel of its own

The dock's tab strip lists the panes and then the console, and selecting it draws it where a pane's
terminal would be. The alternative, built first, was a fourth resizable panel under the dock with
its own titlebar switch and its own three size constants.

**Why:** the console answers a question about what an agent just did, and the dock is where the user
looks to ask it. A fourth row also took height from the editor and the dock permanently, for a
surface that is read in bursts. Reusing the dock means one strip, one size, one
hide button, and no new panel constants — and the tab's dot gives the console the one notification
surface it needs without stealing the view.

**Cost:** the dock's tab strip is not purely the pane list, so it carries one tab with no pane ID
behind it, and the strip's `+`, its close buttons and its dots mean one thing for panes and another
for the console. It also means the console and a pane cannot be read
at once, and the focus rule gains a case: the console holds the keyboard while it is shown, so a
pane that is off screen cannot be typed into.

## Related docs

- [`architecture.md`](./architecture.md) — the rules D3 to D6 produce
- [`agent-manager.md`](./agent-manager.md) — the boundary D8 and D9 create
- [`../backlog.md`](../backlog.md) — the choices still open
