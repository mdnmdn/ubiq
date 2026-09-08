# The interface half: the tree, the picker, the search panel, the buffers

```
state/explorer/     the tree, merged and flattened. No frame, no bus, no path on disk
state/file_picker.rs one dialog, six ways of asking. Reads no disk either
state/search.rs     the panel's query, options and results so far
state/editor.rs     one buffer per open file; the one state module allowed a library type
app/explorer.rs     every explorer mutator, the file dialogs, the drag, the cache, the filter
app/picker.rs       raising, driving and answering the picker
app/host_browse.rs  filling the picker from a remote host's filesystem
app/editor.rs       tabs, save, save-as, the tab menu
app/wire.rs         receive_file / receive_search — where every reply lands
ui/explorer.rs      the panel;  ui/file_picker.rs the dialog;  ui/file_dialog.rs the questions
ui/search.rs        the search panel;  ui/kit/files.rs the row chrome both trees share
```

---

## `state/explorer/`

Split five ways over one `ExplorerState`: `mod.rs` (types and the struct), `tree.rs` (mutation),
`rows.rs` (flattening), `filter.rs` (the background walk), `keys.rs` (the keyboard), `menu.rs`
(the right-click menu).

### The struct's load-bearing fields

| Field | Why it exists |
|---|---|
| `root: Arc<Vec<FileNode>>` | An `Arc` so `filter_snap()` hands the background walk a snapshot without copying the tree on the frame |
| `root_name`, `root_expanded` | The project's own first row: the one handle that collapses everything, drawn even while a filter is typed |
| `root_listed` | Without it an empty tree and a tree whose first listing has not arrived look identical, and every remembered folder would be dropped as gone in the frame before the host answers |
| `cursor: Option<String>` | A **path**, not an index: rows come and go as folders open and the filter narrows |
| `menu_epoch` | Stamped onto each menu so a dismiss says which menu it was aimed at — a right-click on a second row raises a new menu *and* fires the old one's outside-click in the same event |
| `copied: Option<String>` | One path, never the system clipboard. Copy/Paste/Duplicate all work off it |
| `cache_asked: HashSet<String>` | Without it a failed listing is asked for again on the next reply, and one in flight is asked twice |
| `filter_hits`, `filter_job` | The last background result, and a counter so a slow walk cannot land on a query the user has left |
| `git_marks`, `git_inherit`, `git_known`, `git_generation` | The working-tree map. `git_known == false` means *unread*, which is not *clean* |

### `tree.rs`

| Method | Notes |
|---|---|
| `toggle(path) -> Toggle` | `Listing` (open, nothing known — ask the host), `Done`, `Missing` |
| `set_loading(path, bool)` | The row says a listing is on its way rather than looking like an empty folder |
| `merge(listing) -> bool` | Entries matched **by name**; existing keep children + expanded; gone are dropped with their subtree; new arrive shut and unlisted. Host order kept as it came. `false` = the tree does not hold that folder |
| `apply_git(generation, entries, rollups)` | A reply older than what is held is discarded. Untracked/ignored directories go into `git_inherit`; rollups take the higher `rank()` |
| `expanded()`, `reopen(&mut wanted)`, `collapse_all()` | Restore is a level at a time — a listing either resolves one remembered folder or drops it, which is what makes a deep restore terminate |
| `set_view`, `reanchor(filter)` | Re-place the cursor after the row set changed |
| `unlisted_for_cache()` | Folders the background cache may still ask about — never a `WALK_SKIP` name, never one already asked |
| `unlisted_hits(rows)` | Folders a *filter* matched that the host never listed. Skip set left alone: a search for `node_modules` is not a request to list it |
| `begin_cache(paths)` | Marks asked + loading, so an expand while the answer is in flight does not ask again |
| `target_dir(path)` | Where New file / New folder / Paste land: the row when it is a folder, its parent when it is a file, `""` for the project row |
| `free_name(parent, leaf)` | `notes.md` → `notes copy.md` → `notes copy 2.md`, keeping the extension. Best-effort: only listed folders are visible, so a missed collision returns as `FileError::Conflict` |

### `rows.rs` — flattening

`rows(filter)` builds `Vec<Row>`; `Tree` leads with the project row, `List` is flat.
**Filtering is finding, not pruning** — a folder with nothing matching under it drops out rather
than drawing empty, or the answer to a search is a screen of empty folders. Only what the host has
already named can match.

`drawn_rows(filter)` is what the panel calls: an empty filter walks only open folders on the frame;
a non-empty one *borrows* the last background result (`Cow::Borrowed`) and never clones or walks
the cache on the frame. `rows_from_snap(snap, filter)` is the same rules run off the frame.
`visible_rows` is what the keyboard walks — hits when they match, a fresh walk otherwise, so
`tests/explorer.rs` can press keys with no background job.

