---
name: ubiq-files
description: Reference for files in Ubiq — the host's file, search, index and watch workers, the file and search wire families, and the interface's explorer tree, file picker, search panel and editor file loading. Use when listing or reading a project's files, adding a path operation, changing content search or the index, touching the filesystem watch, or working on the explorer, the picker, the file dialogs or the search panel.
---

# Files in Ubiq

Every path fact the interface holds arrived over the bus. **The UI reads no disk** — no absolute
path, no descriptor, no `read_dir` — with exactly two named exceptions (a guest tab dropped in
from outside every project, and the web-export server, `D55`). A pane is an id plus a byte stream;
a file is a project id plus a `rel_path`.

```
ubiq-host                          ubiq-proto                    ubiq (UI)
files/    one level, one file's    files.rs   DirListing,        state/explorer/  the tree,
          bytes, one write, one               FileContents,                       merged and
          diff, one path edit                 FileDiff, PathOp,                   flattened
search/   one walk, streamed                  FileError          state/file_picker.rs  one
index/    which files could match   search.rs  Query, Filter,                     dialog, six asks
watch/    what moved on disk                   FileHit, LineHit  state/search.rs  the panel
                                                                 state/editor.rs  the buffers
```

## Read first

| You are | Read |
|---|---|
| Adding or changing a file / search message | `_docs/tech/transport-contract.md` — the file family, the host browse family, the search family |
| Changing the explorer, the picker, the file dialogs or the editor's tabs | `_docs/features/workbench.md` |
| Changing what is indexed, or how a content search is answered | `_docs/wip/indexing.md` (`status: current`, and it says what is *not* built) |
| Adding a worker, or moving work off the coordinator | `_docs/tech/architecture.md` |
| Restyling any of these panels | `_docs/tech/ui-and-design.md`, plus the `ubiq-ui` skill |

Your change updates the documents it touched, in the same commit. `just docs-touched` names them.

## The hard rules

- **The interface holds project-relative paths only.** Forward-slashed, no leading slash, no `..`,
  empty for the root. The host resolves against the record's root. This is the file-level form of
  "the UI never assumes the pseudo-terminal is local", and it is the seam a remote drone slots into
  — a project id plus a relative path do not say which machine answered.
