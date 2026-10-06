---
id: wip-codeorg-host
title: Code organisation of the headless side — snapshot and pain points
kind: wip
status: current
summary: A counted snapshot of ubiq-host, ubiq-proto, ubiq-app and ubiq-drone (module tree, message set, concurrency, stores, security surface, placeholders, errors, tests, the drone) and a severity-ranked list of pain points with a one-line remedy each. Read-only analysis; nothing was changed.
read_when: you are deciding what to refactor or harden in the host, the protocol or the drone
updated: 2026-10-05
depends_on: [tech-architecture, tech-transport, feat-drone, wip-drone]
---

# Headless side: snapshot (A) and pain points (B)

Counts are `wc`/`grep` over the tree at `0bb6e16` (plus uncommitted UI edits). "Production" counts stop
at the first `#[cfg(test)]` of each file, so inline test code does not inflate them. Paths are under
`crates/`.

---

# Section A - Factual snapshot

## A1. Modules and sizes

| Crate | src lines | tests/ lines | files | `#[test]` | prod `unwrap()` | prod `expect(` | `pub` items / `pub(crate)` |
|---|---|---|---|---|---|---|---|
| ubiq-host | 69,580 | 17,250 | 153 | 1,070 | 21 (1,804 incl. tests) | 31 (497) | 1,113 / 31 |
| ubiq-proto | 17,182 | 1,783 | 45 | 152 | 0 (124) | 1 (35) | 616 / 0 |
| ubiq-app | 1,655 | 0 | 4 | 21 | 0 (16, all in `handoff` tests) | 1 | 15 / 0 |
| ubiq-drone | 3,518 | 1,576 | 15 | 33 | 2 | 22 (mostly `thread::Builder...expect` and mutex poison) | 68 / 0 |

`ubiq-host/src/lib.rs`: 47 `pub mod`, **zero private modules**. Features: `full` (default) = `git`,
`index`, `harness`, `listener`, `db`, `desktop`; the drone links the host with `default-features = false`
(`ubiq-drone/Cargo.toml`). The `listener` feature bundles the inbound TCP/TLS server (`remote`) with
outbound HTTP clients (`connectors`, `web_assets`, ureq/rustls) - one name for two things.

### ubiq-host source by top-level module (lines)

| Lines | Module | Lines | Module |
|---|---|---|---|
| 10,874 | `mcp/` (11 files) | 2,209 | `mission/` |
| 8,587 | `coordinator.rs` (7,029 prod + ~1,560 inline tests) | 2,178 | `connectors/` |
| 4,809 | `db/` | 1,988 | `conversation.rs` |
| 3,775 | `store/` | 1,957 | `files/` |
| 3,655 | `agent.rs` | 1,896 | `web_assets/` |
| 3,626 | `tasksrc/` | 1,699 | `work/` (incl. 346 of `mock.rs`) |
| 3,490 | `plan/` | 1,110 | `search/` |
| 2,667 | `git/` | 1,084 | `projects.rs` |
| 2,316 | `assist/` | <1,000 | the other ~25 (index, kb, repos, shells, armed, catalog, help, runners, feedback, remote, ...) |

### 20 largest files (all four crates, src + tests)

| Lines | File |
|---|---|
| 8,587 | ubiq-host/src/coordinator.rs |
| 4,075 | ubiq-proto/src/messages.rs |
| 3,655 | ubiq-host/src/agent.rs |
| 2,471 | ubiq-host/tests/coordinator.rs |
| 2,296 | ubiq-host/src/plan/service.rs |
| 2,251 | ubiq-host/tests/mission.rs |
| 2,023 | ubiq-host/src/mcp/server.rs |
| 1,988 | ubiq-host/src/conversation.rs |
| 1,941 | ubiq-host/src/mcp/mission.rs |
| 1,901 | ubiq-host/tests/work.rs |
| 1,803 | ubiq-host/src/mission/mod.rs |
| 1,795 | ubiq-host/tests/git.rs |
| 1,452 | ubiq-host/src/mcp/catalogue.rs |
| 1,369 | ubiq-host/src/mcp/tasks.rs |
| 1,353 | ubiq-host/src/work/mod.rs |
| 1,208 | ubiq-drone/src/relay.rs |
| 1,182 | ubiq-app/src/lib.rs |
| 1,153 | ubiq-host/tests/files.rs |
| 1,112 | ubiq-host/src/db/tests.rs |
| 1,105 | ubiq-host/src/files/mod.rs |

Files over 2,000 lines: **7 in src** (`coordinator`, `messages`, `agent`, `plan/service`, `mcp/server`
at 2,023, and the two test files `tests/coordinator.rs`, `tests/mission.rs`; `conversation.rs` and
`mcp/mission.rs` sit just under). Largest functions: `Coordinator::dispatch` **2,096 lines**
(`coordinator.rs:1503-3598`, 183 `Message::` arms), `Coordinator::new` 307, `compose_run` 297
(`agent.rs`), `start_conversation` 264, `conversation::pump` 238, `launch` 206, `spawn_workspace` 194.
The `Coordinator` struct has 57 fields (`coordinator.rs:165-395`).

