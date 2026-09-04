---
id: inbox-shell
title: Proposal — operating-system shell integration
kind: proposal
status: proposal
summary: The four gestures that connect Ubiq to the desktop around it — a file dropped on the window or the app icon, a folder dropped the same way, copy and paste between the explorer and itself, and copy, paste and drag between the explorer and the operating system — the one seam an absolute path is allowed to cross, and the file-family messages that none of it can be built without.
read_when: you are deciding what a drop on the window does, how a dropped file or folder becomes a tab or a project, what the explorer's clipboard means, or which host messages create, move, copy or remove a path
updated: 2026-09-03
depends_on: [tech-architecture, tech-transport, feat-workbench, inbox-omni]
---

# Proposal — operating-system shell integration

Ubiq is a desktop application that cannot be handed a file. Dragging one onto the window does
nothing; dragging a folder onto the window does nothing; dropping either on the dock icon does
nothing, because the bundle declares no document types at all. The explorer copies a path *as text*
and cannot paste one. The only piece of Ubiq that has ever accepted a dropped file is the vendored
terminal, which quotes it and types it into the pseudo-terminal
(`vendor/gpui-terminal/src/view.rs:909`).

This proposes the whole of that surface as one design, because it is one design: four gestures, one
question about where an absolute path is allowed to exist, and one small family of host messages
that three of the four gestures are blocked on.

## 1. Where it stands

**The interface has drag and drop, and none of it is external.** `on_drag`, `on_drag_move` and
`on_drop` are used for the tasks board (`crates/ubiq/src/ui/board/mod.rs:245`), the agents columns
(`crates/ubiq/src/ui/agents/column.rs:89`), the orchestration graph
(`crates/ubiq/src/ui/orchestration/graph.rs:176`) and the dock's own panel tabs
(`crates/ubiq/src/ui/dock/skin.rs:420`) — every payload an in-process Rust type. `ExternalPaths`
appears nowhere in `crates/ubiq`. **The explorer and the editor have no drop handler of any kind.**

**The clipboard is write-only and text-only.** `cx.write_to_clipboard(ClipboardItem::new_string(…))`
is called three times — *Copy path*, *Copy full path* on an explorer row (`crates/ubiq/src/app/explorer.rs`)
and the same on a file tab. There is no `read_from_clipboard` in `crates/ubiq` at
all, so no paste of any kind exists outside the terminal's own `arboard` path.

**Nothing on the bus creates, moves, copies or removes a path.** The file family is
`ProjectTree`, `ReadProjectFile`, `WriteProjectFile`, `DiffProjectFile` and their answers, and that
is the complete set ([`../tech/transport-contract.md`](../tech/transport-contract.md)). The
explorer's context menu already draws *New file*, *New folder*, *Rename* and *Delete* and they do
nothing but `cx.notify()` — the documented reason being that the menu should have somewhere to put
them when the host grows the family. `G70` is that row in the backlog. **This proposal is what grows
the family**, and every gesture below except opening a dropped file waits on it.

**The bundle declares no documents.** `_tools/Info.plist` carries a bundle id, a name, an icon and a
minimum system version — no `CFBundleDocumentTypes`, no `LSItemContentTypes`, no
`UTImportedTypeDeclarations`. macOS therefore refuses a drop on the dock icon, greys Ubiq out of
*Open With*, and never sends the application an open event. `just bundle` assembles `target/Ubiq.app`
by hand from that plist and does not sign it.

**No application-level open hook is installed.** `crates/ubiq-app/src/lib.rs` opens a window and
nothing else; `cx.on_open_urls` and `cx.on_reopen` are unused, and `argv` is not read.

**The toolkit is not the obstacle.** At the pinned revision (`gpui` 0.2.2, rev `6840b8d`) everything
the four gestures need is present and public:

| API | Where | What it gives |
|---|---|---|
| `ExternalPaths(pub SmallVec<[PathBuf; 2]>)` | `gpui/src/interactive.rs:685` | a drop payload, usable with the ordinary `on_drop::<ExternalPaths>` |
| `FileDropEvent::{Entered, Pending, Submit, Exited, Ended}` | `interactive.rs:728` | hover feedback while a drag is over the window |
| `ExternalDragPayload::Files(FileDragPaths)` | `interactive.rs:697` | a drag that *leaves* the window as a native file drag |
| `Interactivity::external_drag_payload` | `elements/div.rs:626` | resolved lazily, once, when the pointer exits the viewport |
| `ClipboardEntry::ExternalPaths` | `platform.rs:2473` | files on the system clipboard, both directions |
| `App::on_open_urls`, `App::on_reopen` | `app.rs:269, 279` | Finder open, dock drop, second launch |

