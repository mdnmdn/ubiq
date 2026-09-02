# Plan: Explorer & Editor Tab Improvements

## Summary

Seven changes across the explorer panel, editor tab strip, and their context menus. All touch
`crates/ubiq/src/` only — no proto or host changes.

---

## 1. Explorer: remove git status dot

**`crates/ubiq/src/ui/explorer.rs`** — `line()` at lines 307-319

Delete the 14px container with the 6px filled circle entirely. The badge letter (M, U, !, S)
already communicates the same information with colour.

---

## 2. Explorer: right-align git badge for file name alignment

**`crates/ubiq/src/ui/explorer.rs`** — `line()` at lines 321-334

Insert `div().flex_1().min_w(px(0.))` spacer after `elided_with` (the name) and before the badge.
This makes the name consume available space and pins the badge to the row's right edge:

```
[twisty] [icon] [name ─── flex_1] [badge] [loading/trailing]
```

---

## 3. Explorer context menu: "Copy Full Path" + "Open in Finder/Explorer"

### State — `crates/ubiq/src/state/explorer.rs`

- Add `CopyFullPath` and `OpenInSystem` to `ExplorerAction` enum (line 217)
- `OpenInSystem` label: OS-dependent via `cfg!(target_os = ...)` — `"Open in Finder"` /
  `"Open in Explorer"` / `"Open in File Manager"`
- Insert both in `menu_entries()` for files and readable directories, after the existing `CopyPath`

### Action handling — `crates/ubiq/src/app.rs` — `pick_explorer_action()` at line 3577

- `CopyFullPath`: resolve absolute path from `project_snapshot(cx).record.path` joined with
  rel_path via `std::path::Path::join`, write to clipboard
- `OpenInSystem`: same path resolution, then `std::process::Command::new("open")` (macOS) /
  `"explorer"` (Windows) / `"xdg-open"` (Linux) on the file's parent directory

---

## 4. File tab context menu: add "Copy Full Path" + "Open in Finder/Explorer"

### Menu items — `crates/ubiq/src/ui/file_tab_menu.rs`

Update `ITEMS` array:

```rust
const ITEMS: &[&str] = &[
    "Close",
    "Close Others",
    "Close Left",
    "Close Right",
    "Close All",
    "Copy Full Path",       // NEW (index 5)
    "Open in Finder",       // NEW (index 6) — label generated dynamically
    "Save",                 // shifts to index 7
    "Word Wrap",            // shifts to index 8
];
```

In `overlay()`, generate the "Open in Finder/Explorer/File Manager" label dynamically using
`cfg!(target_os = ...)`.

### Action handling — `crates/ubiq/src/app.rs` — `pick_file_tab_menu()` at line 4048

Update the index match:

```rust
match index {
    0 => self.close_editor_tab_at_key(&key, cx),
    1 => self.close_editor_tabs_except(&key, cx),
    2 => self.close_editor_tabs_left(&key, cx),
    3 => self.close_editor_tabs_right(&key, cx),
    4 => self.close_all_editor_tabs(cx),
    5 => self.copy_full_path_for_tab(&key, cx),       // NEW
    6 => self.open_in_finder_for_tab(&key, cx),        // NEW
    7 => self.save_file(&key, cx),
    8 => self.toggle_editor_wrap(window, cx),
    _ => {}
}
```

Add `copy_full_path_for_tab()` and `open_in_finder_for_tab()` methods. Same logic as the
explorer versions, using `from_tab_key(key)` to extract the path.

---

## 5. Temp preview: italic + subtle bg, single temp file

### Trigger behaviour

| Input | Action |
|---|---|
| Left click | Temp open |
| Double click | Permanent open |
| Shift+click | Permanent open |
| Cmd+click (macOS) | Permanent open |
| Enter | Temp open |
| Shift+Enter | Permanent open |

### State changes — `crates/ubiq/src/state/editor.rs`

- Add `temporary: bool` field to `OpenFile` (default `false`)
- Add `temporary_key: Option<String>` to `EditorPaneState` (default `None`)
- Add `OpenFile::temporary(path)` constructor — same as `pending()` but with `temporary: true`
- Add `open_temporary(path)`: if already open permanently → return existing index; if another
  temp exists → close it and its panel; otherwise create temp `OpenFile`, push, return index

