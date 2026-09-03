---
id: tech-decisions
title: Decision register
kind: tech
status: current
summary: One entry per structural decision — what was chosen, why, and what it costs — cited as `Dnn` across this library.
read_when: you are about to argue with a rule, reverse a design choice, or make one a reasonable person might later reverse
updated: 2026-09-03
verified: 2026-09-03
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
function over the root view rather than a view of its own.

**Why:** one owner of state and one place that requests redraws is the whole of the "mutation ends
in a redraw request" rule. A window of independent panel views, each with its own projection of the
same coordinator state, would mean reconciling several of them.

**Cost:** the root view grows as the shell does, and an area cannot hold private state without going
through it.

**Half reversed by `D42`.** This entry named its own reversal trigger — *if a panel ever needs its
own focus and key handling* — and a dock of independently focusable panels is that trigger arriving.
What reversed is the "one `Render`" half: a panel is a view, because the library requires one. What
stands is the half that mattered, and the half every sentence above is really about: **`AppState` is
the only owner of state.** A panel is an adapter holding a weak `AppState` handle and a panel kind,
and its render delegates to the same free functions this entry describes. The area modules were not
touched by the reversal, which is the evidence that the two halves were separable.

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

### D25 — The log console borrows the pane strip rather than taking a panel of its own

**Superseded by `D42`.** The console was a tab in the terminal dock's strip, beside the panes,
because the alternative it was weighed against was a fourth resizable panel with its own titlebar
switch and its own three size constants — permanent height taken from the editor and the dock, for
a surface that is read in bursts.

**Why it was superseded:** that trade only existed because the window's arrangement was fixed in
code, so a fourth area cost three constants and a switch. Under a dock a panel costs a `PanelKind`
variant and an arm of a `match`, and the console gets everything the strip was standing in for —
its own tab, its own dot, its own toolbar — while the strip stops carrying one tab with no pane ID
behind it. The cost this entry accepted is paid off with it: a pane and the console can be read at
once, and the focus rule loses its special case, because "no pane holds the keyboard while a
non-terminal panel is focused" is one rule about panels rather than one about the console.

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
view state, no repository acquires a file to gitignore, and no team has to agree on one. The git
directory is inside the project's folder, so this covers it too: the host never writes a ref, never
stages, and never lets libgit2 refresh the index stat cache.

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

### D42 — The window's arrangement is a dock the user rearranges, and it is the component library's

Every area of the workbench — the terminals, the log console, the explorer, the chat, the centre —
is a **panel** in a tree of tabbed groups the user drags, splits, tabs and zooms. The tree, the
drag, the drop geometry and the serialisation are `gpui-component`'s `DockArea`; Ubiq supplies the
panels and a skin over the library's three renderer traits, and writes no drag, no drop indicator
and no layout serialisation of its own. That is `D17`'s "gpui-component first" applied to the
largest widget in the library — and the same entry's other half is what it costs.

The frame it replaces was three resizable slots around a centre, the centre a stack of two, written
in `shell.rs` and varying only by a `visible` flag and a size within a constant range.

**Why:** the alternative is not "keep the frame" — it is "write a second dock". Four facts the frame
could not carry are each one line under this one. Only the focused pane was ever drawn, so the
domain rule that unfocused panes keep drawing had nowhere to be true. The console had to borrow the
pane strip, which is `D25`. `LayoutMode`'s four values were stored, returned by an accessor with no
caller, and drawn by nothing. And the whole arrangement died with the process.

Three things make the adoption a fit rather than a compromise. **A dropped tab is re-parented by
panel id**, so the entity is never rebuilt — a dragged terminal is the same emulator, on the same
stream, under the same harness, which is exactly what the pane rules demand. **Appearance is a
seam**: the engine owns the tree and the drag, and a renderer trait owns every pixel, so `D18`
holds inside a group exactly as it does outside one. And **where a panel may sit is a table**, not a
special case — one function from panel kind to a set of regions, consulted in one place, which is
what keeps the explorer and the chat on a border.

**Cost:** four, and each is real. `D17` is half reversed — a panel is a view, with the entity, the
focus handle and the event emitter that go with it. The library's dock has no top region, so
"docked on top" is a split at the top of the centre and takes its width from the centre rather than
spanning under the explorer. The library's drop is region-blind — a group offers a drop or it does
not — so a panel dropped where its class forbids is moved back on the same edit rather than
refused under the pointer, and the drag shows no indicator saying so. And a panel cannot read
`AppState` to answer whether it is visible, because the dock asks that while the window is mid-update
and the entity is under a lease; the window pushes the answer instead, which is one fact kept in
step by hand.