### `filter.rs` — the debounced background walk

`filter_snap()` → `begin_filter() -> job` → walk on `cx.background_spawn` →
`apply_hits(job, filter, view, rows) -> bool`, which refuses a landing whose job id or view has
moved on. `clear_filter()` is immediate — clearing the field must feel instant.

The `app/explorer.rs` side: `schedule_explorer_filter` clears immediately for a short query
(`short_query` = fewer than `MIN_QUERY` = 3 trimmed chars), otherwise bumps
`explorer_filter_gen`, waits `FILTER_DEBOUNCE` (100ms) on the background executor, and re-checks
the token before spawning. `explorer_filter_ready` asks the host for `unlisted_hits` at
`EXPAND_DEPTH` — a matched folder must answer with its contents — and scrolls to the cursor.

### `keys.rs` — the keyboard

`ExplorerKey::{Up, Down, Left, Right, Enter, ShiftEnter, Delete, Dismiss}` →
`ExplorerPressed::{Ignored, Moved, Open{path}, Listing{path}, Dismissed, ClearFilter,
Remove{path, is_dir}}`.

`Ignored` is how `left`/`right` go back to being the filter field's caret keys in the flat list.
`Remove` is a *request*: the tree removes nothing itself, because a removal is the host's and a
question has to be answered first. `Escape` peels the menu, then the filter, then propagates.
The keystrokes themselves are `ui/explorer.rs`'s — what a platform calls "confirm" is not something
the tree's rules should have an opinion about. `⌘P` / `ctrl-p` reveals the panel and focuses the
filter; `down`/`tab` step onto the tree, `tab`/`shift-tab` back off.

`Follow::{Off, Once, Locked}`, cycled by one header button, `next()` in that order. Never persisted
— a follow that survived a restart would move the tree before the user asked anything. Revealing
runs through the same `wanted` list a restore uses, so a folder the host never listed is asked for
and opened when the answer lands; nothing is collapsed.

### `menu.rs`

`open_menu(path, is_excluded, x, y)` stamps the epoch; `close_menu(epoch)` closes only the menu it
was drawn for. `menu_entries(...)` builds `Vec<ExplorerEntry>` grouped by purpose with
`ExplorerAction::Separator` occupying a real slot — a pick comes back as an **index**, so a line
drawn without a matching entry silently shifts every action below it. A group with nothing in it
takes its separator with it. Only `Paste` is ever disabled, and only with nothing copied.

`ExplorerAction`: `Open`, `OpenDiff`, `CopyPath`, `CopyFullPath`, `CopyLink`, `OpenInSystem`
(labelled Finder / Explorer / File Manager per platform), `OpenInWeb`, `Refresh`, `NewFile`,
`NewFolder`, `Copy`, `Paste`, `Duplicate`, `Rename`, `Delete`, `CollapseAll`,
`ExcludeFromSearch`, `AddToSearch`, `Separator`. The last two write the project's
`search_excludes` through `app/projects.rs::set_project_search_excludes`, which sends the whole
list.

---

## `app/explorer.rs` — the gestures

| Concern | Functions |
|---|---|
| Listing | `toggle_folder` (`EXPAND_DEPTH`), `fill_explorer_cache` (`CACHE_DEPTH`), `ask_listing` |
| Filtering | `schedule_explorer_filter`, `spawn_explorer_filter`, `explorer_filter_ready`, `sync_file_filter_field` |
| Keyboard / mouse | `press_explorer_key`, `click_explorer_row`, `focus_explorer_filter`, `focus_explorer_tree` |
| Follow | `cycle_explorer_follow`, `reveal_active_file`, `follow_active_file`, `scroll_explorer_to_cursor` |
| Menu | `open_explorer_menu`, `pick_explorer_action` |
| Dialogs | `confirm_dialog`, `confirm_file_dialog`, `close_file_dialog` |
| Drag | `drag_path_over`, `drop_path_on`, `toggle_move_unasked` (`MOVE_UNASKED` = 10 min, in memory, per window) |
| Opening | `select_file` (permanent), `select_file_temporary` (preview), `double_click_explorer_row`, `open_guest_file`, `deliver_paths`, `open_diff` |

`select_file` / `select_file_temporary` both: mark selected, push `pending_editor_focus`, open the
tab **and its panel** together (each open file is its own panel, so a tab with none is a file with
nowhere to be drawn), then `ReadProjectFile { max_bytes: Some(MAX_FILE_BYTES) }` when the file is
fresh, or `PanelEdit::Reveal` when it is already open. A temp tab replaces the previous temp tab.

