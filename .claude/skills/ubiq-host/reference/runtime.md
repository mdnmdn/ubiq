# The host at runtime — threads, lifecycles, teardown, durability

## The threads

One host per process. One coordinator thread. Everything slow is somewhere else, because the
coordinator's thread is the one every pane's keystrokes pass through.

| Thread | Started by | Lives | Why it is not the coordinator's |
|---|---|---|---|
| `ubiq-coordinator` | `coordinator::start`, by the binary before the first window | The process | — |
| Files worker | `Files::start()` in `Coordinator::new` | The process | A cold `read_dir` would stall every pane (`D36`) |
| Git worker | `Git::start()` | The process | A cold status on a large repository is seconds |
| Search worker | `Search::start()` | The process | Long-running by nature; a search behind a slow one would stall every folder expand |
| Index thread | `Index::start()` | The process | Hands out a read handle carrying no writer, so a search can never block indexing |
| One reader per pane | `Pty::forward_output` | Until the stream ends or nobody is listening | **A stalled reader stalls the harness** |
| One reaper per pane | `pty::reap` | Until the child exits | Sends `PaneExited` exactly once |
| One debounce + one `notify` watch per open project per window | `watch::start` | Until the `Watcher` handle is dropped | Pushes to its client *and* to the index directly |
| One per clone | `Repos` | The transfer | The git worker's synchronous queue would block for minutes (`D73`) |
| One per connector flow | `Connectors` | The flow | It touches a network |
| One per model probe / suggestion | `dispatch` arms | The probe or `SUGGEST_DEADLINE` | An FFI hop or an HTTP round trip blocks |

A worker cannot borrow `&self`. `Arc<Settings>`, `Arc<FileHarnessCache>`, `Arc<Usage>` and the free
function `probe_catalogue` all exist for that one reason.

**Nothing in the host is bounded by a queue.** `HostEnd`, every `Mailbox` and every pane's byte
stream are unbounded, deliberately: a window that has fallen behind must never stall the reader
draining a harness (`D21`).

## The pane lifecycle

```
SpawnWorkspace ──► resolve_cwd            record → health::probe → files::path::resolve(rel_path)
                     │ refused            ProjectError (naming the project — no pane exists yet)
                     ▼
                   PaneId::generate()
                   Agents::compose / Program::plain
                     │ refused            PaneError + agents.retire(pane_id)
                     ▼
                   pty::spawn(80×24)      TERM/COLORTERM, slave dropped, writer + killer taken
                   owners.insert          ◄── before anything is announced
                   forward_output         reader thread → TerminalOutput
                   pty::reap              reaper thread → PaneExited
                   panes.insert / pane_projects.insert / projects.pane_opened
                     ▼
                   WorkspaceSpawned       the UI draws the pane on the answer, never on the ask

TerminalInput ──► owns() ──► Pty::write            failure ⇒ PaneError
TerminalResize ─► owns() ──► Pty::resize           a resize for a pane that has gone is ignored
Focus ─────────► owns() ──► focused.insert(client, pane_id)

CloseWorkspace ─► owns() ──► owners.remove, panes.remove + kill, focused clear, pane_gone
PaneExited ─────► (the UI closes the tab, which sends CloseWorkspace)
pane_gone ──────► agents.retire(pane_id)   the run directory, credentials seeded into it included
                  login_gone(client, pane)  the only moment "did this login capture anything" exists
                  projects.pane_closed      the picker's count
```

Three ordering rules matter and are easy to break:

1. **The owner goes in before the announcement.** Nothing may arrive about a pane the routing table
   has never heard of.
2. **A failure after `spawn` but before the reader** must kill the child *and* `wait()` it — nothing
   else will ever wait on it — then send `PaneError` and remove the owner.
3. **`agents.retire` happens in `pane_gone`,** which every ending reaches: a close, an exit, and a
   window going away. A harness that exited by itself gets there because the UI answers `PaneExited`
   with `CloseWorkspace`.

