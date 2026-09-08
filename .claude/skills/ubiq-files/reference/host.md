# The host half: files, search, index, watch — and the wire

Four workers, all the same shape: one thread, one unbounded `flume` queue, a `Job` carrying
everything it needs (including the project's root, taken from memory on the coordinator's thread),
and a `Mailbox` to answer on. The handle lives on the coordinator; dropping it ends the thread.
None of them run on the coordinator's thread, which is the dual of "the coordinator's reader is
never blocked by a slow window".

| Worker | Thread name | Handle | Queue holds |
|---|---|---|---|
| `files/` | `ubiq-files` | `Files` | `Job { kind: JobKind::{File, Browse}, reply_to }` |
| `search/` | `ubiq-search` | `Search` | `Job { project_id, search_id, root, query, filter, excludes, fallbacks, index, cancel, reply_to }` |
| `index/` | `ubiq-index` | `Index` | `Job::{Build, Changed, Drop}` |
| `watch/` | `ubiq-watch-<project id>` | `Watcher` (one per `(ClientId, ProjectId)`) | — (`notify` events, debounced) |

---

## `crates/ubiq-host/src/files/`

### `mod.rs` — the four operations and the worker

| Function | Does |
|---|---|
| `listing(root, rel_path, depth) -> Vec<DirListing>` | Breadth-first so a shallow row never waits behind a deep one; `depth` clamped to `1..=MAX_DEPTH`; the first listing is always the directory asked for; stops at the reply ceiling |
| `contents(root, rel_path, max_bytes) -> FileContents` | Stats before opening and **requires a regular file** — a read on a directory succeeds on macOS, and a FIFO or device would block the thread forever, which is why `EntryKind::Other` exists. Reads one byte past the limit so `truncated` is derived from what was read, not from the stat. `version` is withheld when truncated |
| `save(root, rel_path, bytes, expected) -> FileVersion` | Containment settled first. `expected` + file → version must match else `Conflict`; `expected` + no file → `Missing` (a save is not a resurrection); no `expected` + file → `Conflict`; no `expected` + no file → create. Writes via `crate::atomic::write_atomic_with`, keeping the file's permissions |
| `edit(root, rel_path, to, op)` | `Create{dir}` (exactly one level, never a parent), `Move`, `Copy`, `Trash`, `Delete`. `wants_to != to.is_some()` is refused outright |
| `file_error(project_id, rel_path, error) -> Message` | The one constructor. A `Refused` is logged as a wiring mistake — the interface only ever holds paths the host handed it |

| Constant | Value | Bounds |
|---|---|---|
| `MAX_ENTRIES` | `2_000` | One directory's listing |
| `MAX_REPLY_ENTRIES` | `10_000` | Every entry one reply carries, across all its listings |
| `MAX_DEPTH` | `3` | How deep one request may walk |
| `MAX_READ_BYTES` | `2 * 1024 * 1024` | A read, unless the caller asks for less |
| `SNIFF_BYTES` | `8 * 1024` | How far in a NUL is looked for |
| `MAX_COPY_ENTRIES` | `20_000` | A recursive copy — the one op whose cost is not bounded by the path it names |

`from_io` is the one mapping: `NotFound` → `Missing`, `PermissionDenied` → `Denied`, everything
else → `Failed`.

### `path.rs` — the security boundary

`MAX_COMPONENTS` = 64. `components()` uses `Path::components` rather than a split on `/`, so
`/etc/passwd` reads as a `RootDir` and `C:\…` as a `Prefix` under each platform's own rules.
**`..` is refused, never popped** — popping turns a crafted string into a probe of the root's
parent.

- `resolve` — must exist; canonicalise; contain.
- `resolve_for_write` — the *parent* must exist and be contained; the leaf must not be a symlink
  (following one puts the containment check on the wrong path, replacing one silently is worse);
  no folder is created to make the path valid.
- `resolve_inside` — `resolve`, refusing an empty `rel_path`, so the project's own folder can never
  be an argument to a delete or a move.
- `child(parent_rel, name)` — the only construction of a `rel_path`.

### `browse.rs` — the host-browse family's worker

