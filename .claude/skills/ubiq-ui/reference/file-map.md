# Every module in `crates/ubiq`

Three layers: `state/` holds data and mutators and renders nothing; `app/` is `AppState`, the
root view and only owner of state; `ui/` is the element tree. Line counts are a rough guide to
where the weight is, not a rule.

## `state/` — what the workbench knows

Re-exports live in `state/mod.rs`; the work's own records are **not** re-exported — they are
`ubiq_proto::work`'s, and naming them at each use site keeps the dependency direction visible.

| Module | Holds |
|---|---|
| `agents.rs` | The agents screen's view of the work: who is on screen, how they spread across parallel columns, who is on the bench. Owns `COLUMN_MIN_WIDTH`, `COLUMNS_MAX`, the composer row constants |
| `board.rs` | The tasks board's *view* state — filter text, which session, the form (`BoardState`, `Field`, `TaskForm`) |
| `chat.rs` | One entry per open chat tab (`ChatTab`, `ChatPick`, `ChatPicks`, `attach_choices`, `StartOffer`) |
| `clone.rs` | Cloning a project, as held while the modal is up |
| `conversation.rs` | One live agent's conversation (`Conversation`, `ConvBlock`, `Pending`, `Run`) — the largest state module |
| `diagrams.rs` | Mermaid: source in, picture out, plus the disk tier that stops it being drawn twice |
| `dock.rs` | What a panel is and where it may sit (`PanelKind`, `PanelClass`, `Region`, `ChatId`, `Visibility`) |
| `editor.rs` | The files open in the centre pane (`OpenFile`, `FileBody`, `FileLanguage`, `SaveState`, `EditorPaneState`). The one module allowed a component-library type, because the widget's state *is* the model |
| `explorer/` | The file tree and each row's git state — `mod.rs` (`ExplorerState`, `FileNode`, `Row`, `GitStatus`), `tree.rs` (listing, merge, expansion), `rows.rs` (flattening to drawable rows), `filter.rs`, `keys.rs`, `menu.rs` |
| `file_picker.rs` | What the picker was asked for, is showing, and has picked. Owns the four dialog sizes a resize clamps against |
| `git.rs` | The Git screen's view of a repository. Owns `SIDEBAR_WIDTH`, `CHANGES_WIDTH`, `DIFF_HEIGHT`, `LANE_PITCH` |
| `image_edit.rs` | The buffer behind an annotated-capture tab (`ImageEdit`, `EditOp`, `ImageTool`, `ShapeKind`) |
| `layout.rs` | Where the graph puts things, kept apart from what they are |
| `logs.rs` | The console's subsystem, level floor, and whether it follows the tail |
| `nav.rs`, `nav/text.rs` | Where the user is, as a value (`Destination`, `Locus`, `View`); and that value as text |
| `navigator.rs` | The ⌘K navigator: one field over everywhere the window can go |
| `orchestration.rs` | The graph's selection, session and state filters, zoom, and what the pointer has hold of |
| `prefs.rs` | What the interface remembers between runs, and the schema it owns |
| `remote.rs` | The "Connect to a remote host" modal's state and its pure parsing |
| `scene.rs` | An Excalidraw file parsed into something a painter walks without asking a question |
| `search.rs` | The search panel: query, options, results so far |
| `settings.rs` | Application settings the interface owns: schema, the overlay's nav, how a blob is read |
| `sink.rs` | The kitchen sink — the app's own test bench and its fixtures |
| `stats.rs` | Which Control tab is up, and the last reading the host answered with |
| `viewport.rs` | How a picture sits in its panel: a zoom and a pan |
| `vim/` | Modal editing — `mod.rs` (mode, half-typed command, effects), `motion.rs`, `object.rs`, `search.rs`, `step.rs` (one keystroke in, a list of effects out) |
| `when.rs` | How long ago something was, as the picker prints it |
| `windows.rs` | A projection of the host's catalogue, and which window holds which project |
| `work.rs` | One project's work, as this window last heard the host describe it |
| `workbench.rs` | The shell's own state: the active rail mode, and which single-open-at-a-time menu is down |

## `app/` — `AppState`

`AppState` owns the window's chrome (rail mode, open panels) and one `OpenProject` per project
the window holds. **A project's state lives as long as the window holds the project** — its panes,
tree and open files are looked up rather than rebuilt, so switching projects kills nothing and
switching back finds the terminals still running. No process handle or pseudo-terminal reaches
this far: a pane is an ID, a title, and an emulator reading one end of the bus.

**Every mutator ends in `cx.notify()`.** One that forgets is a panel that stops updating.

| Module | Holds |
|---|---|
| `mod.rs` | `AppState` itself, `OpenProject`, the `Render` impl, and the debounce/limit constants (`MAX_FILE_BYTES`, `EXPAND_DEPTH`, `CACHE_DEPTH`, `FILTER_DEBOUNCE`, `REFLOW_DEBOUNCE`, `OUTLINE_DEBOUNCE`, `MOVE_UNASKED`) |
| `wire.rs` | The bus: every inbound message handled, every outbound one sent. The largest `app/` module |
| `shell.rs` | Window-level gestures, `cancel_dialog` (the Escape ladder — peels one layer, `cx.propagate()` with nothing up) |
| `boot.rs` | Window opening; `open_project_window` is the only place a window is created |
| `hosts.rs` | The multiplexer: one window's connection to every host it is attached to |
| `remote_connect.rs` | Reaching a remote host from the UI, and the client half of that wire |
| `host_browse.rs` | Filling the shared file picker from a remote host's filesystem, before a project exists on it |
| `projects.rs`, `clone.rs` | Opening, switching and cloning projects |
| `agents.rs` | The agents screen's behaviour: columns, bench, composers |
| `chat.rs`, `panels.rs` | Chat tabs, and the dock's panels |
| `editor.rs`, `explorer.rs`, `picker.rs`, `nav.rs` | Files: open/save, the tree, the picker, navigation and bookmarks |
| `git.rs` | The Git screen |
| `graph.rs`, `board.rs` | Orchestration and the tasks board |
| `settings.rs` | Application and project settings, connectors, accounts, AI providers |
| `stats.rs` | The Control screen |
| `sink.rs` | The kitchen sink |
| `vim.rs` | What turns a keystroke into an edit on whichever input has focus |
| `capture.rs`, `image_edit.rs`, `clipboard.rs` | Window capture into a dirty untitled tab, the toolbar over it, and the clipboard's image arm |