**And one fact that decides section 3: the interface already builds absolute paths.** *Copy full
path* and *Open in Finder* join `snap.record.path` with the row's `rel_path` and hand the result to
the clipboard or to `open -R` (`crates/ubiq/src/app/explorer.rs`). The project root
travels in `ProjectRecord`, so the rule as
[`architecture.md`](../tech/architecture.md) states it — no path crosses into UI code — is already
narrower in the tree than in the prose: what holds is that *no path the interface composes is ever
sent back to the host*, and that every file operation is expressed as a project id and a relative
path.

## 2. The four gestures

| # | Gesture | Direction | Blocked on |
|---|---|---|---|
| A | A file dropped on the window, the dock icon, or named in `argv` | in | nothing (§4) |
| B | A folder dropped the same three ways | in | nothing (§5) |
| C | Copy, cut, paste, duplicate *within* the explorer | internal | the host file family (§9) |
| D | Copy, paste and drag between the explorer and the operating system | both | the host file family (§9) |

A and B are one entry point with an `is_dir` branch, and that is the whole of their relationship. C
is Ubiq talking to itself; D is C with the system clipboard and a native drag on the other end.

## 3. The one seam an absolute path crosses

A dropped path is absolute, and it arrives in the interface. There is no arranging around that: the
operating system hands `ExternalPaths` to the window, and the window is `crates/ubiq`. So the design
question is not whether an absolute path can be in UI code — it is there the instant the drop lands
— but **what the interface is allowed to do with it.**

**The rule this proposes: an absolute path entering the interface is a token, not a location.** It
may be passed straight back to the host in the same gesture that produced it, and it may be drawn to
the user. It is never resolved, never read, never opened, never joined to another path, and never
stored past the message it is put in. Everything the interface knows afterwards is a `project_id` and
a `rel_path`, exactly as it is today.

This is the mirror image of the workarea in architecture rule 6: a path the interface is *given*
rather than composes. The workarea is given by the host and used by the interface; a dropped path is
given by the operating system and used by the host. Neither weakens the rule, because in both cases
the half that resolves the path is the half that owns the filesystem.

### Why not a direct read — settled 2026-09-03, reversed 2026-09-03

The other reading of "open a dropped file directly, no host involved" is that the interface reads the
bytes itself with `std::fs` and puts them in an editor. **That was decided against, and the decision
is reversed: a file dropped outside every open project is read directly, as a read-only guest tab
(`D54`).** The cost recorded below is real, and cannot reach a guest tab: it carries no `FileVersion`
at all, and `OpenFile::savable()` refuses a save without one, so it is a viewer rather than a second
editor.

`ProjectFileContents` is not a byte array. It carries a `FileVersion`, and a save that does not name
the version it read is refused — the single mechanism that stops a save landing on a change an agent
made in a pane one second earlier. It carries `is_binary`, and `truncated`, which has no version *by
construction*, so the editor cannot offer a save that would destroy the tail. A `FileError`
distinguishes `Refused`, `Missing`, `WrongKind`, `Denied`, `Conflict` and `Failed`, each a different
thing to do. A UI-side read that fed a *savable* editor would reimplement all of it, or — far more
likely — reimplement none of it and quietly ship an editor whose save can eat an agent's work; a
read-only guest tab has no save to protect and so needs none of it.

It also risked forking the editor, which the reversal answers rather than accepts: a savable second
kind of open file would need a second shape wherever `OpenFile` is resolved, saved or tinted. A guest
tab needs none, having no save path — the same `OpenFile`, `tab_key` and `PanelKind::File`
(`crates/ubiq/src/state/editor.rs`), with one `guest: bool` field.

**What was actually wanted — a file opened without ceremony, with no project created, no catalogue
row and no explorer — costs none of that.** §4 keeps the host round trip first proposed here; the
guest tab that shipped is [`../features/workbench.md`](../features/workbench.md)'s to describe.