### D43 — The host links libgit2 and computes hunks itself, rather than driving `git`

`crates/ubiq-host` reads version control through `git2` and builds the hunks with `similar`. Both
are the host's alone: the interface gains no dependency, because a `FileDiff` crosses the bus as
rows with their line numbers on them. The git family — the overview and the working-tree map — uses
the same library, on a worker of its own; gitoxide is not a second reader.

**Why:** the alternative is spawning `git diff` and parsing its output, which puts a text format
between the host and the answer — the parser then owns every corner case the format has, and a
`git` that is missing, old, or configured with a pager or a diff driver changes what Ubiq shows. A
library also answers the two bases directly, as a blob from a tree and a blob from the index, so
`DiffBase` maps onto two calls rather than onto two command lines. And it keeps the whole thing
testable in-process, which is what the rest of the file family's tests are written against.

**Cost:** three. `libgit2` is a C library built from source, so the host's first build is longer and
a platform is one more thing that has to compile. Its behaviour is libgit2's rather than `git`'s,
which is why the clean and smudge filters are not run and `G61` records the difference. And the tree
holds a second comparison engine — the chat transcript's `EDIT` block draws a diff it was handed —
so a change to what a row means has two places to land until the viewer draws both.

### D44 — Mermaid is rendered in the interface, and the interface gets a workarea the host reserves

A diagram's source reaches the interface as a file's bytes, on the file family, like any other text.
The interface renders it — on a thread of its own, into a cache of its own — and the contract has no
diagram family at all. What the host gives it instead is one field: every `ProjectSnapshot` carries
a `workarea`, an absolute path to a directory under that project's own folder in the config root
that the host creates, reserves, and never reads inside.

**Why:** a Mermaid document is text, and the bus carries a file's bytes. A transport family for it
would have made the host answer a question that is not about the machine — what a picture looks
like — when drawing is the whole of what the interface is for. Rendering where the picture is drawn
also puts the palette on the side that owns it: the renderer bakes colours in, so a theme
switch is a re-render, and a re-render on the far side of a round trip is a round trip about
nothing. The workarea is the other half of the same move. A renderer needs somewhere to put what it
it has drawn, and the interface has nowhere: the project's folder is the user's and must stay
untouched, and the host's own files are the host's. Drawing that seam ahead of a remote
host arrives is what keeps it a value the interface is told rather than a path it composes.

**Cost:** four. The interface takes a heavy dependency — `merman`, and the transitive layout engine
under it — into the half that has to hit a frame budget, so the render must stay off the frame
thread and a diagram that is slow to lay out must show as pending rather than as a stall. The
workarea is a path in the interface, which is a real dent in rule 2 of
[`architecture.md`](./architecture.md); it is bounded by the interface never composing it, but a
second interface that is not on the host's machine has to earn that path some other way. A second
interface — a web one — gets no diagrams for free: it renders them itself or shows none, where a
host-side family would have served both. And the cache is the interface's to bound and to
invalidate, on a directory the host will delete without asking the moment its project is forgotten.

### D45 — A terminal pane intercepts a closed set of keystrokes and pointer gestures

The emulator forwards every keystroke and mouse event to the harness except a named set: platform
copy and paste, a defocus chord (`Shift+Escape`, `Ctrl+Escape`, `Cmd+Escape`), text selection when
the harness has not asked for the mouse, click-to-open on OSC 8 and `http(s)://` URLs, and OS file
drops as bracketed paste. `Ctrl+C` is SIGINT on every platform. Bare Escape is `\x1b`.

**Why:** a multiplexer that never copies, never pastes, and never lets the keyboard leave a pane is
a pane the user is trapped in. The intercepts are the emulator's (except defocus, which is the
window's) and invent no bus messages, because clipboard, drops and links are local to the UI
process. Bare Escape stays with the harness so vim, emacs and less keep the key they already own.

**Cost:** the closed set is a product contract. Adding a shortcut means adding it here and in the
emulator, and a chord that looks unused in a shell is often a command in a TUI. Mouse reporting
turns selection and link clicks off, which is what the harness asked for and what every other
terminal does — and what a user who wanted to select text in vim with `mouse=a` will not get.

### D46 — Settings are two layers, not a preference blob