`open_guest_file` is the one path that touches `std::fs` — a file dropped from outside every open
project. It is read-only, hosted by the active project so it has somewhere to live among the
panels, and the bus is never asked because no project can answer for a path outside its own. Its
tab key is the absolute path, which cannot collide with a project-relative one.

Drops that change nothing, onto the row itself, or of a folder into its own child are refused in
the panel with no round trip. A folder drop raises a confirmation carrying the ten-minute
"don't ask again"; a file drop does not, because it is one path and a rename already asks.

---

## `state/file_picker.rs` + `app/picker.rs` + `app/host_browse.rs`

**One picker, asked for in six ways.** `PickerRequest { owner, title, root, pattern, kind, count,
commit, modal }`, built through a builder (`new`, `root`, `pattern`, `kind`, `count`, `commit`,
`modal`). `PickKind::{Files, Folders, Either}` — a folder is drawn in every mode because it is how
the files are reached, and picked only where one was asked for. `PickerCount::{Single, Multiple}`,
`Commit::{OnClick, OnButton}` (meaningless for a multiple pick — no click can mean "and that is all
of them"), `PickerOwner::{Sink, Composer{agent, slot}, HostProject}`.

**Nothing here reads a disk.** The forest is handed in and grown by `set_forest` / `fill_node`,
which is what lets the kitchen sink raise one over the already-loaded explorer tree
(`forest_from_explorer`) and lets a host's listings fill the same dialog one folder at a time.
Paths are project-relative for the former and absolute for the latter, and **the picker never tells
the two apart**.

`PickerNode` carries `listed` (known children, not the same as *having* children) and
`truncated`. `dir_unfetched` is the placeholder for a host folder the user has not walked into;
`needs_load()` / `expanded_needing_load()` is what asks for it.

Sizes: `MIN_WIDTH` 420, `MIN_HEIGHT` 300, `DEFAULT_WIDTH` 660, `DEFAULT_HEIGHT` 560; `resize`,
`start_drag` / `drag_to` / `end_drag` clamp against the viewport, because the window is what the
dialog has to fit inside. `size_label`, `SIZE_LARGE` (300 KB), `SIZE_HUGE` (500 KB),
`size_reading` → `SizeReading::{Plain, Large, Huge}` with `warning()` — see SKILL.md.
`matches_glob` is a hand-written `*` / `?` matcher, deliberately not a crate: a prefilter is
`*.md`, `Cargo.*`, `?ain.rs`, and a dependency that also understood `**/` and `{a,b}` would be a
promise this dialog does not keep.

`show_hidden` is the picker's call, which is why `HostDirEntry.hidden` is carried rather than
dropped upstream.

`app/host_browse.rs` holds `HostBrowseState { host, label, root, parent, root_truncated,
awaiting_root, pending_roots, pending_folders, error }` beside the picker, because the picker holds
a forest and nothing about where it came from. `classify(host, path) -> Arrival::{Root, Folder,
Stale}` is the whole staleness guard, pure and total so its own `#[cfg(test)]` module can pin it
down with no window. `children_of` keeps the host's order — directories first, case-insensitive by
name is `HostDirListing`'s own contract — and never re-sorts. `walk_picker_up` asks for the parent
as a new root.

---

## `state/search.rs` + `ui/search.rs`

`SearchState { query: Entity<InputState>, case_sensitive, whole_word, regex, active:
Option<ActiveSearch>, results: Vec<FileResult>, files_seen, total_hits, truncated, finished,
error }`. `ActiveSearch { search_id, project_id }` is the whole supersede mechanism on this side.

`app/git.rs` (where the project-search mutators live) — `run_project_search` trims the field, does
nothing on an empty query (not a search and not an error), cancels the previous `ActiveSearch`,
`reset()`s, mints a fresh `SearchId::generate()`, and sends `SearchProject` with
`Scope::Files` and a default `Filter`. `submit_header_search` / `search_for` are the `⌘K` route:
switch to IDE, write the text into the panel's own field, `reveal_search`, run.

`app/wire.rs::receive_search` folds batches by `rel_path`, accumulates `total_hits`, and ORs the
`truncated` flags; every arm first checks the reply names the active search *and* project.

`ui/search.rs`: `SHOWN_HITS` = 500 rows drawn. `hit_dest` turns a hit into a `Destination` so
clicking one navigates through the same path everything else does. `glyph_toggle` draws the three
query options.

---

## `state/editor.rs` + `app/editor.rs` — file loading and saving

**Each open file owns its buffer**, which is what makes dirty a comparison against a fact rather
than a flag somebody has to set: `baseline` is exactly the bytes the host sent. This is why the one
state module that names the component library's `EditorState` is this one — the buffer *is* the
file's state. (Language → highlighter mapping stays in `ui/editor.rs`, because that is a drawing
decision.)