## 4. Gesture A — a dropped file opens as a tab

**One message goes out and the interface waits.**

```
OpenExternalPaths  UI → host   { paths: [absolute], window_hint }
ExternalPathsOpened host → UI  { opened: [{ project_id, rel_path, kind }], rejected: [{ path, error }] }
```

The host resolves each path — canonicalising it, following symlinks, refusing devices, sockets and
pipes exactly as `files/path.rs::resolve` already does — and answers with the pair the interface
speaks in. From that point the drop is indistinguishable from a click in the explorer:
`select_file(rel_path)` on the returned project, a tab that says it is reading, and
`ReadProjectFile`. **No new state, no second editor, no second save path.**

**The host answers with one of three project ids, in this order.**

1. **A project already open in this window contains the path.** The most common drop by a wide
   margin — a file dragged out of Finder into the window where its repository is already open. It
   opens as that project's file, with its git badge, its working-tree map, its conflict-checked save.
   The explorer reveals and selects the row, expanding the folders on the way, so the tree agrees
   with the tab.
2. **A project in the catalogue contains the path, open in another window or in none.** The tab
   belongs to that project. Opening it in the window that took the drop would mean a window holding
   a file from a project it does not hold, which the per-project state shape
   (`OpenProject` in `app.rs:171`) does not have a place for. The window holding it is raised and the
   file opens there; if no window holds it, the project opens in the window that took the drop, the
   way the picker's history rows already open one.
3. **No project contains it.** The host mints a **loose project**: root is the file's parent
   directory, `rel_path` is the file's name, id is an ordinary `ProjectId`. It is **not written to
   `projects.toml`, not returned by `ListProjects`, not broadcast as `ProjectAdded`, and has no
   workarea** — it is a resolution root and nothing more, and it is dropped when its last tab closes.
   The interface draws it as a tab and nothing else: no explorer tree, no entry in the project menu,
   no `+` in the pane strip. This is the "simple editor, no project" the gesture asks for, and it
   costs one flag on the host's project record rather than a second half of the interface.

**A loose tab is honest about what it is not.** Its title carries the file name and its tooltip the
containing folder; it takes no git tint, because no repository was discovered for it; and a harness
cannot be started in it. **Several files dropped at once open several tabs**, in the order dropped,
the last one focused, and the rejected list is one notification naming the first path and the count.

## 5. Gesture B — a dropped folder becomes a project

A folder is a project, and the rules the picker already states settle almost all of it
([`../features/workbench.md`](../features/workbench.md)):

- **A folder already in the catalogue opens rather than duplicating.** "Adding a folder already in
  the catalogue points at the project that is there rather than making a second" — a drop is an Add
  without the chooser, so it inherits that sentence unchanged. Open in another window: that window
  is raised.
- **A folder not in the catalogue opens project settings prefilled**, General only, path filled and
  immutable, name from the last component, *Create* sending `AddProject`. Identical to Add, minus
  the folder chooser — the chooser is what the drop replaced. This keeps colour and name in the
  user's hands and keeps exactly one code path into the catalogue.
- **A folder inside an open project is not a project.** Dropping `src/state` from Finder onto a
  window that already holds that repository expands and reveals it in the tree. Making a second,
  nested project record out of it is never what the gesture meant, and the catalogue has no notion
  of nesting.
- **Several folders at once** are one confirmation naming the count, then one settings dialog per
  new folder, in order. A drop mixing files and folders is processed as B for the folders and A for
  the files.

`AddProject` never creates a directory and refuses a path that is not one, so a dropped path that
vanished between the drag and the drop is already an error with a home.

## 6. Where the drop lands

A window-wide drop target is wrong: the window already means four different things in four places,
and the terminal pane means a fifth that works today.

| Drop target | File | Folder |
|---|---|---|
| Editor centre, or the empty brand page | open a tab (§4) | open or add the project (§5) |
| An explorer folder row | import into that folder (§9) | import the folder into it |
| An explorer file row | import into that file's folder | import into that file's folder |
| Explorer empty space, or the tree's root | import into the project root | import into the project root |
| A terminal pane | paste the quoted path — **unchanged, works today** | same |
| The empty state, the project menu, the dock's tab strip | open a tab | open or add the project |