How the application behaves is not where the window was left. View state stays
`GetPreferences` / `SetPreferences` with a `Scope` — opaque, debounced, discarded on a schema
the interface does not know. Application settings are `GetSettings` / `SetSettings` with a
`SettingsLayer`. The Ui layer is opaque the same way view state is, with its own schema so a
layout bump does not throw away a checkbox. The Host layer is parsed: a blob this host cannot
read is `SettingsError` and a corrupt file is preserved, because the host acts on it. Harness
definitions stay in agent-manager; they are neither layer.

**Why:** stuffing settings into `InterfacePrefs` would couple a dock-schema bump to a toggle,
and a host-parsed setting cannot live in a blob the host is forbidden to read (`D29`).

**Cost:** a fourth store, two files beside `preferences.toml`, and a second message family whose
write policy differs from the preference debounce. The Host record is empty of fields this build
acts on; the messages exist so a later host setting does not redesign the wire.

### D47 — Two screens over one set of agents: one to talk to them, one to arrange them

The rail carries `Agents` and `Orchestration`, and both read the same `WorkProjection`. Agents is
parallel columns — one conversation each, tabs that group, a composer per column. Orchestration is
the graph — who spawned whom, which task a card serves, where a card sits. Neither screen holds a
record; each holds its own arrangement over the same ones, and neither arrangement crosses the bus.

**Why:** the two questions want opposite shapes. Talking to an agent wants width — a transcript, a
harness readout, a field — and several of those side by side is the whole point of a multiplexer.
Arranging agents wants a canvas, and a canvas that also had to hold four composers would be a
canvas nobody could read. One screen doing both would be a graph with a chat drawer, which is the
inspector the orchestration screen carries, and which is not where a day's work is done.

**Cost:** two screens to keep honest about the same records, and a rename that moved the meaning of
`rail_mode: "Agents"` — so `prefs::SCHEMA` went to `3` and every window opens on its defaults once.
Two arrangements per project rather than one, and a user who closes a tab on one screen sees
nothing change on the other, because a column and a card are not the same claim about an agent.

### D48 — Version control gets a screen of its own, and it reads

`Git` is a rail mode beside `IDE`, holding the refs, the history, the uncommitted changes and the
diff on one screen. It draws the working tree from the pairs the git family already sends and the
comparison from the file family's `DiffProjectFile`; its branch list and its log are fixtures until
the family carries them. Nothing on it writes: the actions a write version would offer are drawn
inert, and the toolbar says why.

**Why:** the explorer's badges answer "is this file changed" and the status bar's branch answers
"where am I", and neither can answer "what has this repository been doing" — which is the question
a user asks before every commit and after every agent's turn. A screen is also where the *pair*
finally has somewhere to go: staged and unstaged are two lists here and one badge in the tree, so
the fact the wire already carries stops being thrown away at the edge. Drawing the write shape now
and refusing to wire it is what keeps the read version shippable: the layout is settled, and the
write family is one message set rather than a redesign.

**Cost:** a screen whose two most eye-catching areas are invented, which is a standing obligation
to say so — in its module headers, on the screen itself, and in `G83`. Controls that do nothing,
which is a defect anywhere else in this interface and is only defensible because the toolbar names
the reason. And a staged row compared against HEAD rather than the index, because the file family
has no third base — `G85`.

### D49 — A shell pane is a login shell, and which shells exist is the host's answer

`pty::spawn` starts a program with no arguments that `shells::is_shell()` recognises the way a
terminal application starts a shell: argv0 prefixed with `-` on Unix, which is what makes
`.zprofile` and `.profile` run. The menu on the new-pane control offers a fixed candidate list the
host checked for existence — never a path the interface guessed at, and never an open field.

**Why:** a non-login shell reads `.zshrc` and not the profile that put Homebrew, `pyenv` and the
rest on `PATH`, so a pane reported `command not found` for tools that were installed and worked in
every other terminal — and Ubiq launched from Finder starts from a `PATH` that nothing has set up,
which is exactly the case the profile exists to fix. Picking a different shell from a menu would
have moved that same defect onto a different program, so the spawn path was fixed first. The list
being the host's is not a preference: a program on disk is a local fact, and no path crosses into
UI code.

**Cost:** `portable-pty` only does the argv0 prefixing for a builder made with `new_default_prog`,
which takes no program name and reads the shell out of `SHELL` — so the chosen shell is handed to it
there, and the login path is available only to a shell started with no arguments. A fixed candidate
list means a shell nobody thought of is not offered even when it is installed and is the user's own
default — the default is always listed, whatever it is, but nothing else outside the list is. And a
menu that re-probes on every open does a handful of `stat` calls on the coordinator's own thread.

