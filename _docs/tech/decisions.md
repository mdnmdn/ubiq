---
id: tech-decisions
title: Decision register
kind: tech
status: current
summary: One entry per structural decision — what was chosen, why, and what it costs — cited as `Dnn` across this library.
read_when: you are about to argue with a rule, reverse a design choice, or make one a reasonable person might later reverse
updated: 2026-09-01
verified: 2026-09-01
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
pane, both from `crates/ubiq-proto/src/bus.rs`. The obvious alternative — handing the emulator the
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

Every subsystem logs with `tracing`, a layer in `crates/ubiq-proto/src/log.rs` pushes each event into one
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

### D26 — Every id in the contract is a ULID behind a per-kind newtype

`PaneId`, `SessionId`, `WorkspaceId` and `ProjectId` are newtypes over a ULID, replacing `Uuid`
everywhere in the message set. A ULID sorts by creation time, prints as 26 case-insensitive
characters with no hyphens, and so gives a readable directory name and a stable ordering for free.
The newtypes came free with the sweep: every id site was being touched anyway, and later they would
have cost a second one.

Sorting is a property of the *generator*, not the type, so there is exactly one — a process-wide
monotonic `ulid::Generator`. A bare `Ulid::new()` is never called.

**Cost:** one new dependency, and an id that carries its creation time, which is a fact worth
knowing before ids travel to a host the user does not own. `gpui::WindowId` stays the framework's,
so two id schemes coexist — but they never meet.

### D27 — The boundary between the halves is four crates, not a written rule

`ubiq-proto` holds the contract, `ubiq-host` the processes and the catalogue, `ubiq` the interface,
and `ubiq-app` the binary. The interface does not depend on the host, so reaching around the bus is
a compile error rather than a one-line import. `just host` and `just ui` check the two directions
mechanically, the way `just core` checks the harness library.

The binary is its own crate because a `[[bin]]` inside `crates/ubiq` shares that package's
`[dependencies]`: naming the host there would put it in the library's graph too, and Cargo has no
per-target dependency table. An optional dependency does not help either, because an enabled one is
a real dependency of every target in the package.

**Cost:** four manifests pinning their own versions, and a module move is a crate move. The
payoff is that the host builds and tests in seconds, without waiting for a rendering stack.

### D28 — One host per process, with a routing hub in the bus

The catalogue is process-wide, so the thing that owns it has to be. One host is started by the
binary before the first window; each window attaches and gets a client. Pane-family messages route
to the window that owns the pane, recorded when it was spawned; project-family messages are
broadcast, which makes every window's picker agree by construction rather than by asking again.

The alternative — a host per window sharing the catalogue behind a lock — is shared mutable state
reached around the bus, the shape `D3` exists to forbid, and it leaves two writers racing one
file.

**Cost:** a window closing no longer drops anything, so the host has to reap that window's
pseudo-terminals deliberately. Miss that and every closed window leaves a live harness; it is
covered by a test for exactly that reason.

### D29 — The catalogue is one TOML file the host owns; view state is opaque to it

Projects persist as a single `projects.toml` behind a `ProjectStore` trait. Tens of records make a
whole-file rewrite microseconds, the user can repair it in an editor, and nothing the catalogue does
needs a query, an index or a partial read. SQLite is a later swap and it lands on the per-project
cache, which has real volume and is deletable by definition.

View state goes through a second trait and is stored **opaque** — a string the host writes down and
hands back and never parses, on the same discipline that keeps terminal bytes uninterpreted. The
interface owns that schema, so the interface versions it, and a blob that fails to parse is
discarded rather than migrated.

**Cost:** two hosts writing one file is last-writer-wins, which is a backlog row rather than a
design question. And the host cannot validate view state at all, so a bad blob is only ever
discovered by the half that wrote it.

### D30 — Ubiq writes nothing inside a project's folder

Everything Ubiq remembers about a project lives under its own config root, keyed by the project's
ULID. Forgetting a project then cleans up completely, a read-only or missing folder still has its
view state, no repository acquires a file to gitignore, and no team has to agree on one.

