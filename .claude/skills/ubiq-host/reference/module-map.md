# Every module in scope, and its surface

`crates/ubiq-host/src/lib.rs` is the index — its `//!` header lists every module with one line
each. This file covers the ones this skill owns. `agent.rs` and `conversation.rs` belong to
**ubiq-agents**; `remote.rs` to the hub/communication skill; `files/`, `search/`, `index/`,
`connectors/`, `git/`, `repos/` and `assist/` to their own.

## `coordinator.rs` — the run loop (~3600 lines, the centre)

`pub fn start(host, root, projects, work, settings, pending)` spawns the thread named
`ubiq-coordinator` and is the only public item. Everything else is private to `Coordinator`.

### Constants

| Constant | Value | Why |
|---|---|---|
| `INITIAL_COLS` / `INITIAL_ROWS` | `80` / `24` | The harness must start before the emulator has bounds; `TerminalResize` corrects it a frame later |
| `CONVERSATION_POLL` | 500ms | A harness that ends on its own sets a flag and sends nothing, so the loop's wait is bounded while any conversation or clone is live |
| `SUGGEST_DEADLINE` | 60s | Generation cannot be interrupted, so this bounds the wait rather than the work |

### The state it holds

| Field | Is |
|---|---|
| `host: HostEnd` | The one end every client is read through, and every answer addressed on |
| `root: ConfigRoot` | The resolved config root and which of the four answers produced it |
| `projects`, `work`, `settings` | The catalogue, a project's tasks, the two settings layers (`Arc`, because flow threads write the same record) |
| `connectors`, `repos`, `agents`, `ai_providers`, `assist` | The service objects; each does its slow work on a thread of its own |
| `catalogue: Arc<FileHarnessCache>` | The on-disk model/reasoning cache, keyed on the harness binary's version string (`D60`) |
| `files`, `git`, `search`, `index` | Four worker threads, `start()`ed in `new()` and alive for the process |
| `panes: HashMap<PaneId, Pty>` | The pseudo-terminals |
| `owners: HashMap<PaneId, ClientId>` | **The whole routing table.** Everything a pane emits goes to its owner; nobody else may drive it |
| `pane_projects: HashMap<PaneId, ProjectId>` | So a pane opening or closing changes the picker's count |
| `focused: HashMap<ClientId, PaneId>` | One focused pane *per window* — two windows have two, and neither is more focused |
| `watchers: HashMap<(ClientId, ProjectId), Watcher>` | One filesystem watch per window per open project |
| `conversations`, `conversation_owners`, `pending_conversations` | The agents skill's; the same two routing reasons as panes |
| `logins: HashMap<PaneId, PendingLogin>` | Whether a login captured anything is only answerable once its process exits |
| `active_searches`, `active_suggests` | `Arc<AtomicBool>` cancel flags, doubling as "this is over" — the worker sets the flag on its way out, and the next request reaps set entries |
| `started: Instant` | Uptime, measured from the coordinator's thread rather than from `main` |
| `agents_this_run: usize` | A counter, not a length: what it counts has gone |
| `usage: Option<Arc<Usage>>` | `None` when the database would not open — an unwritable root costs token history, never the session |
| `pending: Vec<Reply>` | What the catalogue wanted said before a window existed; delivered to the first attach |

### The methods worth knowing

| Method | Does |
|---|---|
| `run` | The loop. Wait → one event → `reap_conversations` → `register_clones` → `flush_due` |
| `dispatch` | The `match` over `Message`, family by family. Its `other` arm warns rather than dropping |
| `client_here` | Sends `HostInfo { config_root, is_default }`, then drains `pending` |
| `client_gone` | The deliberate teardown — see `runtime.md` |
| `owns` | The gate on every pane message |
| `answer` | `Vec<Reply>` → `To::Everyone` or `To::Client` |
| `sinks` | The two mailboxes a flow thread gets: the asker, and everyone |
| `stats` | One `HostStats` sampled here and now; usage read failures are a log line and empty rows |
| `spawn_workspace` | The spawn path, in order — below |
| `resolve_cwd` / `refuse_spawn` | Record lookup, `health::probe`, `files::path::resolve` of `rel_path`; a refusal re-probes when the folder itself was the reason |
| `pane_gone` | `agents.retire(pane_id)` (the run directory), `login_gone`, then `projects.pane_closed` |
| `watch_project` / `index_level` / `settle_index` | The watch and the index for one open project |
| `register_clones` | Drains finished clones into the catalogue — a clone's own thread cannot touch `Projects` |

### The spawn path, in order

`resolve_cwd` (record → `health::probe` → `rel_path`) → mint `PaneId` → `agent_type` or
`shells::default_program()` → `Agents::compose` when the library knows the type, else
`Program::plain` → `Composed::exec` → `pty::spawn` at 80×24 → **insert the owner** →
`forward_output` on a mailbox → `pty::reap` → `panes.insert` → `projects.pane_opened` →
`WorkspaceSpawned`. Every failure before the owner insert answers `PaneError` or `ProjectError`
and retires the run directory.