## The client lifecycle

**Attach and leave are transport facts, not messages.** They arrive as `FromClient::Connected` and
`FromClient::Gone`, never as a `Message`.

`client_here` sends `HostInfo { config_root, is_default }` — the interface cannot look at its own
config root, and the status bar says so when the run is pointed somewhere unusual — then drains
`pending`, which is whatever the catalogue wanted said before a window existed to hear it.

`client_gone` is a deliberate teardown, in this order:

1. `focused.remove(client)`.
2. Drop that client's watchers — dropping a `Watcher` stops its `notify` handle and ends its
   debounce thread. For each project no *other* client still watches, submit `index::Job::Drop`: an
   index is worth keeping only while somebody has the project open, and the watch map is the only
   record of who has what.
3. For every pane it owned: remove the owner, remove and `kill()` the `Pty`, then `pane_gone`.
   Taking the `Pty` out of the map is most of the work — dropping it closes the master, the kernel
   hangs up the slave and the harness goes; the kill is the guarantee for anything that ignores the
   hang-up.
4. `end_conversation(.., StopReason::Cancelled)` for every conversation it owned.
5. `connectors.client_gone` and `repos.client_gone` — dropping a flow's sender is what stops a
   transfer mid-fetch.

This used to happen by itself: a closed window dropped its whole bus and the coordinator thread went
with it. One host outlives every window, so nothing drops now and every step above has to be
explicit. Without it, every closed window leaves a live harness (`D28`).

## Addressing

| Family | Goes to | Because |
|---|---|---|
| Pane, session, conversation | `To::Client(owner)` | A pane is a running harness on one window's behalf |
| Project catalogue | `To::Everyone` | Every window's picker agrees by construction |
| Listings, preferences, stats, a flow's own stages | `To::Client(asker)` | A sample is a sample, not a broadcast of one window's timing |

A service never holds the bus. It answers `Reply::Asker` / `Reply::Everyone` and `Coordinator::
answer` maps that onto a `To` — which is what makes `Projects` and `Work` testable with no bus at
all. A flow thread that needs both gets them from `Coordinator::sinks`.

## Store durability, in one table

| Store | On a parse failure | On a too-new version | On a write failure |
|---|---|---|---|
| `ProjectStore` | `preserve_aside`, reported as `StoreError::Parse { preserved_as }` | `UnknownVersion`; the file is left exactly as it is | `NotDurable` once, then mutations stay in memory for the session |
| `TaskStore` | Same — it is the catalogue's class, per project | Same, and the project is marked `sealed` so this build never writes it | Warned once per project |
| `PreferenceStore` | The interface owns the schema; the host never reads it | — | A log line |
| `SettingsStore` | Host layer: preserved and reported. Ui layer: opaque | Same as the catalogue | A log line for Ui, an error for Host |
| `FileHarnessCache` | Loads as empty. It is a cache — losing it costs a re-probe | Loads as empty | Degrades to non-durable and carries on |
| `Usage` | The meter is `None` for the run and the AI page is empty | Migrations run on `PRAGMA user_version` | The pump logs it and carries on |

The bargain everywhere: **losing derived data must never cost the user their session, and losing
the user's own data must be loud rather than silent.** That is why a corrupt catalogue is preserved
and a corrupt cache is discarded.

`gc::collect` and `Projects::sweep_ephemeral` both run **only after a load that succeeded**
(`Projects::loaded`). Sweeping against the empty catalogue a corrupt file produces would delete
every project's view state and every ephemeral clone's folder — exactly what preserving the file was
meant to avoid.

## The config root

`config::resolve` answers in this order, and the answer is carried as `RootSource` so the status bar
can say when a run is not in the usual place:

1. `--config-root` (`RootSource::Flag`)
2. `UBIQ_CONFIG_DIR` (`Env`)
3. The nearest `ubiq.toml` walking up — the ascent stops *after* checking a directory holding
   `.git`, so a bootstrap belongs to its own repository (`Bootstrap(path)`)
