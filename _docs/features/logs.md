---
id: feat-logs
title: Logs
kind: feature
status: current
summary: One sink every subsystem writes its diagnostics to, and the console panel that reads it back with a subsystem selector and a level floor.
read_when: you are adding a log event, adding or renaming a subsystem, changing what the console shows or where it sits, or chasing why something the application did left no trace
updated: 2026-09-04
verified: 2026-09-04
code_anchors: [crates/ubiq-proto/src/log.rs, crates/ubiq/src/state/logs.rs, crates/ubiq/src/ui/logs.rs, crates/ubiq/src/state/dock.rs, crates/ubiq-app/src/lib.rs]
depends_on: [tech-architecture, feat-panes]
review_cycle: monthly
---

# Logs

## Purpose

Ubiq runs several things at once that can fail independently: a window, a coordinator per window,
a pseudo-terminal and a reader thread per pane, and the harness library composing a run underneath
all of it. When one of them misbehaves, the question is always the same — what did the others say
while it happened. The log system answers it in one place: every subsystem's diagnostics land in one
ring, and the console reads that ring with the two controls the question needs, which subsystem and
how loud. It sits where the user puts it: a panel in the window's dock, beside a terminal, in a
group of its own, or wherever it has been dragged to.

## Behaviour

**A subsystem logs with `tracing` and nothing else.** There is no sink to acquire, no handle to
thread through a signature and no registration step. `tracing::info!` from anywhere in the process
is collected, which is what makes the system central rather than a convention: a crate that has
never heard of Ubiq — the harness library, the emulator, the framework — is collected on the same
terms as Ubiq's own modules.

**A record's subsystem is derived, not declared.** The event's target is the emitting module's path,
and the map from module to subsystem lives in one function. Six subsystems: **UI** for the window,
its screens, the state they draw and the emulator; **Coordinator** for the coordinator and the bus;
**PTY** for pseudo-terminals; **Harness** for the embedded library; **MCP** for the surface Ubiq
exposes to the agents it hosts; and **External** for everything else that logs. Nothing falls
through — an unrecognised target is External, not missing.

**Records travel one way.** Producers write and never read. The console reads and writes nothing a
producer can see, and the only thing it asks of the sink is to be emptied. That is what makes a sink
shared by both halves something other than a way around the bus: it carries no pane state, no path
and no handle, and neither half learns anything from it — see `D24`.

**The ring holds five thousand records and says what it dropped.** The oldest goes when the next one
arrives, and the count of what went is kept and reported beside the record count, because a console
that silently loses its beginning is a console that lies.

**One ring for the whole process.** Every window's console shows every window's subsystems, which is
the point: two windows mean two coordinators, and a question about one of them is usually asked from
the other. Clearing empties it for all of them at once.

**What reaches the ring is `RUST_LOG`'s decision.** With nothing set, Ubiq's own modules and the
harness library are collected down to debug and everything else only when it complains —
`ubiq=debug,ubiq_app=debug,ubiq_host=debug,ubiq_proto=debug,agent_manager=debug,gpui_terminal=debug,warn`.
The same filter feeds a writer on standard error, so a run from a terminal reports without the
console being open. [`../tech/operations.md`](../tech/operations.md) owns the commands that set it.

**The Harness subsystem is the structured bridges reporting.** A harness driven as a conversation
speaks JSON on a pipe, and `crates/agent-manager/src/io/` is where that is read: every frame it
decodes into an event is a `debug` record naming the event, and every raw frame, in both
directions, is a `trace` record naming the direction and the line. The events are what a transcript
that draws nothing is diagnosed from; the raw frames are what a mapping that drops something is.
The host's own side of the same conversation logs under Coordinator, because that is where
`crates/ubiq-host/src/conversation.rs` lives.