### D50 — The pane region is not furniture: it opens empty, and opening it starts a pane

A fresh window's bottom region holds nothing. The console is not installed in it — the `Logs` row on
the new-pane control's menu is what puts it there, and its tab's × takes it away — and the region
starts put away, at its size, with its tab strip. The titlebar's switch that brings it on screen
starts a pane in it.

**Why:** the console was in every window's arrangement whether or not anyone had asked for it, which
is a panel's worth of screen spent on the window's own diagnostics. Making it a row on the menu the new-pane
control carries costs nothing and gives the region back. The region cannot simply be left out
instead: it is where a pane lands and where the control that opens one is drawn, so it has to exist
before the first pane does — and once it exists, opening it has to give the user something, which is
the pane they were reaching for.

**Cost:** `feat-logs`'s "always present and never closed" is reversed, and a window with no project
reaches its own diagnostics through one menu row rather than a panel in front of it. An empty
region is a legal arrangement, which the tab-strip control has to be told about: the skin is
handed a group and knows nothing about placement, so the window answers `is_pane_region()` for it.
And the IDE's default regions changed under existing users — a window that had arranged its bottom
region keeps it, one that had not opens without it.

### D51 — A pane's place in the arrangement survives a rebuild

A terminal panel writes its pane's id into the saved layout, and a saved leaf naming a pane the
window still holds is rebuilt where it was. One naming a pane the window does not hold is dropped,
as before.

**Why:** "layout persists, harnesses do not" was read as "a terminal leaf is always dropped", which
is right across a restart and wrong within a session: switching rail mode or project rebuilds the
tree, and every pane in it was re-added to the first group of the bottom region — losing its group,
its split, its tab position, and shifting whichever tab was displayed beside it. The pane is alive
and the window is holding it; the only thing missing was a name in the file to put it back by.

**Cost:** a payload on a panel that carried none, and a build closure that may refuse — `restore`
asks the window for each panel rather than assuming it can have one, because whether a pane exists
is the coordinator's answer and not the layout's. A blob written before this carries terminal leaves
with no payload; they are dropped, which is what the old code did with all of them.

### D52 — An agent is composed by the library and confined by default

`SpawnWorkspace` naming an agent type the harness library knows is composed rather than executed:
the library provisions a throwaway configuration directory, answers with the launch, and resolves
the policy the run is confined under. Anything the library does not know stays a program name, which
is what a shell is. Confinement is on unless the host settings turn it off, and grants the project's
folder and that run's own directory — nothing else.

**Why on by default:** an agent that edits files is what a deny-by-default policy is for, and a
default the user has to find is a default nobody has. The grant set is the smallest one that lets a
harness do the work it was opened for.

**Why the registry decides what to exec, not the coordinator:** `Composed` keeps its launch and its
policy private and answers `exec()`, so a confined run cannot be started unconfined by reaching for
the wrong field. Losing confinement silently is the one failure here that looks like success.

**Why the run directory is the pane's:** the library would otherwise mint its own under
`~/.config/agent-manager/runs`, named by a timestamp and a pid. Naming it by the pane is what lets
closing a tab delete exactly one run's state — credentials seeded into it included — and what makes
a directory left by a killed process identifiable at the next start, which is when the host sweeps
them.

**Cost:** three of them. The environment a pane starts from is no longer Ubiq's own, so `pty::spawn`
takes a `Program` rather than a program name — a confined run brings its whole environment, because
the policy sanitized it. Confinement in a terminal Ubiq owns is macOS-only, because isol8 spawns
with inherited stdio and a host cannot hand it a pseudo-terminal; the seam that fixes it is
specified in `refs/isol8-pty-seam-update.md` and the stopgap renders the policy and execs
`sandbox-exec`. And a harness whose toolchain lives outside the project reads as broken until a
recipe grants it — both are rows in the backlog register.

### D53 — The agent conversation is ACP-shaped, bus-transported, and keyed by agent id

A live agent's traffic uses the Agent Client Protocol's `session/update` vocabulary — its event
names, its tool-call shape with a kind and a status and a diff, its permission options, its config
options — in three places: the library's neutral event model, the message family in
`crates/ubiq-proto/src/conversation.rs`, and the one mapper between them in
`crates/ubiq-host/src/conversation.rs`. The transport stays the in-memory bus, and the identity that
routes a conversation is the `agent_id` on every variant.