That root is movable — `--config-root`, then `UBIQ_CONFIG_DIR`, then the nearest `ubiq.toml`, then
`~/.config/ubiq` — so a development run is self-contained by construction rather than by care. A
malformed bootstrap file is an error, never a fallback: falling back to the user's real catalogue
is precisely the accident the mechanism exists to prevent.

**Cost:** a config root you cannot see is a foot-gun, which is why the status bar says when it is
not the default. Redirecting the embedded harness library's own roots is not done yet, so a
development run is self-contained only as far as Ubiq's own stores, which is filed as a gap.

### D31 — Locating a project is its own message, and the interface chooses the colour

`UpdateProject` is display only: it renames and recolours, touches no filesystem, and cannot fail.
`LocateProject` changes truth — it canonicalises a path, re-probes health, and is refused when
another record owns the folder. Collapsing them would make one message "sometimes fallible,
depending which field you set".

The colour is the interface's to pick, because the palette is. `AddProject` carries an optional
swatch index and the host defaults it to zero; the host holds no opinion about what a project looks
like.

**Cost:** one more variant in a wide family, and a colour that nothing picks when a project is
added through some future non-interactive path.

### D32 — A project's folder is chosen in the platform's dialog

Ubiq drew its own folder browser, fed by a `BrowseHost` message, on the reasoning that a native
dialog browses the *interface's* filesystem rather than the host's. That reasoning is sound and the
result was still worse: a list of directory names, with no bookmarks, no network volumes, no path
field, no keyboard completion, and no resemblance to the chooser every other application on the
machine opens. Ubiq would be rebuilding the file manager, badly, to protect a separation it does not
have.

So Add and Locate call `prompt_for_paths`, and `BrowseHost`, `HostListing` and `HostEntry` leave the
contract entirely. What crosses the bus is the chosen path, inside `AddProject` or `LocateProject` —
messages that carry one regardless. Browsing *within* a project is a different question with a
different answer: that is the explorer's, drawn in the interface over a project-scoped listing.

**Cost:** this is the single place the interface assumes the host's filesystem is its own. A
detached host makes the dialog point at the wrong machine, and the browser this replaces would have
to come back — as a host-side listing behind the same two messages, not as a third path. Filed.

### D33 — A window with no project stays open, and Ubiq never closes a window by itself

A window whose last project was closed used to close, with an exception carved out for a first run so
that booting on an empty catalogue did not quit the application. The exception was the tell: "no
project open" was a real state the design kept trying not to have, reachable only by accident, and so
never drawn. It is the empty state instead — no panes, an explorer that says so, and an "Add a
project…" in the middle of the window — and the rule it needed is gone. `WindowRegistry::reap` is
deleted, and with it the convention where four mutations each answered with the windows they had
emptied.

**Cost:** a window may sit there holding nothing, which is one more screen to design and to keep
honest. The application still quits with its last *window*, so nothing about shutdown moved; what
moved is that only the user closes a window.

### D34 — A file failure names its path, not just its project

`ProjectFileError` carries a `project_id` **and** a `rel_path`, and its `error` is a `FileError` enum
rather than a sentence. `ProjectError` has nowhere to put a path, so the interface would have to
guess which tab to un-load or which folder to stop spinning, or mark the whole project for one
unreadable file. This is the third instance of a rule the contract states twice over: `PaneError` is
per pane so the message can go where the user is looking.

**Cost:** a fourth error variant, and an enum whose arms both halves have to agree about. The
alternative — one error type for everything — costs the interface a prose match on every failure.

### D35 — The file tree is listed one directory at a time

`ProjectTree` asks for one level, and the interface asks again when a folder is expanded. A bounded
eager walk was the alternative, and it loses on a real repository: `node_modules` and `target` are
hundreds of thousands of entries, and lazily each of them costs exactly one row and no recursion —
which is what stops the ignore set being the thing that saves you. It is also the shape a filesystem
watch wants (invalidate one directory, ask for one listing) and the shape a bounded transport wants
(a reply is bounded by one directory, not by the repository).