4. `~/.config/ubiq` (`Default`)

A `ubiq.toml` that will not parse, or that names no `config_root`, is an **error, not a fallback** —
quietly landing in the user's real config directory is the trap the whole mechanism exists to avoid.
A relative `config_root` resolves against the bootstrap's own directory, so a checked-in
`config_root = "_data/config"` means the same thing wherever the repository is cloned. Resolution is
`absolute()`, never `canonicalize()`, because the root is very often the directory this run is about
to create.

The tree it produces is owned by `_docs/tech/project-structure.md` (`## The config root`). The rules
that decide where a new file goes:

- **`cache/` is anything the host can re-derive by asking again.** Deleting it costs a re-probe.
  `harness-models.toml` is the one file in it today.
- **`usage.db` fails that test**, so it sits at the top level beside `projects.toml`: nobody can ask
  a harness what it spent last Tuesday.
- **`projects/<ulid>/` holds `tasks.toml`, `view.toml` and `ui/`.** A `projects/<ulid>/` with no
  record is collected at the next successful load — `forget` drops the record first and the
  directory second, so a crash between the two leaves garbage `gc` collects.
- **Nothing Ubiq remembers goes inside a project's own folder** (`D30`).

## The workarea (architecture rule 6)

Every `ProjectSnapshot` carries a `workarea` — `projects/<ulid>/ui/`. The host makes it, names it,
and that is the end of the host's interest: nothing on this side lists it, reads it or writes to it.
The interface uses the string it was handed and **never composes the path** from
`HostInfo.config_root`, which is what makes a host on another machine a change of value rather than a
change of code. What lives there is disposable; anything worth keeping goes over the bus as a
preference blob. `INDEX_DIR` (`projects/<ulid>/index/`) is the mirror in the other direction — the
host's own, and the interface is never told it exists.

## The one thing that speaks without being asked

`watch::start` takes a project's root, the merged excludes (application-wide plus the record's own)
and a mailbox, and holds a recursive `notify` watch plus a debounce thread. Two scopes come out of
one recursive watch, filtered rather than watched selectively — a selective watch would need
re-registering every time a directory appears:

- anything under `.git/` never reaches `changed`; if it is `HEAD`, `MERGE_HEAD`, `index` or under
  `refs/`, it sets the `repository` flag on the next flush instead;
- everything else is dropped if the project's ignore rules exclude it.

Events coalesce by path over `QUIET` (150ms) and the batch is bounded at `BOUND` (64 paths), past
which it flushes with `truncated` and the reader re-lists the subtree rather than patching named
paths. What crosses is `ProjectFilesChanged` — project-relative paths and the git flag, never
content and never an absolute path.

The same flush feeds the index **directly**: `watch::Job.index` carries the index sender, so the
coordinator never sees a change and has nothing to relay. One extra send per flush, downstream of a
debounce that already coalesced the burst. A watch that will not start is logged and the project
simply has none.

`watch_project` is keyed `(ClientId, ProjectId)` and retains by "same client, different project"
because there is no `CloseProject` message — opening the next project is the only signal the old
watch is unwanted. It also submits `index::Job::Build` when `index_level(project).keeps_text()`: an
index is built when a project opens, not at boot, because an index for a project nobody has open is
work nobody asked for (`D77`).

## Cancellation

`active_searches` and `active_suggests` hold an `Arc<AtomicBool>` per live request, and the flag
means two things at once: *cancel me*, set by `CancelSearch` / `CancelSuggest` or by a second
request for the same project; and *I am over*, set by the worker on its way out. An entry whose flag
is already set is reaped by the next request that mints one — the only place either map is
collected. Nothing is timed except a suggestion (`SUGGEST_DEADLINE`, 60s), which bounds the wait
rather than the work, because generation cannot be interrupted.