## `pty/mod.rs` — the only place a descriptor or a process lives

| Item | Is |
|---|---|
| `READ_CHUNK` | 8 KiB. Larger chunks mean fewer messages under a flood |
| `struct Pty` | `master`, `writer`, `killer` — nothing else |
| `struct Program` | `program`, `args`, `env`, `env_remove`, `env_clear`. `Program::plain` is the shell case |
| `spawn(program, folder, cols, rows)` | Opens the pair, sets `TERM=xterm-256color` and `COLORTERM=truecolor` **before** the program's own env (so a confined run keeps its answer), spawns, **drops the slave**, takes the writer and a killer |
| `Pty::forward_output(pane_id, out, scan_links)` | The reader thread. `scan_links` is on only for a login pane. A send that is not delivered stops the reader — nobody is drawing this pane |
| `Pty::write` / `Pty::resize` / `Pty::kill` | Keystrokes in; geometry to the kernel; the guarantee for a harness that ignores the hang-up |
| `reap(pane_id, child, out)` | A thread that waits and sends `PaneExited { code }` once. `-1` when the wait itself failed |
| `command_for` (private) | A no-argument shell becomes a login shell via `new_default_prog` + `SHELL`; everything else is built plainly |
| `spawn_cwd` (private) | Strips Windows' `\\?\` and `\\?\UNC\` verbatim prefixes — file operations want them, `cmd.exe` refuses them as a working directory |

## `projects.rs` — the catalogue as the host runs it

| Item | Is |
|---|---|
| `WORKAREA = "ui"` | The directory inside a project's own that belongs to the *interface*. The host makes it, names it on `ProjectSnapshot.workarea`, and never looks inside |
| `INDEX_DIR = "index"` | The mirror: the host's own, and the interface is never told it exists |
| `DEBOUNCE = 400ms` | A panel drag fires continuously; preferences coalesce per scope. Long enough that a drag is one write, short enough that quitting straight after keeps it |
| `Projects::open` | Loads the catalogue; a corrupt file is preserved and reported through `pending` |
| `list` / `list_projects` / `records` / `record` | The projection and the `Reply` carrying it |
| `add` / `forget` / `update` / `locate` / `opened` / `refresh` | The project family. `forget` drops the record first and the directory second — `gc` collects a crash between the two |
| `workarea(id)` / `index_dir(id)` | The two reserved directories under `projects/<ulid>/` |
| `point_ephemeral_at` / `sweep_ephemeral` | The one tree an ephemeral clone's folder may be deleted from (`D74`); swept once at startup |
| `get_preferences` / `set_preferences` / `next_due` / `flush_due` / `flush` | The debounce, driven by the run loop |
| `pane_opened` / `pane_closed` / `open_count` | The count only this half can know; `open_count` is a `stats()` reading |

Colour arrives from the interface — the palette is the interface's, so `add` and `update` take one
rather than choosing (`D31`).

## `store/`

Four traits in `store/mod.rs`, and two concrete types beside them:

| Trait / type | Durability rule |
|---|---|
| `ProjectStore` | The host's to parse and act on. Corrupt ⇒ preserved aside and reported |
| `TaskStore` | The catalogue's class, per project so per file. `load` returning `None` means *never written*, which is not the same as no tasks — the seeding rule turns on it |
| `PreferenceStore` | Opaque. The host stores a string it never reads; a failed write is a log line |
| `SettingsStore` | Two layers: `Ui` opaque, `Host` parsed (JSON on the wire, TOML on disk) — `D46` |
| `FileHarnessCache` (`store/harness.rs`) | A *cache*, not a catalogue: missing, corrupt or too-new loads as empty and the next probe rewrites it |
| `Usage` (`store/usage.rs`) | A database, because it accumulates and is unbounded — `D78` |

`StoreError` has four shapes, and the distinctions are load-bearing: `Io`, `Parse { preserved_as }`
(the file has already been moved aside), `UnknownVersion { found, supported }` (**not** corruption —
the file is left exactly as it is), and `NotDurable` (an earlier write failed; mutations still apply
in memory and the user is told once).

Versions: `CATALOGUE_VERSION`, `TASKS_VERSION`, `PREFERENCE_VERSION`, `SETTINGS_ENVELOPE_VERSION`
and `HARNESS_CACHE_VERSION` are all `1` today.

`store/memory.rs` is the fallback for an unwritable config root as well as the test double, which is
why every memory store carries `fail_writes`, `fail_load` and a `writes()` counter.

`store/usage.rs`: `HOUR = 3600`, `MINUTE = 60`, `MIGRATIONS` as a const array whose index is the
version (applied on `PRAGMA user_version`). `open` creates the file, sets WAL, migrates and empties
the minute table; `record`, `history(since)` and `this_run()` are the rest. Every dimension column is
`TEXT NOT NULL` with `''` as "not said" — SQLite treats two `NULL`s as distinct in a unique index,
so a nullable column would silently stop the accumulating upsert from accumulating.