The ignore set follows from this: it **bounds descent and never hides a row**. A `ProjectTree` aimed
straight at `node_modules` lists it in full, because a tree with rows missing is a tree that lies. A
false positive — a `build/` holding real source — costs one extra click rather than a hidden file.

**Cost:** expanding a folder costs a round trip, so a deep tree is restored over several. Reading
`.gitignore` instead of a fixed set is filed, and is the one thing that would justify the `ignore`
crate.

### D36 — The file family runs off the coordinator's thread

A `read_dir` on a cold directory, a `canonicalize` on a dead network mount and a two-megabyte read
all block, and the coordinator's loop is what every pane's keystrokes and resizes pass through. So
file requests go to one worker thread and answer through a `Mailbox`, which is the device a pane's
reader thread uses for the same reason. This is the dual of the rule that a slow UI never blocks the
coordinator's reader.

One thread rather than a pool, deliberately: FIFO means one window's replies arrive in the order it
asked, which is what makes "replace the rows under this path" safe. A pool reorders, so two clicks on
one folder could leave the older answer on screen, and fixing that costs a sequence number on the
wire.

**Cost:** head-of-line blocking. A request against a hung mount holds the ones behind it. The fix is
the sequence number and a pool, and it is filed rather than built.

### D37 — A save names the version it read, and is refused if that moved

`WriteProjectFile` carries the `FileVersion` that came back with the contents, and a mismatch is a
`Conflict` with the file untouched. Ubiq is not the only thing editing these files — the agents in
the panes are — so last-writer-wins would silently discard an agent's edit, which is exactly the
class of loss the feature must not introduce. A write with no version means *create*, and is refused
if anything is there; the other reading, "force overwrite", is a footgun the contract would be
handing out for free.

A read cut short at the byte ceiling comes back with **no** version, which is what makes a truncated
buffer unsavable mechanically rather than by the interface remembering not to offer it.

**Cost:** an extra stat either side of a read, and a conflict the user has to resolve by hand —
Ubiq offers no merge. Writes are atomic and preserve the file's permissions, so a saved script does
not stop being executable.

### D38 — Each open file owns its buffer

One `EditorState` per open file, replacing the single shared buffer that was copied in and out on
every tab click. With real bytes behind a tab the shared buffer stops working rather than merely
costing a copy: "dirty" has to be a comparison against exactly what the host sent, contents can
arrive for a tab that is not in front, and a tab can exist with no bytes yet — none of which a
single buffer can express. Buffers also survive a project switch with their undo history, which is
the same promise the panes make.

**Cost:** N editor entities and N change subscriptions per window, each carrying a highlighter and an
undo stack — what every editor in this class carries. `state/editor.rs` names the component library,
which it was written not to.

### D39 — The work is the host's, tasks are written down per project, and sessions and agents stay its mocks

The board and the graph draw one project's work, and the host owns all of it: a fifth message family
in `crates/ubiq-proto/src/messages.rs`, a service in `crates/ubiq-host/src/work/`, and a `tasks.toml`
per project beside the view state under Ubiq's own config root. That is the state-ownership rule
applied to the last part of the tree that was inventing its own truth — with a fixture per window,
two windows disagreed about the same work and a task made or dragged went with the window that made
it.

Half of the domain is durable and half is not, and the naming carries the split: a `TaskRecord` is
the user's data, while a `WorkSession` and a `WorkAgent` are per-request payloads the host mints per
project and never writes down. Tasks were made real first because nothing about a task waits on a
live agent, so that half is finished rather than staged.

**Every reply goes to the window that asked**, unlike the catalogue's. A project is open in exactly
one window at a time, so the window that asked is the only one drawing that project's work and a
broadcast would buy nothing. The service still answers in `Reply`, so a broadcast is one word away
the day that changes; `Reply` lives in `crates/ubiq-host/src/reply.rs` rather than in `projects.rs`
because two services answer in those terms and `projects::Reply` would be a lie about ownership.

