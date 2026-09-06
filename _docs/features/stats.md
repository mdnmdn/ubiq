---
id: feat-stats
title: Stats
kind: feature
status: draft
summary: The Control screen — five readings of the running host on one page, and the usage meter on the other, whose tables exist and whose producer does not.
read_when: you are changing what the Control screen reports, the usage meter's schema or its buckets, or the one thing the interface polls the host for
updated: 2026-09-06
verified: 2026-09-06
code_anchors: [crates/ubiq-proto/src/stats.rs, crates/ubiq-proto/src/messages.rs, crates/ubiq-host/src/store/usage.rs, crates/ubiq-host/src/coordinator.rs, crates/ubiq-host/src/projects.rs, crates/ubiq/src/state/stats.rs, crates/ubiq/src/ui/stats.rs, crates/ubiq/src/app/stats.rs, crates/ubiq/src/app/wire.rs, crates/ubiq/src/app/shell.rs, crates/ubiq/src/ui/dock/mod.rs, crates/ubiq/tests/stats.rs, crates/ubiq-host/tests/usage.rs]
depends_on: [tech-transport, feat-workbench, tech-structure]
review_cycle: monthly
---

# Stats

## Purpose

Ubiq is one host process behind however many windows the user has open, running however many
harnesses they started, and nothing on screen says how much of the machine that is costing. The
Control screen answers it: how much memory the host holds, how long it has been up, how many
projects it is keeping open, and how many agents are alive now against how many have run since
launch. Beside those readings sits the meter of what the agents have spent, which is the same
question asked of tokens rather than of the machine.

## Behaviour

**Control renders the screen directly.** The rail's `Control` mode fills the centre with this
screen and nothing wraps it — there is no outer tab strip, so the screen's own strip is the only
one, with two pages: **General** and **AI**.

**It answers whether or not a project is open.** Control sits under the rail's `APP` group beside
the kitchen sink, and everything it reports is a fact about the application rather than about a
folder: the host runs, holds memory and keeps projects open regardless of what this particular
window has open. A window with an empty catalogue opens it and still reads something true.

**General shows five readings and no meter.** Memory as the host's resident set, uptime as how long
the coordinator has been running, the number of projects with at least one pane in them, the agents
alive now, and every agent started since this run began — the last of which counts conversations
that have already ended, so it only ever rises. **No bar and no percentage anywhere on the page.** A
meter needs a denominator, and nobody can name a memory ceiling for a host that spawns
pseudo-terminals on demand; a fraction of an invented total would be drawn with more confidence than
the number behind it deserves.

**A figure nobody has stated is an em dash, never a zero.** Before the first reply arrives the
window holds no reading at all, and a zero would be the host claiming it uses no memory and runs no
agents. The same distinction holds one level in: a reading whose memory field is absent is the
platform declining to say, and reads as `unavailable`.

**The AI page has no data source.** The database exists, its two tables exist, their migrations run,
and the host answers with them on every reply — and nothing ever writes a row, because the agents
engine does not report what a turn spends. So the page draws its empty state every time and says
why. The table that would draw the rows is written and simply not reached, and no fixture is seeded
into the meter to make it look inhabited: a plausible number on a screen whose whole job is to
report real ones would be a lie. Closing that gap is `G160` and `G161` in
[`../backlog.md`](../backlog.md).

**The meter is a database, kept beside the catalogue rather than in the cache.** It lives at
`<config root>/usage.db`. `cache/` is defined as everything that can be re-derived by asking again,
and nobody can ask a harness what it spent last Tuesday — losing this file loses history rather than
costing a re-probe. [`../tech/project-structure.md`](../tech/project-structure.md) owns the config
root's shape.

**Two granularities, and the second one is this run only.** `usage_hour` is durable and accumulates
across every run; `usage_minute` is emptied when the meter opens, which is what makes "this run"
mean what it says and why the table needs no run-id column — its rows were summed into the hourly
table as they were written, so dropping them loses nothing. Both are keyed on
`(bucket, project, harness, account, model)`, are `WITHOUT ROWID`, and are written with an
accumulating upsert, so a second report for the same five dimensions in the same bucket adds into
the row already there. Every dimension is `TEXT NOT NULL` with the empty string as "not said":
SQLite treats two `NULL`s as distinct in a key, so a nullable column would silently stop
accumulating.

**A bucket is a floored unix second, in UTC.** The host has no opinion about how an hour is written;
naming it in the reader's zone is the screen's, in the one function that draws it.