**Raw frames are asked for by name.** A prompt and the contents of every file a harness reads travel
in them, so the default filter's `agent_manager=debug` collects the decoded events and leaves the
frames out; `RUST_LOG=agent_manager::io=trace` is what turns them on for a run that needs them. The
reason is not the one that keeps terminal bytes out below — it is that a diagnostic ring is a poor
place to keep the user's own code, and a reader who wants it there should have said so. No control
in the console asks for it, which is a row in [`../backlog.md`](../backlog.md).

**Terminal bytes are never logged.** A harness drives a screen at full refresh; putting that stream
in a diagnostic ring would cost more than the harness it is diagnosing and would leak what the agent
said into a buffer nobody asked for. What is logged about a pane is its lifecycle: opened at a size,
started, stream ended, exited, failed.

**The console is a panel.** It has its own tab, its own toolbar and its own place in the
arrangement, so it is dragged, tabbed beside a terminal, split, zoomed and moved between the centre
and the pane region like anything else in the dock, which is what lets a pane and the console be
read at once. **It is opened on demand and closes like anything else.** A fresh window's arrangement does not
hold it: the `Logs` row on the new-pane control's menu is what puts it on screen — the region it
sits in opens and its tab comes to the front — and its tab's × takes it away again. That row is
drawn whether or not a project is open, which is the state the console is most worth reaching in. A
window that was left with the console open comes back with it, like every other panel in a saved
arrangement.

**The console's panel holds the keyboard while it is displayed.** A pane that is off screen must
not keep receiving keystrokes, so a panel that is not a terminal becoming its group's displayed tab
leaves no pane focused at all, and a terminal panel becoming one hands the keyboard back to its
harness. Nothing types into a terminal nobody can see.

**The tab reports the loudest thing the ring holds.** A record at `WARN` or above puts a dot of its
own colour on the console's tab and the same colour on the panel's left edge, so it is visible while
the console is a background tab; clearing the ring clears both. That is the whole notification
surface — nothing steals the view from the agent the user is watching.

**Two controls decide what is drawn.** The subsystem selector picks one subsystem or `All`; the level
selector sets a floor, so `WARN` means warnings and errors. Both sit in the console's own toolbar
row, above the records and beside the record count, the follow switch and `Clear`. Both are the
window's own state, so two windows can watch the same ring through different filters.

**A row states when, how loud, from where and what.** Time in the reader's own zone to the
millisecond, the level as a coloured word, the subsystem, and the message with any structured fields
appended as `key=value`. A warning or an error carries its status colour on the message as well as
the level word, which is what makes one findable in a wall of debug. A message is one line and is
cut off rather than wrapped, because uniform rows are what let the list draw only what is visible.

**Following keeps the tail in view.** With it on, an arriving record scrolls the list; with it off,
the list stays where the reader put it. Records keep arriving either way.

**A burst is one redraw.** The sink nudges each console when a record arrives, the nudge carries
nothing, and the window coalesces whatever else arrives behind a short settling delay. That is also
what stops a record emitted while drawing from redrawing the frame that emitted it.

## Contract

No transport message. The sink is process-wide and both halves write to it directly, which is the
one thing in Ubiq that crosses the UI/coordinator line without the bus, and the reason it is
recorded as a decision rather than left as an exception. When the coordinator becomes its own
process, its records need the transport like everything else; that is filed in
[`../backlog.md`](../backlog.md).

State: `LogState` for the console's filter and its follow flag, `PanelKind::Logs` for the panel
itself, and `MenuId::LogSubsystem` and `MenuId::LogLevel` for the two selectors. The arrangement the
panel sits in, and the focus rule it takes part in, belong to
[`panes-and-terminals.md`](./panes-and-terminals.md).

## Implementation

`crates/ubiq-proto/src/log.rs` is the sink. `install()`, called from the binary before the window and the
host exist, puts two layers behind one `EnvFilter`: the ring, and a plain writer on standard
error. The ring layer turns an event into a `LogRecord` — sequence number, timestamp, level,
subsystem, target, and the message with its fields — and pushes it into a `VecDeque` of shared
records under a mutex. `Subsystem::of` is the module-to-subsystem map, and it tests the specific
prefixes first, because `ubiq_host::pty` is also `ubiq_host`, and the bare `ubiq` arm is last
because every one of Ubiq's crates starts with it. A target is the emitting module's path, so it
carries the crate name: the map is `ubiq_host::pty`, `ubiq_host::coordinator`, `ubiq_proto::bus`
and `ubiq_host::mcp_server`, and a crate renamed without the map following it files every record
under External while compiling perfectly.