**The drop target is decided by what is under the pointer at `Submit`, and nothing else** — the same
discipline the tasks board follows, where "which task it landed in is worked out from where it came
to rest, not from what took the drop."

**A drag over the window says where it would land.** `FileDropEvent::Entered` and `Pending` give the
pointer position for free: the region that would take the drop draws a border and a wash in the
accent token, an explorer row highlights the folder that would receive the import, and a target that
would refuse the drop draws nothing at all rather than a red state. **Every colour is a theme token**
— no literal colour leaves `theme.rs`, and this needs no new token beyond the accent and the surface
overlay that already exist. `Exited` and `Ended` clear it, and `Ended` is what covers a drag the
system cancelled while the pointer is outside.

**An import is a copy, and the modifier says otherwise.** Dragging from outside Ubiq copies —
anything else silently moves a file out of a folder the user was not looking at. Within the same
project, dragging in the explorer moves, because that is what a file manager does and what the tree
shows. `⌥` forces a copy and `⌘` forces a move on macOS, `Ctrl` and `Shift` elsewhere; the overlay
says which, in words.

## 7. Gesture A and B from outside the window — the dock icon, Finder, and `argv`

Three entry points that are one function.

**The bundle has to declare documents.** `_tools/Info.plist` gains `CFBundleDocumentTypes` with two
entries: `public.folder` (`LSHandlerRank: Alternate`, role `Editor`) and `public.data` /
`public.plain-text` (rank `Alternate`, role `Editor`). This is what makes the dock icon accept a
drop, puts Ubiq in *Open With*, and lets the user set it as a handler. `LSHandlerRank: Alternate` is
deliberate — Ubiq should be offered, never seize the default handler for every text file on the
machine.

**The application installs the hook before the first window.** `cx.on_open_urls` in
`crates/ubiq-app/src/lib.rs`, next to the existing `gpui_component::init` call, converting each
`file://` URL to a path. `cx.on_reopen` covers a dock click with no window open, which today does
nothing. Both must survive arriving **before the host is ready and before a window exists**: paths
queue in a small global, and the queue is drained when the first window has a client. This is the
same shape the bus already has and is the one piece of §7 that is easy to get wrong.

**`argv` is the third door**, and the cheapest: `ubiq .` and `ubiq path/to/file` are what a terminal
user will try first, and `open -a Ubiq <path>` on macOS routes through the same handler. One
`deliver_paths(paths, window_hint)` takes the drop, the URL event and `argv` alike, and its whole
body is the message from §4 or the project flow from §5.

**Other platforms get the argv door only, for now.** A Linux `.desktop` file with `MimeType=` and
`%F`, and a Windows file-association shim, are the same idea and neither is in the build today —
`just bundle` is macOS-only. Scoping this proposal to macOS for the shell integration and to every
platform for `argv` is honest about what `just bundle` produces.

## 8. Gestures C and D — the clipboard

**Four flows, and the interface's own is the one that needs no platform work.**

**Copy and paste inside the explorer (C).** `⌘C`, `⌘X`, `⌘V`, `⌘D` and the context-menu rows the
menu already draws. The cut or copied set is **an in-window value — project id and relative paths —
not the system clipboard**, so a cut inside Ubiq cannot be pasted into Finder by accident and a stale
cut cannot outlive the project. A cut row draws at reduced opacity until it is pasted or the cut is
cleared by `escape`. Paste into a folder with a name collision offers *Replace*, *Keep both* — which
appends ` copy`, then ` copy 2` — or *Cancel*, one dialog for the whole set with an *apply to all*.
Pasting into a folder inside the cut source is refused before any message is sent.

**Copy from the explorer to the operating system (D).**
`ClipboardItem { entries: vec![ClipboardEntry::ExternalPaths(…), ClipboardEntry::String(path)] }` —
the paths for a file manager, the string so a paste into a terminal or a chat still gets text. The
absolute path here is composed by the interface from `snap.record.path`, exactly as *Copy full path*
already does, and this proposal fences that: §13 states the rule that such a path may go to the
operating system and never back to the host. **Whether the pinned gpui writes an `ExternalPaths`
entry to the macOS pasteboard in a form Finder accepts is unverified** and is the one API risk in
this document; the fallback is the string entry, which is today's behaviour.