- **`crates/ubiq-host/src/files/path.rs` is the security boundary, and it does two checks.**
  Components are refused *textually* (`..` is refused, never popped; `MAX_COMPONENTS` = 64), then
  every symlink is resolved and the result must still be inside the root. Neither check is
  sufficient alone: a textual check is defeated by a symlink, a canonicalising one by a root that
  does not exist yet. Three entry points — `resolve` (must exist), `resolve_for_write` (parent must
  exist and be contained, leaf must not be a symlink, no parent is ever created), `resolve_inside`
  (refuses the project's own root, so `remove_dir_all` can never take the project).
  `path::child` is the **only** way a `rel_path` is ever constructed, so nothing on disk leaks into
  a message.
- **Nothing in the file family runs on the coordinator's thread.** A cold `read_dir`, a
  `canonicalize` on a network mount, a two-megabyte read would stall every pane's keystrokes behind
  it. The coordinator takes a record's root from memory, hands a `Job` to a worker, and answers
  nothing itself. Same for search, index, watch and git.
- **`files::Files` is one thread, not a pool.** FIFO means replies to one window arrive in the
  order they were asked for, which is what makes "replace the rows under this path" safe without a
  sequence number on the wire.
- **Everything is bounded, and a ceiling that bites is *drawn*, never silent.** `truncated` on a
  listing, on a read, on a diff, on a file's hits, on a whole search, on a watch batch, on a
  candidate set. A tree with rows missing is a tree that lies.
- **Contents cross as bytes.** The host decodes nothing, on the same discipline that keeps a VT
  parser out of the host. `is_binary` is "a NUL in the first 8 KB", not a verdict on encoding.
- **A save names the version it read**, and a truncated read carries no version — which is what
  makes a truncated buffer unsavable *mechanically* rather than by the interface remembering.
- **The index selects candidate files; it never produces hits** (`D75`). Anything that has the
  index answer a query rather than narrow it is a second search implementation.
- **Every refusal falls through to the walk.** No index path ever errors the search, and the
  interface is never told which path ran — `SearchProject` gained no field for it.

## The flow, end to end

| Gesture | UI → host | host → UI | Lands in |
|---|---|---|---|
| Expand a folder | `ProjectTree { rel_path, depth: 1 }` | `ProjectTreeListing { listings[] }` | `ExplorerState::merge` |
| Open a project | `ProjectTree { "", depth: 1 }`, then the cache at `depth: 3` | same | `merge` + `fill_explorer_cache` |
| Click a file | `ReadProjectFile { max_bytes: 2 MiB }` | `ProjectFileContents` | `pending_files` → `OpenFile::attach` |
| `⌘S` | `WriteProjectFile { bytes, expected }` | `ProjectFileWritten { version }` | `OpenFile::saved`, then `RefreshProjectGit` |
| New / rename / copy / trash / delete / drag | `EditProjectPath { rel_path, to?, op }` | `ProjectPathEdited` (echoes the op) | `path_edited` — retargets or closes tabs |
| Open a diff tab, or pick a Git-screen path | `DiffProjectFile { base }` | `ProjectFileDiffed { diff }` | the diff tab *or* `git_view.diff` |
| Nobody asked | — | `ProjectFilesChanged { changed[], truncated, repository }` | re-list + re-read + git refresh |
| Search | `SearchProject { search_id, query, scope, filter }` | `SearchMatches` ×n, `SearchProgress`, `SearchFinished` / `SearchError` | `SearchState` |
| Browse a host with no project | `BrowseHostDir { path? }` | `HostDirListing` / `HostDirError` | `HostBrowseState` + the picker |

Every reply answers only the window that asked, except `ProjectFilesChanged`, which nobody asked
for and which reaches the one window whose own watch produced it.

## The mechanics you cannot infer

**The four numbers, all in `crates/ubiq/src/app/mod.rs`:**

| Constant | Value | Why |
|---|---|---|
| `MAX_FILE_BYTES` | `2 * 1024 * 1024` | What a read asks for. The host has the same ceiling and this never widens it; it keeps a buffer the user cannot read to the end of off the bus |
| `EXPAND_DEPTH` | `1` | One level is what an expand asks for — which is why `node_modules` costs one row |
| `CACHE_DEPTH` | `3` | How far the background cache walks into folders nobody opened. The host clamps to `MAX_DEPTH` = 3; the next unlisted folders are asked for as each reply lands |
| `FILTER_DEBOUNCE` | `100ms` | Coalesces a burst of keystrokes into one background walk |

Plus `MIN_QUERY` = 3 in `app/explorer.rs` (re-exported from `app`): the explorer's filter field
does nothing below three characters, because one or two letters match nearly everything and cost a
full walk to answer with a screen the user has to narrow anyway. `index::ceiling::MIN_QUERY_CHARS`
is also 3, for an unrelated reason — below three there is no trigram to look up.

**`ExplorerState::merge` matches entries by name.** One already there keeps its children and its
expanded flag, one that has gone is dropped with its subtree, one that is new arrives shut and
unlisted. That is what makes a re-listing — a restore, a refresh, a watch — idempotent rather than
destructive, and it is why a watch never loses the user's open folders. It answers `false` for a
listing naming a folder the tree does not hold (a folder collapsed away while its listing was in
flight). The host's order is kept as it came: the host sorts directories first and names without
case *so that two windows agree*, and re-sorting in the UI would put that back in doubt.

**A refresh reconciles without losing state.** On `ProjectFilesChanged` the window re-lists only
the folders the tree *already holds listed* (a listing for one it does not know is thrown away by
`merge` anyway); `truncated` means the burst outgrew the batch and `changed` is empty, so the root
is re-listed instead of patching names. Open tabs are re-read only when clean, not loading, not
the active tab, and not in `just_saved` — the watcher echoing this window's own write is not a
change to react to. A re-read keeps the buffer's selection and scroll, captured before `reload`
drops it (`OpenFile::set_restore` / `take_restore`), because a reread is not a fact the user asked
to see from the top.

**A search is superseded by discarding its id, on both sides.** The UI mints the `SearchId` before
the first message hits the wire and holds it on `SearchState::active`; `app/wire.rs::receive_search`
drops every reply that does not name *both* the active `search_id` and `project_id`. The host keeps
`active_searches: HashMap<ProjectId, (SearchId, Arc<AtomicBool>)>` — one live search per project —
and a new `SearchProject` sets the old one's flag, which the parallel walker checks between files
and the sink between matched lines. The same flag doubles as "this search is over", which is how
finished entries are reaped.

**The picker's size vocabulary** (`state/file_picker.rs`): `size_label` prints B / KB / MB against
one `KB = 1024` constant, so the number printed and the threshold compared against can never
disagree. `size_reading` answers `Plain` / `Large` / `Huge` against `SIZE_LARGE` (300 KB) and
`SIZE_HUGE` (500 KB), **exclusive of both bounds** — exactly 300 KB is not yet large — and
`SizeReading::warning()` is the tooltip sentence. An *unknown* size reads `Plain`: an unknown size
is not a small one and is not guessed at. The thresholds are about a harness's context window, not
about disk.

## Gotchas that bite

- **`GitStatus` on a row is `Option`, and `None` is not "clean".** Until a working-tree map has
  arrived nothing has been read. An untracked or ignored *directory* paints every child, because
  git does not look inside and a child not in the map is not clean.
- **Filtering finds, it does not prune.** A folder with nothing matching under it drops out rather
  than drawing empty. Only what the host has already named can match — which is why the background
  cache exists, and why a matched-but-unlisted folder is asked for (`unlisted_hits`) as the walk
  lands.
- **`WALK_SKIP` bounds a walk; it never hides a row.** An explicit `ProjectTree` aimed at
  `node_modules` is answered in full. `LIST_HIDE` (`.DS_Store` today) is the opposite — omitted
  from *every* listing, including an explicit one. `search_excludes` in `HostSettings` is a third,
  separate list, and they are globs for `ignore`'s `Override`, not bare name tests.
- **A filter walk that lands late is discarded by job id** (`ExplorerState::begin_filter` /
  `apply_hits`), and by view — a walk for the tree must not land on the list.
- **The explorer menu's `Separator` is an `ExplorerAction`**, not a drawing detail: a pick comes
  back as an index, so a line drawn without a slot behind it shifts every action below it.
- **A menu is identified, not flagged.** `menu_epoch` is stamped as a menu opens so the
  outside-click that dismisses the old one cannot shut the new one raised in the same event.
- **`to` on `EditProjectPath` is refused where it does not belong**, never ignored — a field the
  host silently drops is a wiring mistake the interface cannot see. Every op refuses a destination
  that already exists; `Move` and `Copy` also refuse a destination inside their own source.
- **`Trash` and `Delete` are two ops because they are two promises**, and the confirmation says
  which. The Shift modifier is read *at the click*, not while the menu is open.
- **`free_name` is best-effort by construction** — it can only see folders the host has listed, so
  a collision it misses comes back as `FileError::Conflict` on the row. The point is that
  Duplicate, where a collision is certain, needs no round trip.
- **`HostPathError` is not `FileError`.** No `Refused` (no root to escape), no `Conflict` (nothing
  is written), and `NotADirectory` stands in for `WrongKind`. `HostDirEntry.hidden` is marked, not
  omitted — a dotfile is real content, unlike `LIST_HIDE`'s junk. `readable` is a hint for greying
  a row, never a promise.
- **`host_browse.rs`'s staleness guard keys on the canonical path, not a request id**, because the
  protocol echoes back a path. `HostBrowseState` is replaced wholesale on every reopen, so a late
  answer for a session that has been replaced finds nothing in `pending` and is dropped
  (`Arrival::Stale`).
- **A regex query never consults the index** — a term index cannot bound an arbitrary pattern — and
  the index consult sits *after* the matcher is built, so a pattern ripgrep's engine rejects still
  reaches the external fallback (`ag`, then `grep`; `find` and `fd` are skipped with a log line
  because they match names, not contents).
- **`worker::allowed` tests every ancestor directory as well as the file.** The walk gets pruning
  free from `ignore`; a candidate path has no traversal to prune, and the glob `vendor` does not
  match the leaf `vendor/skip.txt`. Missing this returns an excluded folder's files the moment a
  project is indexed, and only then.

## Where the design is ahead of the code

- **The symbol half of the index is not built.** `IndexLevel::Full` is selectable and behaves
  exactly like `Light`; there is no `ProjectSymbols` message and no tree-sitter in
  `crates/ubiq-host/`. Nothing on screen says so — `G156`. Do not write as if symbols exist.
  The buffer's own outline (`crates/ubiq/src/ui/outline.rs`) is a separate thing that does exist
  and needs no host, no index and no project.
- **A project's index is rebuilt whole on every open** — `G154`.
- **Two windows on one project run two watches**, so every change reaches the index twice.
  Delete-then-add is idempotent, so the cost is a wasted read — `G155`.
- `search::Scope::{Files, Project}` search the same thing in v1; `Batch::Tasks`, `TaskHit` and
  `Source::{Task, Chat, Kb}` are placeholders nothing produces.

## Recipes

**Add a file operation, end to end.** Prefer a new `PathOp` variant over a new message — `D57` is
that one message with an op on it, and five variants would be five coordinator arms handing work to
the same worker.

1. `crates/ubiq-proto/src/files.rs` — the `PathOp` variant, with a doc comment saying what the
   *interface* does once it has happened (that is why the reply echoes the op).
2. `crates/ubiq-host/src/files/mod.rs::edit` — the arm. Resolve through `path::resolve_for_write`
   or `path::resolve_inside` **before** touching disk; refuse a taken destination; assert the
   `to`/no-`to` shape (`wants_to != to.is_some()` is the existing check).
3. `crates/ubiq/src/state/explorer/mod.rs` — an `ExplorerAction`, its `label()`, and a slot in
   `menu::menu_entries` (mind the `Separator` positions).
4. `crates/ubiq/src/app/explorer.rs` — `pick_explorer_action` raises the confirmation if the
   gesture cannot be retyped, `confirm_file_dialog` sends the message.
5. `crates/ubiq/src/app/wire.rs::path_edited` — retarget or close the tabs the path took with it.
6. `crates/ubiq/tests/files.rs` — the round trip, over a fake bus.
7. `_docs/tech/transport-contract.md` (the file family table and prose) and
   `_docs/features/workbench.md` (the menu inventory), same commit.

**Add a search filter option.** `search::Filter` already carries `patterns` (gitignore-term globs
against the project-relative path) and `subdir` (validated through `files::path::resolve`, the same
boundary the file family uses). A new option goes on `Filter`, is applied in **two** places —
`search::walk::builder` for the walk and `worker::candidates` / `worker::allowed` for the index
path — or the two disagree the moment a project is indexed. `Query`'s four options are deliberately
closed ("there is no fifth") and are shared with find-in-file, so adding one there is a bigger
decision than adding one to `Filter`.

**Add a bound.** Put the constant in the ceiling module that owns it (`search/ceiling.rs`,
`index/ceiling.rs`, or the `const` block at the top of `files/mod.rs`), and make the reply carry
whether it bit. A silent stop is the thing every one of these modules is written to avoid.

## Reference files

- [`reference/host.md`](reference/host.md) — `files/`, `search/`, `index/`, `watch/`, every
  ceiling with its value, and the file / host-browse / search wire families.
- [`reference/ui.md`](reference/ui.md) — the explorer tree and its keyboard, the picker, the
  search panel, the editor's file loading and saving, and the tests.

## Verifying

`just verify` = `check clippy test host ui docs-lint`. `just ui` checks that `crates/ubiq` still
does not depend on `crates/ubiq-host`. The tests here are state-level and need no frame:
`crates/ubiq/tests/explorer.rs`, `files.rs`, `files_changed.rs`, `file_picker.rs`, `search.rs`.