**Why the vocabulary and not the protocol:** `D9` says Ubiq embeds the harness library rather than
shelling out to `am`. Putting JSON-RPC between two halves of one process would undo that for no
gain. What is worth borrowing is the shape, because it was designed for exactly this problem by
people who had to make it work against several harnesses — and because several harnesses in the
library's own reference table are launched as `<binary> acp`, so an inbound ACP bridge reads its own
vocabulary instead of being translated a third time.

**Why the identity is on the message and not on the event:** an event a bridge produces carries no
session id at all. Whoever holds the table of live bridges attaches one — the host attaches an
`agent_id`, and a server exposing the same library over ACP would attach a `sessionId` to the very
same event. That is what keeps one mapping serving both, and it is the whole reason the library
could later be an ACP agent without a second projection.

**Why one mechanism for models, modes and thinking levels:** upstream deprecated its dedicated mode
methods in favour of a generic config option, and never had model methods at all — a model picker is
a config option whose category says `model`. Copying that gives one shape for every knob, so a
harness that grows a fourth needs no change in the interface, and the pickers are generated from a
list rather than enumerated in code.

**Why deltas rather than records:** the work family echoes a whole record on every change, which is
right for a record that changes rarely. A token stream is the case that breaks it — re-sending a
conversation per token is quadratic in what was said. So a conversation update carries one thing and
the window folds it in, with a per-agent sequence number so a lost message is visible rather than
silent.

**Cost:** three of them. Our vocabulary lags upstream's, and a v2 that reshapes diffs into structured
file changes and makes the message id required is drafted — every one of those is a change here, and
`refs/acp-protocol.md` records what is coming. Two variants are on the wire and refused, because the
family was designed whole rather than grown one at a time. And a conversation and a pane are two
spawn messages rather than one, which is the price of a record that does not carry geometry nobody
set.

### D54 — A dropped folder opens as a temporary project; a dropped file outside every project is a read-only guest tab

A folder dropped on the editor centre or a file tab opens immediately as a project — the host mints
an ordinary `ProjectRecord` with a `temporary` flag and keeps it in memory only, never writing it to
`projects.toml` — rather than opening project settings prefilled and waiting on `AddProject`. A file
dropped that lands outside every open project opens as a read-only guest tab that the interface
reads itself with `std::fs`, rather than through the host as a loose project. Both reverse rows (a)
and (c) of `_docs/inbox/shell-integration-proposal.md` §12, settled here 2026-09-03.

**Why a temporary project instead of prefilled settings:** naming and colouring a folder before it
has proven worth keeping is friction the drop was supposed to remove. Opening it at once and putting
the only decision — keep it — behind the same settings dialog costs nothing extra: `UpdateProject`
on a temporary record is what clears the flag and writes it down, so there is no separate promote
message, and every file, git, work and pane operation resolves a project through the host's
in-memory lookup, temporary or not.

**Why a guest tab instead of a loose project:** the direct read was rejected once, in the proposal's
own §3, for what it costs — reimplementing `FileVersion`, `is_binary`, `truncated` and every
`FileError` arm, or shipping an editor whose save can eat an agent's work. That argument holds; what
is different is the answer to what a guest file may do with what it read. `OpenFile::savable()` also
requires `version: Some(_)`, so a guest file — built with `version: None` — cannot reach a save at
all. The failure §3 feared, a save landing on a change an agent made a second earlier, cannot happen
to a buffer that has no save button. That is narrower than a loose project's read-write editor, and
it is the whole of why the interface is allowed to read the bytes itself here: nothing it produces
can be written back.

**Cost:** two of them. `crates/ubiq/src/app.rs` calls `Path::is_dir` and `std::fs::read` directly,
which architecture rule 2 otherwise forbids the interface — see the exception recorded in
[`architecture.md`](./architecture.md), rule 2. And a guest tab is read-only for good: promoting one
to a real, savable file means dropping it again inside the project that holds it, not an in-place
upgrade.

## Related docs

- [`architecture.md`](./architecture.md) — the rules D3 to D6 produce
- [`agent-manager.md`](./agent-manager.md) — the boundary D8 and D9 create
- [`transport-contract.md`](./transport-contract.md) — the conversation family D53 shapes
- [`../backlog.md`](../backlog.md) — the choices still open