**The seeding rule turns on the presence of the file, not on its emptiness.** `TaskStore::load`
answers an `Option`, and `Work::ensure` mints the fixture, writes it and answers it only for a
project with no `tasks.toml`; from then on the file is the truth, including when it holds no tasks at
all. A user who deletes every task gets an empty board at the next boot, because an absent file and
an empty list are different things — the distinction `Preferences` draws between a blob never set
and an empty one. A load that *failed* never seeds either, on the reasoning that stops
`gc::collect` running after one: putting the fixture on top of a file you could not read is how you
overwrite the thing preserving it was meant to save. A project whose file came back
`UnknownVersion` is sealed and never written to at all, for the same reason one order further out.

Writing the seed immediately rather than at the first edit is what makes the fixture the user's data
on first sight: what they see is renamable, movable and deletable, and still there after a restart.

**A mock agent is linked to the task its session has in flight, or to no task at all.** The fixture
cannot name a task id, because the ids belong to whatever `tasks.toml` holds, so `Work::link` makes
the link where both lists are in hand — once per project, so a card the user has dragged into
another outline keeps where they put it. It reaches only for work that is `InProgress` or `InReview`:
an agent whose session has nothing in flight is left unlinked, which is not a gap but the shape the
graph draws above the containers, and it is where an agent coordinating everything belongs. Falling
back to the session's first task instead buries the project manager inside a container for work
nobody is doing.

**Cost:** the mock's session and agent ids are the same literals in every project, because a seeded
task's `session` is durable and freshly minted ids would leave every one of them naming a session
that no longer exists. `Step.owner` is durable on the same terms and can name no live agent, which
draws as unowned rather than being written out of the record. And the fixture is only ever seen once
per project, so editing it changes what a *new* project starts with and nothing an existing one
holds. All three are rows in the backlog.

### D40 — A task's edits are one infallible `UpdateTask`; a move and an assignment are their own messages

`UpdateTask` carries the title, the description, the priority and the shape, each optional, and
touches nothing outside the record: like `UpdateProject` it is display only and can be refused for
nothing but a task that is not there. `MoveTask` and `AssignTask` are separate variants, which is
`D31`'s test applied a second time.

The move is the sharper of the two, because what forbids folding it in is a rule rather than a
preference: the board prescribes that a column is a stage and a card only ever changes column, so a
`status` field on an update would be a second way to do the one thing the drag exists for, and a
status picker in a form would contradict a *Behaviour* section. The assignment names another entity,
is refused for a session the host does not hold — fallible where an update is not — and would
otherwise need an `Option<Option<SessionId>>` on the wire to tell "leave it alone" apart from "take
it back".

`UpdateTask` is an act rather than a keystroke: the interface commits on Enter, on a tick, or on
blur. That is what lets the host write on every one of them with no debounce, unlike the view state's
400ms.

**Cost:** three messages where one field would have done, and a form that cannot offer the one
control a user might go looking for in it. Driving a status from the keyboard then needs something
the drag does not provide, which is filed rather than built.

### D41 — Position is the interface's, membership is the host's

Where a card sits on the orchestration canvas never crosses the bus. `crates/ubiq/src/state/layout.rs`
owns every offset, a drag moves one, and nothing outside that window has an opinion about it. Which
task an agent *serves* does cross, as `AssignAgent`, because that is a fact about the work rather
than about the drawing — so a card dropped into another task's outline answers the pair: the
interface keeps the new offset and sends the new membership.

The test is whether a second window looking at the same project would have to agree. It would about
membership, and it could not about position, having its own canvas, its own zoom and its own
arrangement. An arrangement is view state, which `D29` makes an opaque blob the interface owns and
versions, so `Layout` staying in `crates/ubiq` is the same choice made twice rather than a new one.

**Cost:** an agent whose record arrives after the screen was arranged has no offset of its own, so
the interface owes it a placement rule; and an arrangement a user made is thrown away when the
project closes until it goes into that blob. Both are filed.

## Related docs

- [`architecture.md`](./architecture.md) — the rules D3 to D6 produce
- [`agent-manager.md`](./agent-manager.md) — the boundary D8 and D9 create
- [`../backlog.md`](../backlog.md) — the choices still open