**The schema migrates on `PRAGMA user_version`.** The steps are a const array of SQL in the meter's
own module, embedded in the executable, and a step's index is its version — `D78`. There is no
migration crate and no schema table.

**`ListStats` is the one thing the interface polls for.** Every other message in Ubiq is pushed
because something happened; memory and uptime change when nothing happens, so there is no event to
push. The window asks every two seconds while Control is the rail mode and stops the moment it is
not, and one flag on the screen's state keeps a second timer from starting behind the first.
Entering the mode starts the loop, and so does restoring a project that was left in it, which never
goes through the mode switch.

**A reply goes only to the window that asked.** Two windows watching the screen take their own
samples, which is what keeps a sample a sample rather than a broadcast of one window's timing.

## Contract

[`Message::ListStats`](../tech/transport-contract.md) and `Message::Stats`, carrying `HostStats`
from `crates/ubiq-proto/src/stats.rs` with its `UsageRow` vectors. Both are documented in
[`../tech/transport-contract.md`](../tech/transport-contract.md)'s project family.

State: `StatsState` and `StatsTab` in `crates/ubiq/src/state/stats.rs`, on the window rather than on
an open project. `RailMode::Control` is the mode that draws it; the rail and its grouping belong to
[`workbench.md`](./workbench.md).

## Implementation

`crates/ubiq/src/ui/stats.rs` draws the screen: `render()` is the tab strip over one of two pages,
`general()` the five readings as label-and-value rows on one surface, and `ai()` the empty state or
the table. `humanise()`, `uptime()` and `at_hour()` are how a byte count, a duration and a bucket
are read; `COLUMNS` fixes the table's widths rather than sizing them to content, because a column of
figures that resizes with its longest cell stops lining up.

`crates/ubiq/src/state/stats.rs` holds the chosen tab, the last reading as an `Option`, and the flag
that says a refresh loop is already running. `crates/ubiq/src/app/stats.rs` is the loop:
`poll_stats()` spawns it, `set_stats_tab()` starts it too, and it exits when the window has gone,
the state has gone, or the mode has changed. `crates/ubiq/src/app/wire.rs` takes `Message::Stats`
and replaces the reading whole.

`crates/ubiq-host/src/coordinator.rs` answers: `stats()` samples memory through `memory_stats`,
uptime from the `Instant` the coordinator started, the live conversations it already holds, the
`agents_this_run` counter it bumps as each conversation starts, and
`crates/ubiq-host/src/projects.rs`'s `open_count()`. The two usage vectors come back from the meter.

`crates/ubiq-host/src/store/usage.rs` is the meter, and unlike everything else under `store/` it is
a concrete type rather than a trait: there is one implementation and nothing substitutes it.
`open()` creates the file, sets WAL, migrates, and empties the minute table; `record()` writes one
delta into both tables in one transaction; `history()` and `this_run()` read them back.

`crates/ubiq/tests/stats.rs` covers the rail grouping and the two pages' copy without a frame;
`crates/ubiq-host/tests/usage.rs` drives the meter against a real file.

## Failure

| What happens | Result |
|---|---|
| The database cannot be opened — a read-only config root, a corrupt file | The coordinator logs it and carries an absent meter; the session is untouched and the AI page is empty |
| A usage read fails | Empty rows and a log line, never a dead screen: a screen reporting on the host's health must not be the thing that takes it down |
| The platform will not report resident memory | The reading is absent and the row says `unavailable`, not zero |
| No reply has arrived yet | Every figure is an em dash, and the agents-live badge is one too |
| The screen is left for another mode | The timer stops on its next tick, and the last reading stays for whenever the screen comes back |
| A harness reports usage | Nothing is recorded — no caller reaches `record()` yet (`G160`) |

## Related docs

- [`workbench.md`](./workbench.md) — the rail, its modes, and the centre panel this screen fills
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the two message variants and the polling rule
- [`../tech/project-structure.md`](../tech/project-structure.md) — the config root the meter's file sits in
- [`../tech/decisions.md`](../tech/decisions.md) — `D78`, SQLite for the meter and what it costs
- [`../backlog.md`](../backlog.md) — the producer the AI page waits on

## Next steps

- Record usage where a harness reports it, and widen the event so the schema's columns can be filled.
- Bound the history the host answers with, once the screen has an opinion about how far back a chart goes.
- Draw the this-run minutes as a shape rather than a table.
- Report cost beside tokens, where a harness states one.