**Paste from the operating system into the explorer (D).** `read_from_clipboard`, take the
`ExternalPaths` entry if there is one, and run the import of §9 into the selected folder. A clipboard
holding only text is not a file paste and does nothing.

**Drag out of the explorer to the operating system (D).** `on_drag` with the row's payload, then
`external_drag_payload` resolving to `ExternalDragPayload::Files(FileDragPaths::new([(abs, is_dir)]))`.
The resolver runs **at most once, when the pointer leaves the window**, which is exactly the right
moment to compose the absolute path and means an in-window drag never composes one at all. `is_dir`
is already on the explorer's `FileNode`, so nothing is stat-ed to answer it.

## 9. The file family this needs

Six messages, in the shape the family already has: every one names a project by id and a path by
`rel_path`, answers only the window that asked, and fails as a `ProjectFileError`.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `CreateProjectPath` | UI → host | `project_id`, `rel_path`, `kind` | `ProjectPathChanged` or `ProjectFileError` |
| `MoveProjectPath` | UI → host | `project_id`, `from`, `to`, `overwrite` | `ProjectPathChanged` or `ProjectFileError` |
| `CopyProjectPath` | UI → host | `project_id`, `from`, `to`, `overwrite` | `ProjectPathChanged` or `ProjectFileError` |
| `RemoveProjectPath` | UI → host | `project_id`, `rel_path`, `trash` | `ProjectPathChanged` or `ProjectFileError` |
| `ImportPaths` | UI → host | `project_id`, `dest_rel`, `sources[]` *(absolute)*, `mode`, `overwrite` | `ProjectPathChanged` or `ProjectFileError` |
| `ProjectPathChanged` | host → UI | `project_id`, `listings[]`, `moved[]` | — |

**`ProjectPathChanged` carries listings, not events.** Nothing watches a project's folder (`G34`), so
a mutation whose answer was "done" would leave the tree lying until the user collapsed and expanded
the folder. Answering with a fresh `DirListing` for every affected parent means the existing
`ExplorerState::merge` redraws the tree with no new code, and `moved[]` — pairs of old and new
`rel_path` — is what lets an open tab follow its file instead of going stale. When a real watcher
lands, it delivers the same listings unsolicited, which the merge already accepts.

**`ImportPaths` is the only message in Ubiq that carries an absolute path**, and it is always a path
the operating system handed the interface during the gesture being processed. Everything in §3
exists to keep that sentence true and checkable by grep.

**`RemoveProjectPath` prefers the system trash.** `trash: true` is the default and the only thing the
explorer's *Delete* offers; an unrecoverable unlink is not a menu row. On a platform or a filesystem
with no trash the host answers `Refused` and the interface says why, rather than silently deleting.

**`FileError` grows two arms**, `Exists` and `NotEmpty`, because "the destination is already there"
and "the folder you are removing is not empty" are two different dialogs and neither is `Failed`.

**Every one of these runs in the host's existing file worker pool** (`files/mod.rs::Files::submit`),
so a folder copy of ten thousand entries does not block the coordinator. An import over a ceiling —
count or bytes — is refused up front with `Refused` rather than started and abandoned halfway.

## 10. What can go wrong, and what is decided about it

- **A path that escapes the project.** `files/path.rs::resolve` already refuses one after symlink
  resolution; `to` and `dest_rel` go through it unchanged.

- **A folder dropped into its own subtree**, or a file onto itself. Refused in the interface — a
  prefix test on two relative paths, no host round trip.
- **A `.app` bundle, a `.bundle`, an `.xcodeproj`.** These are directories, and a folder drop of one
  would offer to make it a project — which is almost never meant. The host reports them as
  `EntryKind::Package` and the interface treats the drop as gesture A: it opens the bundle's folder
  in a loose tab rather than adding a project. Recommended, and listed in §12 as reversible.
- **A drop while the host is starting**, or onto a window whose project is unhealthy. The queue in §7
  covers the first; the second answers `Missing` or `Denied`, the interface's existing cue to send
  `RefreshProject`. A very large import is bounded by the ceiling above, reports progress in the
  status bar, and is not cancellable in the first phase.
- **Sandboxing.** A confined pane's policy has nothing to do with any of this — an import is the
  host's own file work — but an imported file *is* visible to every agent confined to that project.

## 11. Phases