### Click trigger — `crates/ubiq/src/ui/explorer.rs` — `line()` at line 387

Change the click handler to detect modifiers, and add `on_double_click`:

```rust
line.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
    let permanent = event.modifiers.shift || event.modifiers.platform;
    this.click_explorer_row(path.clone(), permanent, window, cx);
}))
.on_double_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
    this.double_click_explorer_row(path_for_double.clone(), cx);
}))
```

### Keyboard trigger — `crates/ubiq/src/ui/explorer.rs`

- Add `ExplorerShiftEnter` to `gpui::actions!` and `key_bindings()` with `"shift-enter"`
- Wire `on_action` handler

**`crates/ubiq/src/state/explorer.rs`** — `ExplorerKey`

- Add `ShiftEnter` variant; in `press()`, behaves same as `Enter`

**`crates/ubiq/src/app.rs`** — `press_explorer_key()`

- `ExplorerPressed::Open` from `ShiftEnter` → `select_file` (permanent)
- `ExplorerPressed::Open` from `Enter` → `select_file_temporary` (temp)

### App methods — `crates/ubiq/src/app.rs`

**Update `click_explorer_row()` (line 3497) — add `permanent` parameter:**

```rust
pub fn click_explorer_row(
    &mut self,
    path: String,
    permanent: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
) {
    // ... existing click logic ...
    match pressed {
        ExplorerPressed::Open { path } => {
            if permanent {
                self.select_file(path, cx);
            } else {
                self.select_file_temporary(path, cx);
            }
        }
        // ... rest unchanged ...
    }
}
```

**Add `double_click_explorer_row()` method:**

```rust
pub fn double_click_explorer_row(&mut self, path: String, cx: &mut Context<Self>) {
    self.select_file(path, cx);
}
```

**Add `select_file_temporary()` method:**

```rust
pub fn select_file_temporary(&mut self, path: String, cx: &mut Context<Self>) {
    let Some(project) = self.project(cx) else { return };
    let Some(open) = self.projects.get_mut(&project) else { return };
    open.explorer.selected = Some(path.clone());

    let fresh = open.editor.index_of(&path).is_none();
    let index = open.editor.open_temporary(&path);
    open.editor.active = index;

    if fresh {
        self.pending_panels
            .push(PanelEdit::Open(PanelKind::File(tab_key(&path, Subject::File))));
        self.bus.send(Message::ReadProjectFile {
            project_id: project,
            rel_path: path,
            max_bytes: Some(MAX_FILE_BYTES),
        });
    }
    self.remember(project, cx);
    cx.notify();
}
```

### Promotion (temp → permanent)

**`crates/ubiq/src/app.rs`** — `activate_file()` at line 3874:

```rust
// After open.editor.active = at;
if open.editor.temporary_key.as_deref() == Some(key) {
    if let Some(file) = open.editor.active_file_mut() {
        file.temporary = false;
    }
    open.editor.temporary_key = None;
}
```

**`crates/ubiq/src/state/editor.rs`** — `refresh_dirty()` at line 397:

```rust
// At the start of the method:
if self.temporary {
    if let Some(base) = self.baseline() {
        if text != base {
            self.temporary = false;
        }
    }
}
```

### Visual rendering

**`crates/ubiq/src/ui/dock/skin.rs`** — tab rendering at lines 300-331:

When `info.temporary == true`: apply `.italic()` to the tab div and a subtle background tint
(e.g. `theme::surface().opacity(0.3)`).

---

## 6. Editor tabs: no left dot, git-coloured title, right dot for dirty/save

### Remove left dot, add right dot — `crates/ubiq/src/ui/dock/skin.rs` lines 327-331

Current:

```rust
if let Some(colour) = dot {
    tab = tab.child(div().size(px(7.)).flex_none().rounded_full().bg(colour));
}
tab = tab.child(title.clone());
```

Replace with (title first, dot after):

```rust
tab = tab.child(title.clone());
if let Some(colour) = dot {
    tab = tab.child(div().size(px(7.)).flex_none().rounded_full().bg(colour));
}
```