`logs()` is the ring, held in a `OnceLock`. `snapshot()` filters and hands back shared records, so a
console's read costs a pointer each and never holds the lock across a frame; `counts()` answers the
record readout and `loudest()` the tab's dot; `clear()` empties both; `subscribe()` hands out a
channel a window is nudged through, and a listener whose window has gone is dropped on the next
push.

`crates/ubiq/src/state/logs.rs` holds `LogState`: the chosen subsystem, the level floor, the follow
flag, and the index arithmetic the two selectors are drawn and answered with. It holds no records —
those belong to the sink, which is why the console's state is a default rather than a fixture.

`crates/ubiq/src/ui/logs.rs` draws the panel. `render()` is the whole of it, a toolbar row over the
records; `actions()` is that toolbar — the two selectors, the record readout, the follow switch and
`Clear` — and `body()` is a lazy uniform list of rows on a surface like a pane's. `level_colour()`
is what a level is reported in, on a row and on the tab's dot alike.

`AppState` owns the filter state, the list's scroll handle, and the task started in `for_project()`
that drains the sink's nudges and asks the window to redraw. The console's keyboard belongs to its
panel, like every other panel's. `pick_log_subsystem()`, `pick_log_level()`, `toggle_log_follow()`
and `clear_logs()` are the rest.

The events themselves are spread across the subsystems that own them: a window opening, a workspace
started or failed, a conversation started, updated, failed or stopped, a pane closed and its harness
killed, a pseudo-terminal opened at a size, a pane's stream ending, a pane exiting, and a message
that arrived at the half that may only send it. The harness library's own are all under
`crates/agent-manager/src/io/`, one per bridge.

`crates/ubiq-proto/tests/log_sink.rs` drives the sink the way a subsystem does — real `tracing`
events, read back through `snapshot()` — and covers the classification, the two filters composing,
the nudge and the clear. `crates/ubiq/tests/logs.rs` covers the two pickers on the indexing the
console draws them with. No window is needed for either.

## Failure

| What happens | Result |
|---|---|
| Nothing matches the filter | The console says which subsystem and level found nothing; the controls keep their selection |
| The ring fills | The oldest records go, and the header reports how many |
| `install()` is called twice | The first collector stays; the second is dropped rather than replacing it |
| A record arrives while the console is a background tab | It is kept in the ring, and a warning or an error shows on the tab's dot; a collapsed region draws nothing for it |
| A record is emitted while the window draws | Coalesced behind the settling delay, so it cannot drive the frame that emitted it |
| A window closes | Its listener is dropped on the next push; the ring and every other console are untouched |
| `RUST_LOG` names a filter that excludes a subsystem | That subsystem's records never reach the ring, and the console cannot show what was not collected |

## Related docs

- [`panes-and-terminals.md`](./panes-and-terminals.md) — the panes the console shares the dock with, and the focus rule it takes part in
- [`workbench.md`](./workbench.md) — the shell the dock sits in
- [`../tech/architecture.md`](../tech/architecture.md) — the two halves, and the rule the sink is measured against
- [`../tech/decisions.md`](../tech/decisions.md) — `D24`, why the sink is shared and what it costs, and `D42`, the dock the console is a panel in
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens the rows are coloured with and the console's size constants

## Next steps

- Filter rows by text as well as by subsystem and level.
- Open the console from the keyboard, and from a click on an error the status bar reports.
- Copy a row, or the visible selection, to the clipboard.
- Write the ring to a file on request, for a bug report that outlives the process.
- Reach the raw harness frames from the console, rather than only from `RUST_LOG`.
- Carry the coordinator's records over the transport once it is its own process.