`FileBody::{Loading, Text{state, baseline, truncated, version}, Diff, Bytes, ImageEdit, Binary,
Failed}`. `SaveState::{Idle, Saving(String), Failed(String)}` — the in-flight text travels with the
save, so the acknowledgement clears dirty against **what was written**, not against whatever has
been typed since.

A tab exists from the click that asked for the file (`OpenFile::pending`): a click has an effect, a
second click cannot ask twice, and a failed read has somewhere to say so.

`tab_key(path, subject)` / `from_tab_key` — `Subject::{File, Diff(base)}`, the tag prefixing the
path, and `Subject::File`'s tag is empty (which is what lets a guest tab's absolute path be its own
key).

| Method | Notes |
|---|---|
| `attach(...)` | Builds the buffer from the host's bytes and re-applies `take_restore()` |
| `set_binary`, `set_failed` | The two non-buffer outcomes of a read |
| `savable()` | The one gate. A truncated read has no `version`, so it is readable and never savable — writing a prefix back would shorten the file |
| `version()` | What goes out as `expected` |
| `mark_saving(text)` → `saved(written, current)` | `current` is what the buffer holds **now**, not what was written: anything typed while the save was in flight is still unsaved |
| `reload()`, `set_restore`, `take_restore` | The external-change path — selection and scroll survive a re-read |
| `retarget(path)` | What a rename does to an open tab |

`save_file` / `save_active_file`: an untitled buffer asks where to go first (`ask_save_as`), a
capture writes its flatten instead of text, everything else sends `WriteProjectFile { bytes,
expected }` after `mark_saving`. On `ProjectFileWritten` the window also sends
`RefreshProjectGit { full: true }` and records the path in `just_saved`, so the watcher echoing
this window's own write is not treated as an external change.

`reach_wanted` (in `app/editor.rs`) is the restore/reveal driver: each arriving listing either
resolves one remembered folder or drops it, which is what makes restoring a deep path terminate.

---

## `ui/` — drawing

- `ui/kit/files.rs` — the chrome the explorer and the picker share: `ROW_FONT` = 12.5,
  `row_height(font_size)`, `row_indent(font_size)` (an explorer row is sized from its text, not a
  constant), `view_switch`, `filter_bar`, `file_row`, `twisty`, `kind_icon`. **A row is one line,
  always** — elided with the whole string as a tooltip; the indent is *drawn*, not padded, so a
  selected row's accent bar stays flush left. Colour, the leading mark and the trailing element are
  the caller's, which is what lets git state land on an explorer row without the picker learning
  version control.
- `ui/explorer.rs` — `git_colour`, `name_colour`, `icon_colour`, `key_bindings()`, `render`,
  `line`, `follow_button`, `bookmarks_section`. The panel opens and decorates; the picker ticks and
  confirms.
- `ui/file_picker.rs` — drawn through `deferred` + `anchored` like the kit's modal, so a picker
  raised from inside a dock panel covers the window rather than being clipped to the panel.
  It is *worked in* rather than answered, so it is bigger than a modal, remembers its size while
  up, and has a corner grip. Whether an outside click dismisses it is the caller's
  (`PickerRequest::modal`).
- `ui/file_dialog.rs` — New file, New folder, Rename, the two Delete questions and the folder-move
  confirmation, painted from the window's root rather than the explorer (one of them is the
  editor's save-as). All kit calls except the folder move, which is the only one with a control in
  it. Enter confirms exactly what the dimmed button allows; Escape is `AppState::cancel_dialog`'s,
  not the dialog's own binding.
- `ui/search.rs` — the panel.

---

## Tests (state-level, no frame)

| File | Asserts |
|---|---|
| `crates/ubiq/tests/explorer.rs` | How a listing lands, what survives a re-listing, the keyboard, the menu entries, restoring a remembered set one level at a time |
| `crates/ubiq/tests/files.rs` | The gesture round trips: a menu pick or drop raises a question, confirming sends one `EditProjectPath`, `ProjectPathEdited` settles tabs and tree. Needs a window because the dialogs and their field are the window's |
| `crates/ubiq/tests/files_changed.rs` | `ProjectFilesChanged`: one listing per affected folder the tree already holds, a git refresh when the plumbing moved, a re-read for every clean open tab, and a dirty tab left alone |
| `crates/ubiq/tests/file_picker.rs` | The request-plus-filter function — folders-only lists no files, a prefilter never hides a folder, the list says which folder each match came from, a single pick replaces. Fixture is the sink's own tree, so the test exercises the tree the application actually raises one over |
| `crates/ubiq/tests/search.rs` | The trigger side: one `SearchProject` with the options as set, an empty query is not a search, a second cancels the first, a reply naming an unheld search is dropped |