No root, nothing to contain: this is what happens *before* a project exists, and it is the one
place in the neighbourhood allowed to see an absolute path. `directories::BaseDirs` gives the
default starting place (the user's home), the same source `cli_shortcut.rs` and `config.rs` use.
`MAX_ENTRIES` = 2 000, independent of the file family's own budget. `Listing` carries `path`
(always canonicalised), `parent` (`None` only at the filesystem root), `entries`, `truncated`.

### `diff.rs` — one file against a version-control base

**A diff is not a file**: its content is a comparison, and computing one means reading version
control, which is the host's. The host answers hunks with line numbers already worked out, so no
diff library reaches the interface — the same discipline that keeps a VT parser out of the host.
Both sides are compared as **raw bytes as stored**: git's clean and smudge filters are not run,
because running them means running configured programs from a folder the user merely opened.

`MAX_SIDE_BYTES` = `MAX_READ_BYTES`, `MAX_HUNKS` = 400, `MAX_ROWS` = 10 000, `CONTEXT` = 3.
A project with no repository is `FileError::Refused`, not an empty diff.

---

## `crates/ubiq-host/src/search/`

`mod.rs` starts one worker with one unbounded queue; the walk inside it is parallel and uses the
project's own ignore rules. Interruption is an `Arc<AtomicBool>` checked between files by the
walker and between matched lines by the sink.

### `worker.rs::run`, in order

1. Log the search — **lengths and counts, never payloads**: the query is user text and a hit
   carries file contents.
2. Resolve `filter.subdir` through `files::path::resolve` — the same boundary, already tested on
   its own. A non-directory or a refusal is `SearchError::BadFilter`.
3. Build the matcher: `RegexMatcherBuilder` with `case_insensitive(!case_sensitive)`,
   `fixed_strings(!regex)`, `word(whole_word)`. On failure, try the external fallback; if that does
   not run, `SearchError::BadQuery`.
4. **Then** try the index (`candidates`). If it answers, `run_candidates` reads those files and
   returns — same sink, same batching, same ceilings, same messages.
5. Otherwise `walk::builder` and a parallel walk. A glob that will not compile is
   `SearchError::BadFilter`.

`candidates` answers `None` — meaning *walk it* — for: no index, `!ready()`, a regex query, a query
with no trigram, or an index error (logged once). A subdirectory filter or an include glob does
**not** disqualify the index; the candidates are filtered afterwards by `allowed`, which tests
every ancestor directory as well as the file.

### `ceiling.rs`

| Constant | Value |
|---|---|
| `HITS_PER_FILE` | `100` |
| `FILES_WITH_HITS` | `1_000` |
| `TOTAL_HITS` | `10_000` |
| `PROGRESS_INTERVAL` | `100` files |
| `BATCH_FILES` | `64` |
| `BATCH_HITS` | `512` |
| `BATCH_INTERVAL` | `100ms` |

### `walk.rs`, `hits.rs`, `fallback.rs`

`walk::builder` is **the one walk** both content search and the index's cold build run, so the two
can never disagree about what a project's files are. The project's own rules (`.gitignore`,
`.ignore`, hidden files — `ignore`'s defaults, unchanged) apply first, then one
`ignore::overrides::Override` carrying the include globs and every exclude.

`hits::scan_file` is split out because a hit is produced the same way however the file was chosen —
the walk visits, the index looks up, both read the same bytes through the same `grep-searcher`
sink under the same ceilings, so a hit is byte-identical either way. `hits::State` is the
accumulator: `saw_file`, `add_file`, `should_flush`, `take_batch`, `at_ceiling`.

`fallback` is invoked from exactly one place — the `Err` arm of `RegexMatcherBuilder::build`.
`pick(order)` walks `HostSettings::search_fallbacks` (default `["ag", "grep"]`) through
`shells::locate`, skipping `find` and `fd` because they match file names and cannot answer a
content search. `FALLBACK_DEADLINE` = 10s, and the process is killed at it — a wedged external
tool would otherwise wedge the worker permanently.

---

## `crates/ubiq-host/src/index/`

Only the full-text half exists. It answers *which files could match*, never what matched (`D75`).
The thread owns every `text::Text`, so a writer is never shared and never locked; what leaves is a
`text::Reader`, published into a `readers: Arc<Mutex<HashMap<ProjectId, Reader>>>` map **beside**
the queue — a search needs the reader now, on the coordinator's own thread, and asking the index
thread for it would queue behind whatever build is in flight.

`Job::Build { project_id, root, home, excludes }` / `Job::Changed { project_id, root, paths,
truncated }` / `Job::Drop(project_id)`. The index lives in a host-owned `index/` directory under
`projects/<ulid>/` — the interface is never told it exists.

`text.rs`: each document is a file's **path and its trigrams**; the content is not stored.
`ngram(3,3)` with case folding, no stemmer, queried as the `AND` of the query's trigrams (`D76`) —
trigrams make the candidate set a **superset** of what a substring search finds, where a word
tokenizer would make it a subset and no re-read could recover a file the index never named.
Trigrams are computed over `chars`, not bytes, or a split multi-byte character silently answers
"no candidates". Candidates come from `DocSetCollector`, not `TopDocs`: the body is indexed without
positions or frequencies, so there is no score to rank by, and sorting by path makes truncation
deterministic. `indexable(path, len)` is the size + NUL gate.

`ceiling.rs`:

| Constant | Value | Means |
|---|---|---|
| `STAMP` | `1` | A stamp that is missing, unreadable or foreign deletes the directory and builds cold. Never repaired, never migrated |
| `MAX_FILE_BYTES` | `1 << 20` | Over this, never indexed — still found by the walk |
| `BINARY_SNIFF_BYTES` | `8 * 1024` | A NUL in here, never indexed |
| `MIN_QUERY_CHARS` | `3` | Below this there is no trigram |
| `CANDIDATES` | `2_000` | One query's answer; sets the same `truncated` flag the walk's ceilings set |
| `FILES` | `50_000` | Per project |
| `WRITER_HEAP` | `15_000_000` | tantivy's writer |
| `COMMIT_QUIET` | `1s` | Commit debounce — an editor save is a burst, not an event |
| `COMMIT_PENDING` | `512` | …or this many paths, whichever first |

Per changed path it is **delete-then-add** — one code path for create, modify and delete, and a
rename arrives as two paths that each do the right thing. A `truncated` burst clears the index and
rebuilds from the tree; searches walk while that runs.

---

## `crates/ubiq-host/src/watch/`

One recursive `notify` watcher per open project plus one debounce thread. Two scopes out of one
recursive watch, **filtered rather than watched selectively** — a selective watch would have to be
re-registered every time a directory appears:

- anything under `.git/` never reaches `changed`; if it is `HEAD`, `MERGE_HEAD`, `index` or under
  `refs/`, it sets `repository` on the next flush instead;
- everything else is dropped if the project's ignore rules exclude it.

`QUIET` = 150ms (coalesced by path), `BOUND` = 64 paths — at which the batch flushes with
`truncated` and the reader re-lists the subtree instead of patching names.

`Job.index: Option<flume::Sender<index::Job>>` is a **second destination, not a relay**: the watch
thread pushes `ProjectFilesChanged` straight to its client, so the coordinator never sees a change
and has nothing to forward. One extra send per flush, downstream of the debounce.

---

## The coordinator's part

`crates/ubiq-host/src/coordinator.rs` holds `files`, `search`, `index`, `watchers`, and
`active_searches: HashMap<ProjectId, (SearchId, Arc<AtomicBool>)>`.

- `watch_project(client, project_id)` — a window shows one project at a time and there is no
  `CloseProject`, so opening the next one is the only signal the previous watch is unwanted:
  the map is `retain`ed on `(owner, watched)`. It builds the index (when the level keeps text) and
  starts the watch. A watch that will not start is logged and nothing else.
- `index_level(project_id)` — `record.index` (an `Option<IndexLevel>`, an **override**, not a
  value) else `HostSettings::index_level`. **The one place the question is answered**, so the
  settings dialog and the index thread can never come apart.
- `settle_index(project_id)` — called whenever either could have moved: the project's own
  `UpdateProject`, or a `SetSettings` moving the default under every open project. Building an
  index that exists and dropping one that never did are both no-ops, so it is safe to call
  whenever the answer *might* have changed.
- `search_job(...)` — reaps finished searches (the cancel flag doubles as "this search is over"),
  merges `HostSettings::search_excludes` with `record.search_excludes`, cancels and logs the
  superseded search, and puts `self.index.reader(project_id)` on the job. `None` there is never an
  error — the worker walks.
- `file_job` / browse — resolves the record's root from memory and submits a `files::Job`; a
  project not in the catalogue is a `FileError::Refused`.

---

## The wire

Owner: `_docs/tech/transport-contract.md`. Types: `crates/ubiq-proto/src/files.rs` and
`search.rs`.

### File family (`Message`, `crates/ubiq-proto/src/messages.rs`)

| Message | Direction | Payload |
|---|---|---|
| `ProjectTree` | UI → host | `project_id`, `rel_path`, `depth` |
| `ReadProjectFile` | UI → host | `project_id`, `rel_path`, `max_bytes?` |
| `WriteProjectFile` | UI → host | `project_id`, `rel_path`, `bytes`, `expected?` |
| `DiffProjectFile` | UI → host | `project_id`, `rel_path`, `base` |
| `EditProjectPath` | UI → host | `project_id`, `rel_path`, `to?`, `op` |
| `ProjectTreeListing` | host → UI | `project_id`, `rel_path`, `listings[]` |
| `ProjectFileContents` | host → UI | `project_id`, `rel_path`, `contents` |
| `ProjectFileWritten` | host → UI | `project_id`, `rel_path`, `version` |
| `ProjectFileDiffed` | host → UI | `project_id`, `rel_path`, `diff` |
| `ProjectPathEdited` | host → UI | `project_id`, `rel_path`, `to?`, `op` |
| `ProjectFileError` | host → UI | `project_id`, `rel_path`, `error` |
| `ProjectFilesChanged` | host → UI | `project_id`, `changed[]`, `truncated`, `repository` |

`ProjectFileError` is **per path**, for the reason `PaneError` is per pane: the interface can only
mark the row or tab the user is looking at if the message says which. The host does not re-probe a
project's health for a file failure — a `Missing` or `Denied` is the interface's cue to send
`RefreshProject`.

### Records

- `EntryKind::{Dir, File, Other}` — a symlink is classified by what it points at, and only when
  that is inside the root. `Other` is drawn (faint, unclickable) because a row the interface never
  sees is a tree that lies.
- `DirEntry { name, rel_path, kind, size, symlink }` — `size` is what lets the interface warn
  before opening something large; it cannot learn it any other way.
- `DirListing { rel_path, entries, truncated }` — always exactly one level, whatever depth was
  asked for, so a depth change never changes a type.
- `WALK_SKIP` = `.git .hg .svn .jj node_modules target dist build .venv __pycache__ .direnv .cache`
- `LIST_HIDE` = `.DS_Store`
- `FileVersion { len, modified? }`, `FileContents { bytes, len, truncated, is_binary, version? }`
- `PathOp::{Create{dir}, Move, Copy, Trash, Delete}`
- `DiffBase::{Head, Index}`, `DiffRowKind::{Context, Added, Removed}`, `DiffRow`, `DiffHunk`,
  `FileDiff { base, hunks, binary, truncated }` — line numbers one-based, absent on the side the
  row is not on, worked out by the host because a gutter that counts rows itself gets it wrong the
  first time a hunk is truncated.
- `FileError::{Refused, Missing, WrongKind, Denied, Conflict, Failed}` — an enum rather than a
  sentence because each is a different thing for the interface to do.
- `HostDirEntry { name, kind, hidden, readable }`, `HostPathError::{Missing, NotADirectory, Denied,
  Failed}`.

### Host browse family (`D82`)

`BrowseHostDir { path? }` → `HostDirListing { path, parent?, entries[], truncated }` or
`HostDirError { path?, error }`. `path` absent asks for a sensible starting place, not a listing of
one the interface named — a remote host's home directory is a fact only that host can state.

### Search family

`SearchProject { search_id, project_id, query, scope, filter }`, `CancelSearch { search_id,
project_id }` → `SearchMatches { batch }`, `SearchProgress { files_seen }`, `SearchFinished {
searched, truncated }`, `SearchError { error }`.

`Query { text, case_sensitive, whole_word, regex }` — four options, shared with find-in-file, and
"there is no fifth". `Filter { patterns, subdir? }`. `Scope::{Files, Project}` (the same thing in
v1). `Source::{File, Task, Chat, Kb}`, `Batch::{Files, Tasks}`, `FileHit { rel_path, lines,
truncated }`, `LineHit { line, text, ranges }` (1-based line, byte offsets into `text`,
half-open), `SearchError::{Root, BadQuery, Walk, BadFilter}`.

### Settings that reach these workers

`HostSettings` (`crates/ubiq-proto/src/settings.rs`):

- `search_excludes` — default `node_modules .git target dist build .venv __pycache__ .cache
  .direnv .DS_Store .gitkeep`, kept consistent with `WALK_SKIP` / `LIST_HIDE` but written as globs
  for `ignore`'s `Override`, so a leaf like `.gitkeep` matches wherever it sits.
- `search_fallbacks` — default `["ag", "grep"]`.
- `index_level` — `IndexLevel::{None, Light, Full}`, `Light` when unset; `keeps_text()` is
  `Light | Full`, `keeps_symbols()` is `Full` and nothing acts on it yet (`G156`).

`ProjectRecord::index: Option<IndexLevel>` is an override; `UpdateProject` carries
`IndexChange::{Inherit, Set(level)}` because serde reads an absent field and an explicit `null`
into the same `Option::None`, and "clear the override" must be distinguishable from "say nothing".