## `ui/` — the element tree

Each screen area is a free function over the root view. Nothing here names the coordinator, a
process, a path on disk or a file descriptor.

### The shell

| Module | Draws |
|---|---|
| `shell.rs` | The skeleton: titlebar, rail, dock, status bar |
| `titlebar.rs` | What is open, where it lives, the switches for the dock's three edge regions, and the back/forward controls (their own helper, because `icon_button` has no room for a control with nowhere to go — `text_faint()` and inert at the history's end) |
| `rail.rs` | The activity rail: destinations, grouped, exactly one active |
| `status_bar.rs` | The bottom strip — open file and caret, or the agents screen's column fill, or the graph's selection |
| `ribbon.rs` | The `alpha`/`beta` build-channel band across the bottom-left corner |
| `dock/mod.rs` | The window's arrangement and `WorkbenchPanel`, the adapter that makes a screen area a panel — a weak `AppState` handle plus a panel kind, its render a `match` delegating to the same free functions |
| `dock/skin.rs` | Ubiq's appearance for the library's dock: tab strip, displayed-tab mark, panel dots, close affordance, drop indicator, resize strips. Ubiq writes no drag, no drop geometry, no layout serialisation |
| `empty.rs` | The pages a screen with nothing to show draws |

### Screens

| Module | Draws |
|---|---|
| `agents/` | `mod.rs` the screen, `column.rs` one parallel column of conversation, `sidebar.rs` every session and agent down the side |
| `board/` | `mod.rs` the cards in columns, `detail.rs` the panel beside them, `form.rs` its editable half |
| `orchestration/` | `graph.rs` cards on a dotted ground joined to whoever spawned them, `inspector.rs` the panel beside it, `tasks.rs` the drawer under it (and the inspector's second tab) |
| `git/` | `mod.rs`, `refs.rs` the ref sidebar, `history.rs` the log and its lanes, `changes.rs` what the selected row is about, `diff.rs` what the selected path changed |
| `stats.rs` | The Control screen |
| `sink/` | The kitchen sink — `style.rs` is **the style reference: every token, surface, control and field on one page**, and where every new kit primitive gets its specimen. Also `docs.rs`, `files.rs`, `project.rs`, `settings.rs`, `messages.rs` |
| `settings.rs` | Application settings over the window: a nav, a column of rows, a fixed-size panel. The largest UI module |

### Panels and content

| Module | Draws |
|---|---|
| `explorer.rs` | The file tree panel |
| `editor.rs` | One open file — and `welcome(app)`, the no-file page: Ubiq's mark at 200px, half opacity, blue on light and white on dark |
| `outline.rs` | The definitions in the buffer on screen. Parses the buffer a *second* time on `cx.background_spawn` behind a 200ms debounce, because the library's tree is unreachable (`G153`) |
| `terminal.rs` | One pane's terminal. `config()` builds the emulator palette — the only place in the UI that converts a token into anything but a GPUI colour |
| `chat/` | `mod.rs` the panel, `sidebar.rs` the tab's head |
| `conversation/mod.rs` | One live agent's conversation, drawn once for every surface that shows one. `ConversationView` carries `header`/`footer`/`composer` flags; `lifecycle` and `lifecycle_menu_enabled` are read in exactly one place regardless of caller |
| `logs.rs` | The log console, a dock panel like any other |
| `search.rs` | The project search panel |
| `viewer/` | What draws a file when it is not plain text — `markdown.rs`, `diff.rs`, `image.rs`, `image_edit.rs`, `diagram.rs`, `scene.rs` (paints a scene's own colours straight through), `viewport.rs` (fills its panel, starts fitted, wheel zooms, drag pans) |
| `work.rs` | **What a work record reads as**: the colour an activity or bucket takes, the glyph a role wears. One file, so no screen can disagree |

### Overlays and menus

| Module | Draws |
|---|---|
| `file_picker.rs` | The resizable file dialog |
| `file_dialog.rs` | The questions a file gesture asks |
| `navigator.rs` | The ⌘K list under the titlebar's command field — anchored, no scrim, keys bound on the field |
| `project_menu.rs` | The project picker |
| `new_pane_menu.rs` | Which harness or shell a new pane runs |
| `file_tab_menu.rs` | A file tab's right-click menu |
| `clone.rs` | The clone modal |
| `remote_connect.rs` | The "Connect to a remote host" modal |

### `kit/`

See [`kit-and-theme.md`](kit-and-theme.md).

## `tests/`

`crates/ubiq/tests/` — state-level tests that drive `AppState` and assert on state, not pixels.
One file per area (`agents`, `board`, `chat`, `conversation`, `dock`, `explorer`, `git`,
`navigator`, `orchestration`, `search`, `settings`, `sink`, `stats`, `vim`, `windows`, …), plus
the ones that assert a convention: `dismiss.rs` (every modal has a rung in the Escape ladder),
`panel_reentrancy.rs`, `mode_restore.rs`, `when.rs`.