## `work/`

| Item | Is |
|---|---|
| `Work` | Tasks (durable, per project, `tasks.toml`) plus `mocks`, `live` agents and `live_sessions` (invented or ephemeral, never written) |
| `open` / `forget` / `list` | The work family's entry points |
| `create`, `update`, `move_task`, `assign`, `delete` | Task edits (`D40` — one infallible `UpdateTask`, with move and assign as their own messages) |
| `add_step`, `rename_step`, `remove_step`, `move_step`, `toggle_step` | Steps |
| `assign_agent`, `send_to_agent` | Into the mock thread — nothing answers (`G52`) |
| `add_live_agent`, `remove_live_agent`, `live_agent_names` | The real agents, which sit in the same lists as the mocks (`G94`) |
| `sealed`, `warned` | A project whose file this build may not write; and telling the user once |
| `work/mock.rs` | `sessions()`, `tasks()`, `agents()` — five sessions and eleven agents behind **ULID literals**, because a seeded task points at a session id forever and a per-boot ULID would empty every session pill on the second run. Two projects therefore share mock ids (`G48`). `tasks()` is a *seed*: editing a row changes what a new project starts with, never what an existing one holds |

The whole shape is `D39`: the work is the host's, tasks are written down per project, and sessions
and agents stay its mocks.

## The small modules

| Module | Holds | Never holds |
|---|---|---|
| `config.rs` | `ConfigRoot { path, source }`, `RootSource` (`Flag` / `Env` / `Bootstrap(path)` / `Default`), `BOOTSTRAP = "ubiq.toml"`, `default_config_root`, `discover_bootstrap`, `resolve`, `resolve_from_env` | A second answer to a question a store already answers |
| `atomic.rs` | `write_atomic`, `write_atomic_with(mode)`, `preserve_aside(path, now)` | A write that is not temp-fsync-rename in the same directory |
| `reply.rs` | `Reply::Asker` / `Reply::Everyone`, `message`, `is_broadcast`, `into_message` | The bus itself |
| `gc.rs` | `orphans(root, keep)`, `collect(root, keep)` over `projects/<ulid>/` | A deletion of a directory whose name is not a ULID, or a run after a failed load |
| `health.rs` | `probe(path) -> ProjectHealth` — one `symlink_metadata`, so a broken link is `NotADirectory` (a fact) and not `Missing` (which would invite a Locate that cannot help) | A second stat per record at boot |
| `shells.rs` | `CANDIDATES`, `EXTRA_DIRS`, `default_program`, `available`, `is_shell`, `repair_path` | A launcher for anything on disk — the set is fixed and bounded |
| `cli_shortcut.rs` | `handle(action)` over `Query` / `Install` / `Remove`; `MARKER = "ubiq-target:"`, `NAME` (`ubiq.cmd` on Windows) | A path parameter — nothing it exposes takes one |
| `links.rs` | `LinkScanner::new` / `feed`, `TAIL_CAP = 4 KiB`, `SEEN_CAP = 16` | A URL parser or a VT parser |
| `watch/mod.rs` | `Job { project_id, root, excludes, index, reply_to }`, `Watcher`, `start(job)`, `QUIET = 150ms`, `BOUND = 64` | An absolute path on the wire, or an opinion about what a reader redraws |
| `mcp_server.rs` | A doc-comment and a TODO. **Nothing is implemented** | — |

`shells::repair_path` is `D62`: a desktop-launched host asks the user's login shell for its `PATH`
once, with `-lic`, because the environment a Finder launch inherits is exactly the one that cannot
be trusted. `available()` checks `PATH`, that repaired `PATH` and `EXTRA_DIRS`, and always includes
`default_program()` whatever it is.

`cli_shortcut` writes a *script*, not a symlink (`D56`): on macOS it runs `open -a Ubiq.app` so
`ubiq .` reaches the running window through LaunchServices instead of starting a second application.

## Known gaps in this area

| Gap | Is |
|---|---|
| `G19`, `G60` | The session family is `SpawnWorkspace` / `WorkspaceSpawned` / `CloseWorkspace` and nothing else. `CreateSession`, `AttachToSession`, `ListSessions` and their records are in the transport contract and in no code |
| `G38` | `spawn_workspace` probes a folder and resolves a path on the coordinator's thread, which every pane's keystrokes pass through |
| `G46` | The catalogue, view state and tasks are written from the run loop, so a config root on a network mount blocks it |
| `G30` | Health is probed at load, on open and on request; the watch never re-probes the record |
| `G47` | The host answers work messages and never pushes one — nothing tells a window an agent moved on its own |
| `G48` | Mock session and agent ids are the same literals in every project |
| `G53` | `store/` is in no document's `code_anchors`, so a change to a trait or an on-disk format is told it owes nothing |
| `G94` | Live agents sit beside mock ones in the same list |