### ubiq-proto modules
45 files; `messages.rs` (4,075) is the enum, the rest are per-family payload modules: `tasksrc` 1,061,
`mission` 909, `bus` 856, `work` 800, `connectors` 619, `db` 618, `settings` 580, `conversation` 567,
`projects` 557, `git` 511, `assist` 490, `log` 443, `notifications` 431, `carrier` 418, `plan` 390,
`ids` 386, `files` 385, `wire` 339, `kb` 326, `drone/` 336. It is the contract plus the in-process bus
(`bus.rs`: Hub/HostEnd/Client/Mailbox/Voice/Tape) plus the log sink (`log.rs`) plus wire framing.

### ubiq-app
`lib.rs` 1,182 (boot: argv parsing by hand, `--serve`/`--bind`/`--port`/`--tls-*`, Windows console
FFI with 9 `unsafe`, `announce`), `handoff.rs` 468 (single-instance door: unix socket / Windows
loopback port + token file), `main.rs` 5, `build.rs` 41. It is the only crate naming both halves.

### Layering as it is
```
ubiq-app boot -> coordinator::start (one thread "ubiq-coordinator", handle dropped, coordinator.rs:144)
  Coordinator (single struct, 57 fields): panes HashMap<PaneId,Pty>, owners routing table,
    conversations, watchers, 4 workers (Files/Git/Search/Index), tasksrc sync thread,
    stores: Projects (catalogue) / Work / Settings behind 4 traits (store/mod.rs, file+memory impls),
    mcp::Serving (own loopback listener thread), assist, quota, connectors, repos, kb, plan, mission, db
  services (projects.rs, work/, plan/, mission/, kb/, tasksrc/) return Vec<Reply>; Coordinator::answer addresses
```
Services are testable without a bus (good). Business logic of ~15 families still lives in
`dispatch` arms or `Coordinator` methods rather than in the service (`launch`, `spawn_workspace`,
`start_conversation`, `suggest_job`, ... in `coordinator.rs`).

### Visibility habits
Everything is `pub`: 1,113 pub items vs 31 `pub(crate)`; no `mod` is private at the crate root;
`#[allow(clippy::too_many_arguments)]` x24, 33 `allow(` total in ubiq-host. No `[workspace.lints]`.
Doc comments are long and rationale-heavy (the code is unusually well explained, which also makes
files large).

## A2. The message set

* `ubiq-proto/src/messages.rs:72` `pub enum Message`, `#[serde(tag="type", content="payload")]`,
  **~350 variants** in one enum, ~4,000 lines. Grouped only by `// ── X family: UI -> host ──` comment
  banners (about 70 banners): pane, session, account, quota, feedback, agent definition, skills/MCP
  catalog, connector, repository, task-source, project, CLI shortcut, shell integration, host browse,
  file, git, knowledge-base, database, work, plan, mission, conversation, search, assist, notification,
  web asset, help, carrier (4 transport-only variants). Payload structs live in per-family modules.
* **Direction is documentation only.** No type separates UI->host from host->UI; the coordinator's
  last `dispatch` arm "names" response-direction variants and warns (`SKILL ubiq-host`).
* **Encoding**: 4-byte BE length + MessagePack via `rmp_serde::to_vec_named` (`wire.rs`), so struct
  fields are maps and `#[serde(default)]` additions are compatible (77 `serde(default` + 32
  `skip_serializing_if` in `messages.rs`; 0 `deny_unknown_fields` in it). `MAX_FRAME` = 64 MiB
  (`wire.rs:26`), checked before allocation.