**P1 — the drop that opens something.** `ExternalPaths` drop handlers on the centre, the explorer and
the empty state; `OpenExternalPaths`/`ExternalPathsOpened` and the loose project; the folder flow
into the existing settings dialog; `Info.plist` document types; `on_open_urls`, `on_reopen` and
`argv` behind one `deliver_paths`. **No new file-mutation message.** This is the whole of the user's
first two asks and depends on nothing else in this document.

**P2 — the host file family.** §9 in full, closing `G70` and the four dead context-menu rows, plus
internal explorer copy, cut, paste, duplicate, rename and delete.

**P3 — the operating system's clipboard and drag out.** `ClipboardEntry::ExternalPaths` both ways,
`external_drag_payload` on explorer rows, imports by drop onto a folder row.

**P4 — the polish P1 skipped.** Reveal-and-select on an external open, progress for a long import,
*Keep both* naming, and whichever of §12 came back the other way.

## 12. What this asks to be decided — rows (a) and (c) settled and reversed, the rest still asking

| | Question | Recommendation |
|---|---|---|
| a | Does a dropped file outside every project open through the host as a loose project, or does the interface read it directly? | **Settled 2026-09-03, reversed 2026-09-03 — the interface reads it directly, as a read-only guest tab.** `OpenFile::savable()` now also requires a version, so a guest tab (built with `version: None`) cannot reach a save — the failure the loose-project answer was chosen to avoid. `D54`. |
| b | Does a loose tab survive a restart? | **No.** It is not in the catalogue and nothing about it is worth persisting; reopening is one gesture. |
| c | Does a folder drop open project settings, or create the project silently? | **Settled 2026-09-03, reversed 2026-09-03 — it opens immediately as a temporary project.** The host mints the record with a `temporary` flag and never writes it to the catalogue; naming it in project settings, opened from the titlebar's `+`, is what keeps it. `D54`. |
| d | Copy or move, when dragging from outside? | **Copy**, with `⌘` to move. Within the project, **move**, with `⌥` to copy. |
| e | Does *Delete* trash or unlink? | **Trash**, and `Refused` where there is no trash. |
| f | Is a macOS package a project or a document? | **A document** — a loose tab on its folder. Reversible if it proves annoying. |
| g | Does Ubiq claim any default file association? | **No.** `LSHandlerRank: Alternate` only. |
| h | Does the pinned gpui write file paths to the macOS pasteboard in a form Finder pastes? | **Unverified.** The one API risk; the string entry is the fallback and is today's behaviour. |

## 13. Rules this adds

- **An absolute path in the interface is a token, not a location.** It may be sent straight back to
  the host in the gesture that produced it, or handed to the operating system, and it is never
  resolved, read, opened, joined or stored. `ImportPaths` is the only message carrying one.
- **A dropped file is opened by the host, like every other file.** There is one editor, one save
  path, one version rule.
- **A drop target is what is under the pointer when the drop lands**, and it says so in theme tokens.
- **A mutation answers with listings** — nothing watches the folder.

## 14. Rows this proposes for the backlog

| Id | Row |
|---|---|
| G100 | Nothing accepts a dropped file or folder: no `ExternalPaths` handler exists in `crates/ubiq`, and only the vendored terminal has one |
| G101 | `_tools/Info.plist` declares no `CFBundleDocumentTypes`, so the dock icon refuses drops and Ubiq is absent from *Open With* |
| G102 | `argv` is ignored and `on_open_urls`/`on_reopen` are not installed, so `ubiq .` and `open -a Ubiq <path>` do nothing |
| G103 | The clipboard is write-only and text-only in `crates/ubiq`; no `read_from_clipboard` and no `ClipboardEntry::ExternalPaths` in either direction |
| G104 | The explorer cannot drag a file out to the operating system, though `external_drag_payload` and `FileDragPaths` are available at the pinned gpui revision |

`G70` already records the dead context-menu rows and is what §9 closes.

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — the locality rules, and the workarea this proposal mirrors
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the file family §9 extends, and the `rel_path` discipline
- [`../features/workbench.md`](../features/workbench.md) — the explorer, the editor tabs, the project picker and the empty state
- [`../backlog.md`](../backlog.md) — `G34` (nothing watches a folder), `G70` (the dead menu rows)