### Git colour for title text

**`crates/ubiq/src/ui/editor.rs`**

- Rename `state_colour` → `dirty_colour` (returns save/dirty indicator colour for the right dot)
- Add `git_colour(file: &OpenFile, explorer: &ExplorerState) -> Rgba` that looks up `file.path`
  in `explorer.git_marks` and returns the colour using the same mapping as
  `explorer::git_colour()`

**`crates/ubiq/src/ui/dock/mod.rs`** — `WorkbenchPanel::tab()` at line 151

Replace return type `(SharedString, Option<gpui::Rgba>)` with a struct:

```rust
pub struct TabInfo {
    pub label: SharedString,
    pub title_colour: gpui::Rgba,
    pub dot_colour: Option<gpui::Rgba>,
    pub temporary: bool,
}
```

For `PanelKind::File`:
- `title_colour`: `editor::git_colour(file, explorer)`
- `dot_colour`: `Some(editor::dirty_colour(file))` when dirty/saving/failed; `None` when
  clean+idle
- `temporary`: `file.temporary`

**`crates/ubiq/src/ui/dock/skin.rs`** — tab rendering

- Apply `info.title_colour` to the tab text via `.text_color(info.title_colour)` instead of
  the current active/inactive ternary for file tabs
- Draw right dot only when `info.dot_colour.is_some()`
- When `info.temporary`: apply `.italic()` and subtle bg

### Remove dirty `·` from label — `crates/ubiq/src/ui/editor.rs` line 62

Remove the `·` bullet from the label text (line 68). The right dot handles dirty indication
visually.

---

## 7. Scrollable editor tab bar

**`crates/ubiq/src/ui/dock/skin.rs`** — `render_tab_bar()` at lines 409-418

- Add `tab_scroll: gpui::ScrollHandle` field to `Skin`, initialized in `Default`
- Replace `overflow_hidden()` (line 418) with `overflow_x_scroll().track_scroll(&self.tab_scroll)`
- Add left/right chevron scroll buttons at the tab bar edges, visible only when tabs overflow.
  Use `IconName::ChevronLeft` / `IconName::ChevronRight` at `Size::XSmall`
- Clicking an arrow scrolls by ~120px (roughly 3-4 tabs)

---

## 8. Click-to-focus on already-open file (no change needed)

`select_file()` calls `open_pending()` which returns the existing index if the file is already
open, and sets `active` to it. The dock's `set_active` → `activate_file()` handles focus. No
code change required.

---

## Files touched

| File | Changes |
|---|---|
| `crates/ubiq/src/state/editor.rs` | `temporary` field, `temporary_key`, `open_temporary()`, `baseline()` helper |
| `crates/ubiq/src/state/explorer.rs` | `CopyFullPath`, `OpenInSystem` actions, `ShiftEnter` key, updated `menu_entries()` |
| `crates/ubiq/src/ui/explorer.rs` | Remove git dot, flex spacer, modifier-aware click, double-click handler, shift-enter binding |
| `crates/ubiq/src/ui/editor.rs` | Remove `·` from label, rename `state_colour` → `dirty_colour`, add `git_colour()` |
| `crates/ubiq/src/ui/dock/mod.rs` | `TabInfo` struct, extended `tab()` returning title colour + dot colour + temporary |
| `crates/ubiq/src/ui/dock/skin.rs` | No left dot, right dot after title, title git colour, italic+bg for temp, scrollable tab bar |
| `crates/ubiq/src/ui/file_tab_menu.rs` | Add "Copy Full Path" + "Open in Finder/Explorer" to `ITEMS` |
| `crates/ubiq/src/app.rs` | `click_explorer_row` with permanent flag, `double_click_explorer_row`, `select_file_temporary`, `activate_file` temp promotion, `copy_full_path_for_tab`, `open_in_finder_for_tab`, updated `pick_file_tab_menu` indices, updated `press_explorer_key` for shift-enter |

## Documentation

Run `just docs-touched` after the diff. The change touches `features/workbench.md`. Update it in
the same commit per `_docs/_meta/authoring.md`.