* **Versioning**: `MESSAGE_SCHEMA = 1` (`wire.rs:48`) is checked **only** in the drone handshake
  (`DroneHello/DroneReady`, `carrier.rs`) and only covers what a drone speaks. The `--serve` TCP
  attach (`remote.rs`) has **no version or capability exchange at all**. Nothing verifies a bump
  mechanically (acknowledged in the constant's own doc; backlog G256). In-process the two halves
  are one build, so skew only exists across the TCP and ssh carriers.
* **Skew behaviour**: `carrier::pump` reads `while let Ok(message) = wire::read_frame(..)`
  (`ubiq-host/src/carrier.rs:140`); a frame that fails to decode (unknown variant from a newer peer)
  ends the whole session exactly like EOF. Writer side: any `write_frame` error (including an encode
  failure of one message) also ends it (`carrier.rs:~105`).
* **Cost of adding a message** (per `_docs/tech/transport-contract.md:3252` "Adding a variant"):
  (1) variant in `messages.rs` (+ payload type in a family module), (2) table row in the 3,312-line
  transport-contract doc, (3) arm in `Coordinator::dispatch` (+ usually a method/job), (4) UI handler
  arm in `crates/ubiq/src/app/wire.rs` (3,661 lines, 140 `Message::` arms) and often an
  `AppState` field, (5) if it carries a `pane_id`, add to `pane_id_of`
  (`crates/ubiq/src/app/hosts.rs:207`, **catch-all returns "no pane"** - an omission misroutes only
  with 2+ hosts), (6) decisions row if structural. The drone relay needs nothing (catch-all refuses
  typed; seven messages have no error variant and are dropped with a warn). So **5-6 places, two of
  them silent-failure traps** (pane_id_of, drone silent drops). No exhaustive per-variant serde
  round-trip test (wire.rs has 8 tests).
* **Routing** (`bus.rs`): `Hub::connect` -> `Client`; host reads one `HostEnd` of `FromClient::{Connected,
  Said, Gone}`; replies addressed `To::Client(id)` or `To::Everyone`; `Mailbox`/`MovingAddress`/`Voice`
  (host talking to itself, `is_host_voice`). Ownership: `owners: HashMap<PaneId,ClientId>`,
  `owns()` gates every pane message. All channels are `flume::unbounded` (`bus.rs:106,163,349,350`).
* **Bus tape** (`bus.rs:564-790`): a global 500-entry ring of every message as JSON (8 KiB cap per
  entry), dumpable to `$TMPDIR/ubiq-tape-<secs>.jsonl` or streamed to `$UBIQ_TAPE_DIR`.

## A3. Concurrency model

* **Threads, no async runtime.** Zero `tokio`/`async fn` in any of the four crates. Channels: flume
  (16 `unbounded` in host, 8 bounded/sync_channel), 9 `std::sync::mpsc`. 126 `Mutex` mentions in host,
  49 `AtomicBool`.
* **Coordinator**: one thread, `run` loop (`coordinator.rs:1205`): `recv_timeout(wait)` with wait
  capped by `CONVERSATION_POLL` 500 ms and `TASK_SYNC_EVERY` 2 s (the loop wakes at least every 2 s
  forever), then `dispatch` and 7 housekeeping calls (`collect_scans`, `sync_tasks_due`,
  `remember_sessions`, `name_conversations`, `adopt_harness_titles`, `reap_conversations`,
  `register_clones`, `projects.flush_due`). On disconnect it flushes projects and returns.
* **Reader never blocked by UI**: per-pane reader thread -> unbounded mailbox (`Pty::forward_output`).
* **Workers**: `Files`, `Git`, `Search`, `Index` are each `start()` -> `(Sender<Job>, thread)` with
  `flume::unbounded` and `submit()` that logs on send failure (files/mod.rs:851, git/mod.rs:84,
  search/mod.rs:37, index/mod.rs:67): the same ~12 lines copy-pasted four times. `watch::start` returns
  a handle whose drop stops it. tasksrc has its own `Sync` handle thread. The rest are **one-off
  `thread::Builder` threads**: 42 sites in host (coordinator.rs alone has 9: lines 154, 3795, 4919,
  5625, 5651, 5734, 5752, 6088/6102, 6484) with ad hoc names; clone, kb sync, web_assets, connector
  flows, repos, quota, feedback, suggest, naming each roll their own. MCP: one `ubiq-mcp` accept
  thread handling requests inline, plus thread-per-call for ask and SQL (SQL capped by `MAX_CALLS`).
* **Not uniform**: panic handling, naming, queue bounds, stop mechanism (handle drop vs `AtomicBool` vs
  channel drop) all differ per site.
* **Panics**: `catch_unwind` appears only in `host_meta.rs` (2). No panic hook (`panic::set_hook`
  absent in ubiq-app). `coordinator::start` discards the `JoinHandle` (`coordinator.rs:154`). A panic
  in the coordinator or a worker thread kills just that thread; the process, the window and the
  remaining threads stay up. `Files::submit` etc. then only `tracing::error!("the files thread has
  gone; a request was dropped")` - every later request is silently dropped.
* **Blocking on the coordinator**: `dispatch` contains no direct `std::fs`/`Command`/`sleep` (one
  `remove_dir_all` at 3472). The known exceptions are the three file-backed stores (G46:
  `Projects`/`Work`/`Settings` read/write through `store::file` inline, with fsync, on the coordinator
  thread) and the spawn path's folder probe (G38).
* **Shutdown**: no explicit host shutdown. `Pty` has no `Drop`; panes die when the master fds close
  at process exit (SIGHUP) or via `client_gone` (explicit teardown, five ordered steps, documented
  in `reference/runtime.md`). Workers end when their sender is dropped. Remote carrier: writer polls
  `STOP_POLL` 200 ms; heartbeat 20 s ping, 3 misses = gone.
* **Remote sessions**: `remote.rs` is thread-per-connection (`accept_loop`, line 240) with an 8 KiB
  header cap and 10 s handshake deadline, no connection cap; then `carrier::pump` = 2 threads per
  session over an unbounded per-client channel.

## A4. Persistence

Config root resolved flag > `UBIQ_CONFIG_DIR` > nearest `ubiq.toml` > `~/.config/ubiq`
(`config.rs`). On disk (observed in `_data/config` and via grep): `projects.toml`, `preferences.toml`,
`settings`/`host-settings.toml`/`ui-settings.toml`, per-project dir (`project.toml`, `tasks.toml`,
`view.toml`, `kb.toml`, `db.toml`, `tasksrc.toml`, `mission.toml`, plan sidecars, `notes.md`...),
`accounts/`, `profiles/`, `environment.toml`, `harness-models.toml`, `ai-models.json`, `catalog.json`,
`usage.db` (SQLite), per-conversation `conversation.json`, conversation `store.jsonl`, `keychain/`
(secret index), search index dirs, `ui/` shared workarea.

* **Atomic writes**: `atomic::write_atomic` (temp sibling `.name.pid.counter.tmp`, `sync_all`, rename,
  dir sync) - 45 production call sites. Mode: temp file is created with default umask (not 0600);
  `write_atomic_with` carries an existing mode for user files. Raw `fs::write` remains only in
  `help/mod.rs:236` (derived data), `web_assets` (`.part` download), append logs (`conversation.rs:208`,
  `store/mission.rs:125`, `log.rs:354`), `bus.rs:751` (tape dump).
* **Corruption**: parse failure -> `atomic::preserve_aside` moves the file to `*.corrupt-<stamp>`;
  `gc::collect` only after a successful load. **Versioning**: every envelope has `version: u32 = 1`
  (`CATALOGUE_VERSION`, `TASKS_VERSION`, `PREFERENCE_VERSION`, `SETTINGS_ENVELOPE_VERSION`,
  `MISSION_VERSION`, `HARNESS_CACHE_VERSION`, `ANNOTATIONS_VERSION`: all 1). `StoreError::UnknownVersion`
  refuses a too-new file (work seals the project). **No migration code exists**; the only real
  migrations are ad hoc (`project_dir::migrate_project`, `agent.rs` profiles rename). Mission
  records are "made lazily, never migrated".
* **Credentials**: stored through `agent_manager::credentials::OsSecretStore` (OS keychain; plaintext
  engine deliberately bypassed, `connectors/store.rs`); records hold references. DB passwords sealed
  with a keychain key. `Secret` newtype has redacting `Debug` but **`#[serde(transparent)]`**
  (`messages.rs:3944`).
* **Concurrency of writers**: no file locking anywhere in host/app (`flock`/`fs2`/lockfile: 0
  hits). Every store holds the whole file in memory and rewrites it wholesale (projects debounce
  400 ms, `projects.rs:74`). So last-writer-wins (G29) is not just `projects.toml`: it applies to
  every store file. `ubiq-app/handoff.rs` stops a second instance only of the **same executable
  path** (the socket name hashes `current_exe()`), so `ubiq` + `ubiq-studio` + `--serve` hosts on one
  root are all allowed (AGENTS.md just says "never run both").
* **Drone**: `state.rs` JSON state file next to the socket (written via `File::create` of a tmp, 5 s
  refresh); scrollback ring in memory only.

## A5. Security surface (verified in code)

| Surface | What it is | Verdict |
|---|---|---|
| Process spawn (`pty/mod.rs:139-147`, `agent.rs`) | `CommandBuilder::new(program)` + `.arg()` per arg; no `sh -c` strings; env via `env`/`env_remove`/`env_clear`; login shells via `new_default_prog` | No injection. The program name is whatever the client sent, by design |
| Other `Command` sites | `kb/ops.rs:160-182` (explorer/open/xdg-open with `.arg(path)`), `runners/mod.rs:175`, `shells.rs:413`, `search/fallback.rs:62` | fine except below |
| `search/fallback.rs:82` | `ag` branch: `command.arg(&job.query.text)` with **no `--`**, a query starting with `-` is parsed as a flag (`ag --pager=<cmd>` executes a program). grep branch uses `-e`, drone `search.rs:461-492` uses `--` correctly | Low today (query is user-typed), Medium once a drone/agent tool feeds model text |
| File path safety | `files/path.rs`: textual component refusal + `canonicalize` + containment, leaf symlink refused on write, 64-component cap. Used by files/kb/diff/search/plan/coordinator (29 call sites). TOCTOU between check and `fs::write` remains | Good |
| Host browse / `HostWrite` | `BrowseHostDir`, `JobKind::HostWrite` take absolute host paths with no project | By design, but a remote client reaches them (see next) |
| Remote host `--serve` (`remote.rs`, `ubiq-app/lib.rs:355-430`) | TCP, **bare `--serve` binds every interface**; plaintext HTTP unless `--tls-cert/--tls-key`; single process-lifetime 256-bit token in `GET /attach?token=`; constant-time compare; 401 otherwise. A holder gets the **entire** message set: `SpawnWorkspace` with arbitrary program+args (a shell), host browse/write, secrets messages, git write. `announce()` says so | Authenticated but all-or-nothing and plaintext by default; token in URL; no rate limit, no connection cap, no per-client scope, no token rotation/expiry |
| Drone transport | stdio over `ssh` (auth = ssh) or unix socket `0600` in dir `0700` (`socket.rs:300-320`). No auth inside the protocol. Hash-pinned (SHA-256, not signed) binaries; manifest is **empty** | Reasonable; fallback to `temp_dir()` when no XDG/HOME, and `set_permissions(parent)` result ignored (`socket.rs:307`) |
| ssh argv (`crates/ubiq/src/app/ssh_connect.rs`) | target refused if empty/starts with `-`/control chars (305, 717); user passed via `-l`; `BatchMode=yes` when no secret; `SSH_ASKPASS_REQUIRE=force` helper | Good |
| MCP servers (`mcp/server.rs`) | tiny_http on `127.0.0.1:0`, one port for all agents; path `/mcps/<agent-ulid>/<name>`; **no auth beyond loopback + unguessable id** (documented); no Origin/Host check; request body `read_to_string` unbounded (`server.rs:222`); one accept thread handles requests inline. 12 servers: test, project-info, manage-ubiq-tasks, use-task, ubiq-plan, ubiq-mission, use-mission, ubiq-kb, ubiq-help, ubiq-ask, ubiq-sql-read, **ubiq-sql-write** (`execute`), per-profile opt-in | A local process that learns an agent id (visible to other processes only if the URL leaks) can write tasks/plans/KB/SQL. Loopback DNS-rebinding unmitigated. Bounded blast radius, but unauthenticated |
| OAuth redirect | fixed `127.0.0.1:47821` (`ubiq-proto/connectors.rs:33`, `flow.rs:330`), PKCE S256 + state | Fine; fixed port = `PortBusy` refusal if two flows |
| Web panels / assets | `web_assets/`: vendor bundles fetched once, verified against in-source manifest hashes, served by UI from own origin | Not deeply reviewed (UI crate) |
| Credentials | OS keychain; `Secret` redacts Debug | **Bus tape records `serde_json::to_value(message)` for every message (`bus.rs:624-647`), and `Secret` serialises transparently** -> pasted tokens, DB passwords, ssh passphrases enter the in-memory ring and any dump to `$TMPDIR` (default perms, predictable name, `fs::write` follows symlinks) or `$UBIQ_TAPE_DIR` |
| Baked-in secrets | `feedback/mod.rs:35-40` `option_env!("UBIQ_FEEDBACK_API_KEY")`, `UBIQ_FEEDBACK_GITHUB_TOKEN`, OAuth client ids | A token compiled into a distributed binary is extractable with `strings`; scope decides impact |
| Windows handoff token | `RandomState`-derived 128 bit, loopback | Adequate |

## A6. Hardcoded / placeholder values

* `TODO|FIXME|XXX|todo!|unimplemented!` in src: **0 real hits** (only `ADD_TODO` tool constants);
  `panic!`-class in production: host 4, proto 0, app 0 (tests excluded). The project tracks gaps in
  `_docs/backlog.md` instead of in code.
* **Mock data in production**: `work/mock.rs` (346 lines) - "the work the host invents": five fixed
  mock sessions and eleven mock agents with literal ULIDs, minted for **every project** in
  `Work::mock` (`work/mod.rs:230-241`), still answered through `prepare()` on every public method
  (G48, G52, G94). A real user's project list gets fake agents on the Teams-old canvas; fake
  `SendToAgent` appends to a fake thread.
* **Stubs by design**: `assist/stub.rs` (no on-device model, honest "unavailable"); in-memory store
  fallbacks (`store/memory.rs`, documented).
* **Placeholders**: `placeholder` 10 hits in host - all legitimate (Windows registry `%1`, UI text).
* **Fixed endpoints**: OAuth redirect port 47821; `127.0.0.1:0` MCP; remote default port
  `SERVE_PORT` (7420 by the tests) bound on all interfaces; provider URLs (`api.github.com`,
  `gitlab.com`, `api.trello.com`, `api.openai.com/v1`, `api.anthropic.com`) in `connectors/providers.rs`
  and `assist/api.rs:221`; MCP protocol version string `"2024-11-05"` (`mcp/server.rs:51`);
  `/bin/sh` fallback `shells.rs:50`; shell candidate list `zsh,bash,fish,sh` (`shells.rs:24`).
* **Named magic numbers**: 169 `const` Duration/usize/u64 in the three library crates, all documented
  and none centralised or configurable. Examples: coordinator `CONVERSATION_POLL` 500 ms,
  `TASK_SYNC_EVERY` 2 s, `RUNNER_FRESH` 3 s, `SUGGEST_DEADLINE` 60 s; `HANDSHAKE_TIMEOUT` 10 s,
  `MAX_HEADER` 8 KiB (remote); `MAX_FRAME` 64 MiB; ping 20 s; pane start 80x24; watch `QUIET` 150 ms /
  `LATEST` 1.2 s; `IDLE_GRACE` 5 min (mission); drone `DEFAULT` linger 600 s, `STARTUP_GRACE` 30 s,
  `TICK` 250 ms, `IDLE` 3600 s; db `IDLE` 10 min, `EDITOR_TIMEOUT` 5 min; assist/feedback timeouts
  10-30 s. 52 further inline `from_secs/from_millis` in production host code.
* **Harness names**: essentially none hardcoded in host (rule respected): 2 doc/comment hits for
  "claude"/"gemini"; no `/Users/...` outside a doc comment (`environment.rs:22`).

## A7. Error handling

* **No crate-wide error type.** `thiserror` derives: 3 in host (`StoreError`, `SecretsError`, ...),
  proto has hand-written enums for wire/files/git/repos/search/connect/carrier/drone errors
  (`WireError`, `FileError`, `GitError`, `CloneError`, `SearchError`, `ConnectError`, `HandshakeError`,
  `DroneError`, `HostPathError`) with `Display` written by hand.
* **Strings dominate**: `Result<_, String>` 186 occurrences in host (e.g. `suggest_job`, web_assets,
  help, mcp tools), `anyhow` 50 uses (config, pty spawn, mcp start, feedback), 67 `map_err(|e|
  format!/to_string)`.
* **Errors become messages** as per-family error variants (`PaneError{error:String}`, `ProjectError`,
  `FileError` typed, `GitError` typed, `SearchError`, `SuggestError`, `ConnectError` typed). Most
  families carry a `String`; typed enums exist for files/git/search/connect/clone only.
* **Swallowed errors**: `let _ =` 113 in host (19 in drone, 11 in app), `.ok();` 24. Mostly sends to
  a gone window (intentional) but also `set_permissions` (`socket.rs:307`), `remove_dir_all`, etc.
* **Panics**: production `unwrap` 21 host / 2 drone; production `expect` 31 host / 22 drone. The
  `expect`s are overwhelmingly `thread::Builder::spawn(..).expect("the X thread")` (spawn failure
  aborts the owning thread) and `Mutex::lock().expect(..)` (poison cascade: one panic while holding the
  heartbeat/linger/scrollback lock takes every other user down). 152 `tracing::warn/error` in host.
  `catch_unwind` x2 (host_meta). See A3 for panic effect.
* No `unsafe` in host except 8 (platform), 9 in app (Windows console), 2 in drone, 1 in proto.

## A8. Tests

| Where | Count | Notes |
|---|---|---|
| ubiq-host `tests/` | 24 files, 17,250 lines, ~480 tests | coordinator 53, git 73, files 60, work 59, mission 41, projects 29, search 24, settings 19, diff 17, conversation 16, trello 15, remote **6**, watch 5... Drive real types against temp dirs, not mocked bus |
| ubiq-host inline | ~590 | biggest: coordinator 46, agent 41, plan/service 39, mcp/server 22, shells 20, db/tests 18. Uncovered (no inline and none obviously in tests/): `tasksrc/service.rs`, `search/worker.rs`, `git/observe.rs`, `connectors/flow.rs`, `web_assets/manifest.rs`, `mcp/plan.rs`, `mcp/kb.rs`, `db/agent.rs`, `mission/scheduler.rs` |
| ubiq-proto | 152 | `tests/` bus, catalog, files, ids, log_sink, plan, repos, secret, settings, work. **`tests/work.rs` now compiles** (checked with `cargo test -p ubiq-proto --no-run`; `wip/drone.md` still says it does not) |
| ubiq-app | 21 | all `handoff.rs` |
| ubiq-drone | 33 (14 unit, 19 integration) | ran green here: detach 4, handshake 3, managed 4, root_id 1, search 5, session 2 |

* **PTY/harness doubles**: no PTY fake. Coordinator tests spawn real `/bin/cat` and `/bin/sh -c`
  (`tests/coordinator.rs:210,356,1563`), so they are unix-only and need a real pty; `conversation.rs`
  has an in-file fake harness (`:1383,:1448`). `#[ignore]`: 1 in the four crates.
* **Gaps**: dispatch coverage ~53 integration tests for ~183 dispatch arms; no per-variant serde
  round trip; remote `serve_tls` has no test (grep: only `ubiq-app/lib.rs` and the module itself;
  `ubiq/tests/remote.rs` mentions tls/https but not the listener constructor); unknown-variant/skew
  behaviour untested; no test of panic-in-worker behaviour; sandbox-only known failures: 2 coordinator
  tests (watch events, home listing) per memory/wip notes.

## A9. The drone (`ubiq-drone`)

**Purpose**: "one machine's terminal, files and facts over one duplex byte stream" - a lean host
(no harness, git, index, listener) that a Ubiq on another machine drives over `ssh` stdio, or that
lingers behind a unix socket. Binary `ubiq-drone` with `--stdio|--listen|--attach|--foreground|--linger|
--root|--root-id|--list|--status|--stop|--probe|--version`, argv parsed by hand (`main.rs`, 297).

| File | Lines | Role |
|---|---|---|
| `relay.rs` | 1,208 | the run loop standing in for the coordinator (pane, file, browse, project-list, shell-list, search); catch-all refuses typed |
| `search.rs` | 630 | shells out to `rg`/`ag`/`grep`, `--` and `-F/-e` used correctly |
| `socket.rs` | 616 | unix socket listen/attach/stop, preamble, `hold()` via tmux/screen/setsid, state refresh |
| `state.rs` | 195 | `DroneState` JSON beside socket; `--list/--status` |
| `scrollback.rs` | 193 | ring per pane while detached; replay on attach |
| `linger.rs` | 174 | linger policy/countdown |
| `carrier.rs` | 124 | stdio wired to `ubiq_host::carrier::pump` |
| `lib.rs` | 81 | `capabilities()` (`files`, `search` if a tool probes), `triple`, `probe_line` |

Also `ubiq-proto/src/drone/` (manifest + verify, 336 lines) and the deployer / ssh profiles / askpass in
`crates/ubiq/src/app/ssh_connect.rs` (1,481 lines, UI crate).

**Maturity** (docs and code agree): phases 1-9 built (handshake+heartbeat, ssh profiles, attach over
ssh, detach+linger, managed drones, per-project origin `runs_on`, provenance, search). `cargo test -p
ubiq-drone` = 33 pass. Prod `unwrap` 2, no `TODO`s, clean layering (depends only on proto + lean host).
Phase 10 (drone as an MCP tool for agents: `drone_*`, `RunCommand/CommandFinished`, per-root
read-only mode) is **designed, none written** (G266-G268). No code in `mcp/` mentions drones.

**Gaps vs `features/drone.md` / `wip/drone.md`**
1. Nothing has ever run against a real `ssh`/`scp`/`sshd` (argv, askpass handshake, deployer's four
   steps, adoption over the wire are unit-tested over inputs only): G258, G261, G262.
2. `BINARIES` manifest is **empty** (`ubiq-proto/src/drone/manifest.rs`): the deployer refuses every
   triple; no cross-built drone has ever been produced (`just drone-build` never run).
3. Linger expiry is silent: panes vanish and a Session host reconnect starts a fresh drone (G263).
4. Seven messages are refused with no error variant (silently dropped + warn), so the UI can wait
   forever for them.
5. `DiffProjectFile` refused; `git` capability never advertised (G264).
6. No Windows support (G291: `state.rs` uses `UnixStream`, breaks `just check` on Windows).
7. Schema bump not mechanically verified (G256); heartbeat breaks pre-heartbeat peers (G257).
8. `hold()` (tmux / screen / setsid / bare spawn) and `shell_line` quoting are untested; `relay.rs`
   (1.2k lines, the whole message surface) has 0 inline tests and is covered only by the 19
   integration tests.
9. Security model is "ssh is the auth": a drone grants an attached peer an arbitrary-program pane
   plus read/write on roots (no read-only mode, G267); the unix-socket path relies on dir `0700`.
10. Detached-drone tmux session naming (`ubiq-drone-<stem>`) can collide across users/roots; stale
    socket removal deletes any non-socket file at the path (`socket.rs:~316`).

---

# Section B - Pain points, ranked by severity

Severity: **H** = can cause data loss/compromise/hang in normal use; **M** = real risk or heavy
maintenance tax; **L** = hygiene.

| # | Sev | Pain point | Evidence | Remedy (one line) |
|---|---|---|---|---|
| 1 | H | `--serve` hands the *whole* message set (arbitrary-program `SpawnWorkspace`, host browse/write, secret submission, git write) to anyone with one long-lived URL token; bare `--serve` binds all interfaces, plaintext by default, thread-per-connection with no cap or rate limit | `ubiq-app/src/lib.rs:355-430, 732`; `remote.rs:45,240`; `announce()` itself warns | Default to loopback and TLS-required, add per-connection cap + auth-failure backoff, token rotation/expiry and a capability scope (read-only vs full) |
| 2 | H | A panic in the coordinator or any worker thread is invisible: `JoinHandle` dropped, no panic hook, no supervisor; later requests are silently dropped while the window stays up (UI waits forever) | `coordinator.rs:144-163`; `files/mod.rs:851-870`, `git/mod.rs:84`; `catch_unwind` only in `host_meta.rs` | Install a panic hook that sends a host-fatal message and, per job, `catch_unwind` in the 4 workers; make `submit` answer an error variant when the thread is gone |
| 3 | H | Secrets cross the bus as `Secret` with transparent serde and the always-on tape serialises every message to JSON, so tokens/DB passwords/ssh passphrases land in the ring and in `$TMPDIR` dumps | `messages.rs:3944`; `bus.rs:624-647, 725-790` | Make `Secret` serialise as redacted in tape/log path (custom `Serialize` flag or scrub fields named in an allowlist), write dumps `0600` with `create_new` |
| 4 | H | No cross-process write coordination: all stores rewrite whole files from memory with no lock; `handoff` only blocks the same executable path, so `ubiq` + `ubiq-studio` (+ a `--serve` host) on one root lose updates for **every** store, not only `projects.toml` (G29) | no `flock` anywhere; `handoff.rs:name()` hashes `current_exe`; `store/file.rs` | Take an advisory lock on `<root>/.lock` at boot (hand-off keyed by root, not exe) and refuse/attach the second process |
| 5 | H | Decode failure of one frame = session death; no version/capability handshake for the TCP attach, `MESSAGE_SCHEMA` covers the drone only and is never mechanically checked | `carrier.rs:140`; `wire.rs:48`; `remote.rs` has no schema field | Send hello with schema+build on `/attach`; on `Decode` of an unknown variant log and skip the frame; add a CI snapshot test of the variant list |
| 6 | M | Production serves **mock** sessions and agents for every project (11 fake agents, 5 fake sessions, fake `SendToAgent` thread) beside live ones | `work/mod.rs:230-241,1070,1203`; `work/mock.rs`; G48/G52/G94 | Gate `mock` behind a dev flag / remove once Teams-old canvas is retired, return only live agents |
| 7 | M | `Coordinator::dispatch` is a 2,096-line match and the struct has 57 fields; logic for many families sits in coordinator methods (`launch`, `spawn_workspace`, `start_conversation`...), 8.6k-line file | `coordinator.rs:165-395, 1503-3598` | Split dispatch into per-family handlers (`fn handle_git(&mut self, ..)` in `coordinator/<family>.rs`) and group fields into sub-structs per family |
| 8 | M | Adding a message touches 5-6 places incl. two silent-failure traps (`pane_id_of` catch-all; drone relay drop for 7 messages); direction not typed; one 350-variant enum | `transport-contract.md:3252-3285`; `hosts.rs:207`; `drone.md` "What a drone refuses" | Split `Message` into `ToHost`/`ToUi` per family enums (or at least a generated direction table + exhaustive `pane_id_of`) and add error variants for the 7 drone-silent ones |
| 9 | M | Blocking work still on the coordinator thread: three file-backed stores (fsync per write) and the spawn probe, so a slow disk stalls every pane's keystrokes | SKILL ubiq-host gotchas, G38/G46; `projects.flush_due` in `run` | Move store persistence to a writer thread fed by a coalescing channel |
| 10 | M | Per-client bus channel is unbounded; a slow-but-alive remote client (heartbeat answers) grows host memory without bound with `TerminalOutput` | `bus.rs:163`; `carrier.rs:~105`; ping policy 20 s x3 | For remote clients, coalesce/drop `TerminalOutput` past a high-water mark (the UI can ask for a redraw) |
| 11 | M | MCP listener is unauthenticated beyond loopback + id in URL, no Origin/Host check, unbounded body read, single serving thread (one slow client stalls all MCP); exposes write tools (`ubiq-sql-write`, tasks, plans, KB) | `mcp/server.rs:10-17,113,222`; `catalogue.rs` SERVERS | Per-run bearer token in the injected header, `Host`/`Origin` check, body cap (e.g. 1 MiB) and a read timeout; thread pool for the accept loop |
| 12 | M | Worker/threading pattern is copy-pasted and non-uniform: 4 near-identical `start()/submit()` workers, 42 ad hoc `thread::Builder` sites, different stop mechanisms, `Mutex::expect` poison cascades | `files/mod.rs:851`, `git/mod.rs:84`, `search/mod.rs:37`, `index/mod.rs:67`; 126 `Mutex` | One `Worker<Job>` helper in `ubiq-host` (named thread, panic isolation, typed stop); use `parking_lot::Mutex` (already in proto) |
| 13 | M | `search/fallback.rs` `ag` branch passes the query without `--` (option injection; `ag --pager` executes) - becomes remotely/agent reachable once phase 10 lands | `search/fallback.rs:82` vs drone `search.rs:461` | Add `--` (or `-e`) before the query, mirror the drone's argv builder (share one function) |
| 14 | M | Errors are untyped strings in most families (`Result<_, String>` x186, `PaneError{error:String}`), so the UI cannot branch, and 113 `let _ =` hide failures | grep counts in A7 | Introduce a `HostError` enum per family with `kind` + `detail`, keep strings only for display |
| 15 | M | Drone never exercised in practice: empty manifest, no real ssh run, silent linger expiry, `relay.rs` (1.2k lines) untested inline, tmux/screen `hold()` untested | A9 gaps 1-8; `manifest.rs` `BINARIES = &[]` | Add a CI job that builds one triple and runs `ubiq-drone --stdio` through a local `sshd` container; add a linger-expiry notice message |
| 16 | M | Compiled-in third-party secrets (`UBIQ_FEEDBACK_GITHUB_TOKEN`, `..._API_KEY`) ship in release binaries | `feedback/mod.rs:35-40`, `feedback/issues.rs:21-24` | Route feedback through the project's own endpoint (API destination) and drop the embedded GitHub token |
| 17 | L | No schema migration machinery: every on-disk envelope is `version: 1` with only "refuse if newer"; first real bump will have to invent it | `store/file.rs:25,284,532,616`, `store/mission.rs:40` | Add a `migrate(from, to)` hook per envelope with a fixture test now, while only v1 exists |
| 18 | L | Visibility is all-`pub` (1,113 vs 31 `pub(crate)`; 47 pub modules, 0 private): every internal is public API, so refactors cannot tell what is safe to change | `ubiq-host/src/lib.rs`; counts in A1 | Default new items to `pub(crate)`, make the 20 leaf modules private and re-export the few that app/drone/tests use |
| 19 | L | Atomic write creates temp with default umask (not 0600) for config files that hold credential *references*; stale-socket removal in drone deletes any non-socket file at the path; `set_permissions` result ignored | `atomic.rs:66`; `socket.rs:307, ~316` | Create temp files `0600` where the store is sensitive; `symlink_metadata` check before `remove_file` |
| 20 | L | `listener` feature mixes inbound TCP server and outbound HTTP; docs drift (wip/drone says `ubiq-proto/tests/work.rs` does not compile, it does); `transport-contract.md` (3,312 lines) re-states the enum by hand | `Cargo.toml:156`; `wip/drone.md` "pre-existing failures"; audit `inbox-transport-audit` | Split features `serve` vs `net-client`; generate the contract table from `messages.rs` (a `_tools` script) and lint on drift |
| 21 | L | Magic numbers are well named but scattered (169 consts, 52 inline), not configurable or grouped; coordinator wakes every 2 s forever (`TASK_SYNC_EVERY`) even when idle | A6; `coordinator.rs:79, 1215-1219` | Collect timings in one `timing.rs` and wake on deadline only when a sync is actually due |
| 22 | L | Tests need a real PTY and unix (`/bin/cat`, `/bin/sh`); no PTY test double; only 6 tests for the remote server and none for TLS | `tests/coordinator.rs:210,356`; `tests/remote.rs` | Add a `FakePty` behind the `pty` module boundary and a TLS round-trip test with a self-signed cert |

## Notes on method
Read-only: counts from `grep`/`wc`; production-only counts via first-`#[cfg(test)]` cut-off (a heuristic:
files with `#[cfg(all(test,..))]` markers may be slightly over-counted). Run: `cargo test -p ubiq-proto
--no-run` (ok) and `cargo test -p ubiq-drone` (33 pass). Not run: full `just verify`, ubiq-host
tests (long; two known sandbox failures). UI-crate code (`crates/ubiq`) was only read where it is the
other half of a host contract (`hosts.rs`, `wire.rs`, `ssh_connect.rs`).
