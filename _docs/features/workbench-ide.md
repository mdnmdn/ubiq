---
id: feat-workbench-ide
title: IDE mode — the explorer and the editor
kind: feature
status: draft
summary: The rail's IDE mode — the project's file explorer and its right-click menu, the editor tabs each open file is a panel of, the viewer that draws one by kind, Markdown reading width and its minimap, diagrams and Excalidraw scenes, the image editor over any picture, and how a file is saved.
read_when: you are changing the explorer tree, the editor tabs, what a file panel draws, which viewer draws it, how a diagram is rendered or cached, capturing the window, editing a picture, or saving a file
updated: 2026-09-25
verified: 2026-09-25
code_anchors: [crates/ubiq/src/ui/explorer.rs, crates/ubiq/src/app/explorer.rs, crates/ubiq/src/state/explorer/mod.rs, crates/ubiq/src/state/explorer/tree.rs, crates/ubiq/src/state/explorer/rows.rs, crates/ubiq/src/state/explorer/menu.rs, crates/ubiq/tests/explorer.rs, crates/ubiq/tests/files_changed.rs, crates/ubiq/src/ui/kit/files.rs, crates/ubiq/src/ui/editor.rs, crates/ubiq/src/state/editor.rs, crates/ubiq/src/ui/mark.rs, crates/ubiq/src/app/mark.rs, crates/ubiq/src/ui/viewer/mod.rs, crates/ubiq/src/ui/viewer/diff.rs, crates/ubiq/src/ui/viewer/markdown.rs, crates/ubiq/src/ui/viewer/md_options.rs, crates/ubiq/src/ui/viewer/diagram.rs, crates/ubiq/src/ui/viewer/scene.rs, crates/ubiq/src/ui/viewer/viewport.rs, crates/ubiq/src/ui/viewer/image.rs, crates/ubiq/src/ui/viewer/image_edit.rs, crates/ubiq/src/ui/viewer/web.rs, crates/ubiq/src/ui/kit/md_navigator.rs, crates/ubiq/src/ui/kit/minimap.rs, crates/ubiq/src/app/capture.rs, crates/ubiq/src/app/feedback.rs, crates/ubiq/src/state/feedback.rs, crates/ubiq/src/ui/feedback.rs, crates/ubiq/src/app/image_edit.rs, crates/ubiq/src/state/image_edit.rs, crates/ubiq/tests/image_gestures.rs, crates/ubiq/src/app/clipboard.rs, crates/ubiq/src/state/diagrams.rs, crates/ubiq/src/state/viewport.rs, crates/ubiq/src/state/scene.rs, crates/ubiq/tests/diagrams.rs, crates/ubiq/tests/viewport.rs, crates/ubiq/tests/scene.rs, crates/ubiq/tests/viewer_kind.rs, crates/ubiq/src/ui/file_dialog.rs, crates/ubiq/src/ui/web_view.rs, crates/ubiq/src/app/web_panel.rs, crates/ubiq/src/state/web_panel.rs]
depends_on: [feat-workbench, tech-ui]
review_cycle: monthly
---

# IDE mode — the explorer and the editor

## Purpose

IDE is the rail mode a project's files are read and written in: the explorer down the left, one
editor tab per open file in the centre, and a viewer chosen from the file's own kind to draw it.
It is where Ubiq is a text editor, and the mode the workbench opens a file into from anywhere
else. The chrome around it — the rail, the dock, the tabs and the status bar — belongs to
[the workbench](./workbench.md).

## Behaviour

**The explorer draws the project's folder, one directory at a time, in the same two arrangements
the file picker uses.** Opening a project asks the host for its top level; expanding a folder asks
for that folder's children, and only then. A repository's `node_modules` is therefore one row
rather than a walk, and a tree the user never opens costs nothing. The tree is the folders that
have been opened, indented; the list is every match the host has already named, flat and sorted by
name without case, each row saying which folder it came from. **The view switch in the panel header
is the one control that says which arrangement is on**, and what was typed survives the toggle: one
filter field sits over both, under the header. A filter finds rather than prunes: every folder
already listed is walked while one is typed, and a folder with nothing matching under it drops out
instead of drawing as empty. **A branch a filter found is drawn open and stays collapsible**: the
twisty and the arrow keys still work while a query is typed, and shutting one takes its matches off
screen and leaves its own row as the way back in. That is a per-filter override, dropped when the
field clears — shutting a branch to read one search result is not a decision about how the tree is
left, so the folders the window writes down are untouched, and a branch drawn shut stops being
prefetched. **The field starts searching at three characters**, because one or two
letters match nearly everything in a project and cost a full walk to answer with a screen the user
has to narrow anyway — anything shorter is treated as an empty field and the tree is left alone. A
hit on a folder the host has never listed is asked for, so a matched folder answers with its
contents; the walk's skip set is still never asked for. **The listings are cached in the window, filled in the background when the
project opens**, so a search reads what is already named rather than waiting on the host. The walk's
skip set is left alone — `node_modules` stays one row — and a cached listing does not expand the
tree: collapsing is still not forgetting, and an unfiltered tree still shows only what the user
opened. **A filter is debounced and walked off the frame, on a background thread.** Typing a letter does
not clone or walk the cache on the keystroke: the field keeps the draft, the last result stays on
screen, and after a short pause one snapshot — an `Arc` of the tree, not a copy — is walked on the
background executor. Clearing the field is immediate. Clicking a folder in
the tree expands it; clicking a file opens it; a folder in the list is only where the cursor lands.

**Hidden files are off by default, and one switch shows them.** The eye at the end of the panel
header is that switch, and the same one is a row in application settings — it is a reading habit
rather than a fact about one project, so it is global and every open tree follows it at once. With
it off a dotfile is not a row at all: a hidden folder takes its whole subtree with it, in the tree,
in the flat list and in what the filter can reach, and the background cache does not list a folder
the tree would not draw. Hidden is read from the name — a leading dot, the same rule the host
applies when it browses a machine — because the wire carries no flag for it, and it is a separate
thing from `LIST_HIDE`, which drops junk like `.DS_Store` from *every* listing whatever this switch
says.

**The project is the tree's first row.** It carries the project's name, sits above everything with a
twisty of its own, and collapsing it puts the whole tree away behind one handle. Its path is the
empty one — the same path that already means "the project's own folder" everywhere else in the file
family — so a right-click on it offers a folder's menu, a drop on it moves the path to the top level,
and a New file made from it lands there. It is the one row that cannot be dragged, renamed or
removed: there is nowhere to put a project inside its own tree, and the host refuses its path for the
same reason. The flat list has no such row, because a list is flat by definition and there is nothing
for a root to contain.

**The explorer is worked from the keyboard, and the filter and the tree are two focuses** — the
convention [`tech/ui-and-design.md`](../tech/ui-and-design.md) states for every tree in Ubiq. `⌘P`
(and `ctrl-p`) reveals the explorer and puts the caret in its filter from anywhere, including from
inside another field. `down` and `tab` step from there onto the tree, `tab` and `shift-tab` step
back off it onto the field, and the keys below are live only while the tree holds the keyboard.
`up` and `down` move a cursor bar through the rows and stop at the ends; `right` opens
the folder the cursor is on and then steps into it, `left` shuts it and then steps out to the
folder holding it; `enter` opens a file in a temporary preview and toggles a folder, `shift-enter`
opens the file permanently; `escape` closes the right-click menu, then clears the filter, then is
handed back. `enter` and `escape` are the two the field keeps as well, so a query can be opened or
cleared without leaving it. What the panel has no answer for it hands back. The cursor is not
the open file: the accent is the file that is open, the keyboard's bar is only where the next key
lands.

**Backspace removes the row the cursor is on — Delete does the same, and both work on every
platform.** Backspace is what a macOS file manager uses and Delete is what the others use, so each
is bound everywhere rather than made to depend on which operating system the keyboard came with. It
raises the same question the menu's Delete row does, with the same Shift on it: on its own the path
goes to the Trash, and with Shift held it is removed outright. **Neither key exists at the field**,
because Backspace is how a query is corrected and a tree that deleted a file because the field had
run out of letters would be indefensible. The project's own row is refused, as are the rows the
host will not follow.

**Clicking a row puts the keyboard on the tree.** A single click previews a file and leaves the
focus where the next arrow key is useful; a permanent open — double-click, Shift+click, ⌘+click —
takes the keyboard to the editor, because the file is where the user is now going.

**A three-state follow button sits in the panel header.** Off, revealed once, locked, cycled by
clicking it. *Once* reveals the active editor's file in the tree and stops there; *locked* reveals
every file that becomes the active editor, however it became active — a tab brought forward, a file
opened from the tree, the picker or a search. **Revealing opens the folders above the file and
scrolls the row into view**: it runs through the same `wanted` list a restore uses, so a folder the
host has never listed is asked for and opened when the answer lands. Nothing is collapsed — a
reveal opens the way to one file, it does not tidy the rest of the tree away. It is never written
down, so a window opens with it off.

**A right-click on a row raises a menu at the pointer.** A file offers Open, Open diff vs HEAD, Copy
path, Copy full path, Copy link, Open in Finder, Open in Web, New file, New folder, New Excalidraw,
New draw.io, Copy, Paste, Duplicate, Rename and Delete; a folder offers Expand or Collapse, New file,
New folder, New Excalidraw, New draw.io, Copy path, Copy full path, Copy link, Open in Finder, Open
in Web, Refresh, Copy, Paste, Duplicate, Rename and Delete; a click on the
empty panel offers New file, New folder, New Excalidraw, New draw.io, Paste and Collapse all, because
that is where the actions that need no row live. **New file, New folder, New Excalidraw, New draw.io
and Paste land in the folder the row is in**, which is
the row itself when it is a folder and the one holding it when it is a file — so a file row offers
them rather than making the user find its folder first. Only Paste is ever disabled, and only while
nothing has been copied.

**The rows are grouped by what they are for**, with a hairline between groups: opening, then making,
then the clipboard, then the ways of naming a path, then Refresh, then renaming and removing. A
group with nothing in it takes its separator with it, so an unreadable row's menu is one group and
no lines. **A separator holds a position in the menu's list as well as on screen**, because a pick
comes back as an index and rows drawn without matching entries behind them would silently shift
every action below one. There is no Expand or Collapse row: the twisty and a click on the row
already do it, and a third way to say the same thing is a row that earns nothing. `Open in Finder` — Explorer or File Manager on other platforms — reveals
the file or its folder in the system's file manager. `Open in Web` starts the local web-export server
if it is not already running and opens that file or folder in the system browser — see the titlebar's
browser button below; unlike every other row here it needs nothing from the host, since the interface
already reads the project's own files for it (`D55`). `Refresh` asks the host to list that folder
again. The menu is the window's one open menu, dismissed by the first click outside it or by escape.
**A menu is identified rather than flagged:** each one is stamped as it opens, and an outside click
closes only the menu it was drawn for. That is what lets a right-click on a second row raise its menu
in the same event that dismisses the first, without the new one being shut by the old one's
dismissal.

**Making, moving and removing a path is one message with an op on it**, which the transport contract
owns and `D57` explains. What the panel adds is the gesture and the question in front of it.

**Every gesture that cannot be retyped asks first, and the question says which one it is about to
do.** New file, New folder, New Excalidraw, New draw.io and Rename each raise a modal with a single
field, seeded with the leaf name for a rename, `drawing.excalidraw` for New Excalidraw,
`diagram.drawio` for New draw.io and empty for New file and New folder, and the confirm stays dim
until the name is both non-blank and different from what is already there. New Excalidraw and New
draw.io force their extension onto whatever is typed rather than merely suggesting it, so deleting
the seed's suffix cannot turn a drawing into a plain text file — the modal's title itself follows the
extension being created (`New Excalidraw`, `New draw.io`, or `New file` for any other one) rather
than a flag saying only that some extension is set. **Enter
confirms and Escape cancels**, in every
one of them: Enter reaches a field's dialog as the field's own key rather than as a binding, since
taking Enter at the depth a text box holds focus would take it from the chat composer too, and both
keys are handed straight back when no dialog is up so a bare Escape still reaches the explorer and
the panes. Escape is not this dialog's own binding — it is the window's one handler,
`AppState::cancel_dialog`, which takes whichever overlay is topmost and peels a dropdown before the
modal under it; `_docs/tech/ui-and-design.md` holds the rule and the order. Enter refuses exactly what the dimmed button refuses, so it can never do what the button
will not. A new file opens in the editor once the host
says it exists, because a file made and not shown is a gesture with no visible effect.

**Delete offers the Trash, and Shift asks for the other thing.** The row reads Delete, and what it
does is decided when it is clicked: on its own it moves the path to the platform's trash, and with
Shift held it removes it outright. The confirmation names which — that it can be restored from the
Trash, or that nothing comes back — and its button says `Move to Trash` or `Delete permanently`, so
the difference is read rather than remembered. A folder is named as taking its contents with it. The
modifier is read at the click and not while the menu is open, which is why the label is the neutral
one: a row whose wording depended on a key held during a menu that does not redraw would go stale
without saying so.

**A row is dragged onto a folder to move it, and onto the panel's own background to move it to the
project's root.** The folder under the pointer lights up, which is the only answer the user gets
before letting go. A file moves on the drop with no question — it is one path, and a rename already
asks. A **folder** raises a confirmation, because a mis-drop can rehome a whole subtree, and that
confirmation carries a *Don't ask again for 10 minutes* checkbox: ticking it lets the next folder
drag through silently, for ten minutes, per window and only in memory. Ten minutes is not a
preference — a user reorganising a tree wants the question gone for as long as that takes and back
again afterwards, and there is nothing about it worth persisting. A drop that changes nothing, a drop
onto the row itself, and a drop of a folder into its own child are all refused in the panel without
a round trip, so no confirmation is raised for a move that could never happen.

**Copy, Paste and Duplicate work off one remembered path, and never the system clipboard.** Copy
remembers the row and asks the host nothing; Paste copies it into the folder that was right-clicked,
or into the root from the empty panel; Duplicate is a paste into the path's own folder. A name
already taken is stepped past — `notes.txt` becomes `notes copy.txt`, then `notes copy 2.txt` — which is
what makes Duplicate work at all, since there a collision is certain. That stepping reads the
children the host has already named, so it is best-effort by construction: a collision it cannot see
comes back from the host as a refusal on the row. There is no Cut, because a move is a drag.

**A path the host moved or removed takes its tabs with it.** A rename or a drag retargets every open
tab at or under the old path; a delete closes them. The explorer's remembered copy is forgotten if it
was the path that went.

**A row the host will not follow is drawn and does nothing.** A symlink leading out of the project or
nowhere, a socket, a device, a pipe: the row appears, faint, and takes no click. Drawing it is the
point — a tree with rows missing is a tree that lies about what is in the folder.

**The explorer holds project-relative paths and nothing else.** No absolute path reaches the
interface, for the same reason no file descriptor does: the folder the tree describes is the host's,
and the two need not be on one machine.

**A change on disk reaches the window without being asked for.** The host watches the folder of the
project a window has open and reports what changed; the explorer re-lists the parent of each changed
path, but only the folders it already holds listed, so a change deep inside a shut folder costs
nothing. A burst too large to name its paths names nothing, so what is on screen is re-listed
instead: the root and every folder open under it, because one listing is a single level deep and the
root alone would leave an open folder holding what it held before the burst. Every open tab is in
the same position — nothing says which files moved — so each clean background one is read again. A change to the git
directory refreshes the overview and the working-tree map, the same refresh a save already sends.
Nothing is redrawn from the message itself — it names paths and carries no content, so the window
asks for what it wants the ordinary way.

**The explorer states git position by colour and by badge.** Modified, untracked, conflicted and
staged each take a colour from the status group and a single-letter badge, the colour so it
reads at a glance and the badge so it does not rely on colour alone. Ignored takes the faint colour
and no badge: it marks a path git is not tracking, not a change to act on. The badge sits at the
row's right edge, aligned under its fellows by a spacer after the name, with no separate status dot
— the badge already says with colour whatever a dot would. The host's working-tree map fills those
in: a path in the map gets a status, a path not in it is clean, and until a map has
arrived every row is unmarked because nothing has been read. An untracked or ignored directory
paints every child the same, because git does not look inside and a child not in the map is not
clean, but that inheritance stops at a nested repository's own root: its files carry their own
status from the merged map, not the outer folder's, so an untracked directory holding a clean
nested repository does not paint that repository's files untracked. Clean and unread draw the same
on the row; the status bar's branch is how a repository is known. A project that is not a
repository draws no badges and no branch.

**A folder that is itself a repository carries a branch label of its own.** In both the tree and
the list, a `git-branch` icon and the head's label — the branch name, `detached <id>`, or the
unborn branch's name — follow the folder's name, faint and small, whether the repository is a
submodule or an independent clone nested inside the project; only the row's tooltip says which,
reading "submodule — <head>" or "repository — <head>", or "<kind> (unreadable)" for a repository
the host could not open.

**A tab exists from the click that asked for the file.** It appears at once, says it is reading, and
fills when the bytes arrive — so a click has an effect, a second click cannot ask for the same file
twice, and a read that fails has somewhere to say so. Bytes that arrive for a project the window has
since switched away from are still put in their tab; bytes for a tab that has been closed are
dropped.

**A file dropped from outside every open project opens as a guest tab.** The interface reads the
bytes itself with `std::fs` — no host, no bus — and builds the same `FileContents` value a host read
would produce, version included: `FileVersion` comes from the same stat the read already took, kept
only when the read was not truncated, on the file family's own rule. It is hosted by the currently
active project so it has somewhere to live among the panels, drawn in a muted colour rather than the
repository's, and its tab key is the file's absolute path rather than a project-relative one. It
goes through the same arrival path as any other file from there, so images, markdown and the text
editor all work unchanged. **It saves like any other tab** — an untruncated body and a version are
still what `savable()` requires — except the write is `WriteHostFile` (an absolute path, no project)
rather than `WriteProjectFile`, and the host answers with `HostFileWritten`/`HostFileError` rather
than a project-scoped reply; see `transport-contract.md`'s host browse family. There is no git badge
for it and no save-as prompt for a missing folder — `WriteHostFile` refuses anything but an exact
overwrite of a file that already exists, on the same version it was read at. A file dropped that is
inside an open project opens normally through the host instead, with its git badge. A folder dropped
anywhere else becomes a temporary project instead of a guest tab, above.

**The drop target is the editor centre and the file tabs, and nothing else yet.** The explorer and
the chat panel deliberately do nothing with an external drop; the terminal pane keeps its existing
behaviour of quoting the path into the pseudo-terminal.

**Each open file owns its buffer.** Switching tabs and switching projects both leave a buffer exactly
as it was, with its undo history, its selection and its scroll — nothing is copied in or out. The
active tab is marked on its bottom edge. The tab's title takes its colour from the repository, the
same map the explorer draws on, and a small dot at the tab's right edge reports the *file*: reading,
saving, a failed save, or an unsaved edit. A clean, idle tab draws no dot.

**A file opens in a temporary preview on a single click, permanently on a double-click**, when
*Open files in previews* is on — which is the default, and a setting. Off, a single click and
`enter` open permanently too. `shift-enter` opens permanently either way; `shift` or `cmd` with a
click, and a double-click — on the explorer row *or on the tab itself* — open permanently. Only one
preview tab exists at a time — opening another preview replaces it and closes its panel. A preview
is promoted to permanent by its first edit, or by the explicit gesture that opens it permanently; a
single click that merely brings the tab to the front does not promote it. A preview tab is drawn in
italics with a faint background so it reads as tentative at a glance.

**Every open file in a project wraps together.** Whether the file editors soft-wrap long lines is a
project's preference, written into its view prefs, and one flip brings every
already-open buffer into line rather than waiting for a reopen. A buffer with no wrap preference
uses its editor's own default, which is to wrap.

**With no file open, the centre shows the brand, big and soft.** The page that is about to hold a
file says so without furniture — no button, no menu, just Ubiq's own mark at 200px and half opacity
on the window's ground. The mark is theme picked, the blue on a light palette and the white on a
dark one, exactly as the rail's mark is on its swatch.

**An empty page that already has words keeps them, and gains the mark behind them.** The agents
row with no columns, the documents centre with no document open and the teams graph with nothing
to draw each keep the sentence and the icon they had, with the mark painted underneath at 0.16
opacity — far fainter than the no-file page's half, because a watermark that competes with the
sentence in front of it has stopped being a background.

**The mark turns over, and nothing on screen asks it to.** The spin winds back a fraction of a turn,
runs two full turns and settles onto the mark it started from, swelling a tenth as it winds up and
back to its own size by the end; the whole thing takes 2.4 seconds. Only the ring moves — the cubes
in the middle stay put, which is what makes the turn read as the ring turning rather than the image
spinning. Two things start it, neither of them a control: a **triple click on the big cube** in the
middle of the mark, and an **idle spin** every seven to fifteen minutes for as long as the window is
open. Asking for a spin while one is running restarts the curve rather than being swallowed, and
extends the run rather than queueing a second one.

**Each open file is a panel, so its tab is the dock's.** There is no tab strip of the editor's own:
a file's tab belongs to the group it sits in, which is what lets a file be dragged beside another
into a row, a column or another group — two files side by side is a drag rather than a mode. Making
a file's tab the displayed one is what makes it the active file, and closing it is what closes the
tab — and the keyboard moves with it, so clicking a file in the explorer or on a tab puts the caret
in that file's buffer.

**A row of tabs scrolls when it runs out of room.** Too many tabs for the strip squeeze into a
scrollable band rather than shrinking the tabs past readability; the active tab scrolls into view,
and chevrons at the strip's ends nudge the band by a step — always present, so an overflowed strip
can be told to be pushable, and a no-op when the strip has nothing more to show. The scroll offsets
only the tabs, so the strip's chrome — the active tab's underline, the `+` that opens a terminal —
stays put.

**A tab says what it is looking at.** A file's change against a version-control base is a tab of its
own beside the file rather than something the file's tab switches into, so opening a comparison
never takes over what is being read or edited. The tab is named for the file with what it is looking
at after it, and what identifies it — in the saved arrangement and in what the project remembers —
is that pair rather than the path alone.

**A viewer is a pure function of bytes and a kind.** What draws a file is chosen once from its
extension when the tab opens, and nothing about it looks at a path again: `.md` gets the Markdown
view, `.mmd` and `.mermaid` the diagram, `.excalidraw` the scene, the image extensions the picture
itself, and everything else the highlighted buffer — which is the general case rather than a
fallback. A comparison against version control is not a viewer kind: it is what the tab is *looking
at*, so a diff opens beside the file rather than inside it. The extension's answer is a default and
not a verdict: the status bar's file-kind chip overrides it for as long as the tab is open, and
`AppState::set_viewer_kind` re-settles the layout when the new kind does not offer the one the tab
was in.

**A viewer with more than one thing to draw has a layout toggle, and it persists.** A strip above the
body offers the positions that viewer's kind names — `ViewerKind::layouts()` — and only those: the
buffer has nothing to toggle to and an image has no source, so neither draws a strip at all. Markdown offers Source, Split, Preview and Annotation —
the fourth is the annotated-document surface, and no other viewer has a document handle to point it
at (T-124); Mermaid offers Source, Preview and Split; Excalidraw and draw.io offer Editor and Preview only,
because their source is JSON or XML nobody edits by hand and Editor is where each format's own
component does that instead. Split
shows the file's own buffer, not a copy of it, so switching costs nothing and loses no undo history.
Which position is on screen belongs to the file rather than to the strip, and it is written into the
saved arrangement and into what the project remembers, so a document reopens in the layout it was
left in — a layout an older arrangement named that the file's viewer no longer offers is refused
rather than restored. A new markdown file opens in Preview or Source as the Editor setting says,
**unless the file is already annotated, in which case it opens in Preview whatever the setting
says** (T-124): a sidecar beside the file is what says so, read off the explorer's own tree
(`AppState::markdown_open()`), because asking the host would index the document and write that
sidecar as a side effect. A folder nobody has listed yet takes the setting;
Mermaid, Excalidraw and draw.io still open in Preview. Already-open tabs keep the layout they were
left in.

**A Markdown preview scrolls, and YAML frontmatter is a bar above it rather than part of it.** A
document that opens with a `---` block draws it as a collapsible bar at the head of the preview —
collapsed to its first few field names, expanded to the raw YAML — and the bar keeps its height
while the document below it takes the rest of the panel and scrolls inside it. Whether the bar is
open belongs to the file, like the layout toggle. A document without frontmatter is the same view
without the bar.

**The preview always scrolls through an external scroll handle, not `TextView`'s internal one.**
`ui/viewer/mod.rs`'s `markdown_preview` calls `markdown::render_scrollable` unconditionally, whether
or not the heading minimap is on — `TextView`'s own virtualised scroller clipped the last line, left
dead space below it, and drew its own scrollbar bound to the centred reading column rather than the
pane's edge. `render_linked_scrollable` draws `gpui_component::scroll::Scrollbar` as an
absolutely-positioned sibling of a pane-width scroll `div`, so the bar sits flush at the panel's own
edge regardless of where the reading column is centred.

**Split draws the same way, uncapped, and its two panes stay at the same fraction down the
document** (T-166). `ui/viewer/mod.rs`'s `markdown_split` gives the preview half `file.md_scroll`
through `markdown::render_split` rather than the plain `markdown::render` every other viewer
position calls — the same external-scroll-handle fix `markdown_preview` carries, so the split's
preview stops leaving dead space below its last line too. `render_split` also does not cap the
column at the width preset: a half-pane is already narrower than the full viewer, and pinning it to
the reading measure on top of that left it using less width than it had, not more, so it reflows to
whatever the pane actually offers. `markdown_split`'s own `sync_markdown_split_scroll` keeps the
buffer and the preview at the same **fraction** down the document, not a mapped line or block — the
two lay the same content out at different heights per block, so there is no pixel-honest way to
line up a byte offset in one against a byte offset in the other. Whichever side moved further since
the last frame is read as this frame's mover, from `OpenFile::md_split_scroll`'s memory of both
sides' fractions as of the last reconciliation, and its fraction is copied onto the other side.

**A Markdown preview reads at a measured width, and the user picks how wide and how dense.** The
text column is capped at a width preset — Readable (~75 characters, the default), Wide (~95) or
Full (the pane's own width) — computed from the body font's average character width rather than a
fixed pixel figure, so the column stays true at any zoom. The column centres in the pane when
there is room for it plus a margin on both sides, and fills the pane with that same margin as
padding when there is not, so a resize never leaves a thin, uneven strip on one side. Above it sits
one body line of top padding, and below it enough bottom padding that the last line scrolls clear
of the frame rather than sitting flush against it. A table or a fenced code block keeps the
paragraph's own left edge and content width rather than breaking out to the column's outer frame
(T-134) — the two read as part of the same prose flow — and scrolls horizontally within its own
frame (`typography()`'s `Overflow::Scroll` on both, over `node.rs`'s `render_scroll_table` for the
table) instead of spilling past the column when it is wider than the measure. Inline code draws in
a more contrasted chip-like background. Line height
follows the width
preset (tighter at Readable, looser at Full) and a separate density toggle — Comfortable, the
default, or Compact — trims it and the paragraph spacing further for a reader scanning a long
document. Both the width preset and the density are a window-wide choice, not written down per file
or per project — see `theme.rs`'s `MdWidth`/`MdDensity` and the note in `ui/viewer/markdown.rs` for
what the underlying renderer does and does not expose a knob for.
`_docs/inbox/markdown-improvement-proposal.md` is the proposal this behaviour implements (§3–§7,
§12); its §8.2's structure-and-label minimap rendering, §6.2 file-reference links and §6.3 italic-
face selection are not built by it.

**The width preset, the density, and the minimap's visibility and side, are one popover on the
Markdown header — `ui/viewer/md_options.rs`, `MenuId::MdOptions`.** The loose pills the width and
density toggle first drew as are folded into it; `kit::popover` over `kit::choice_pill` rows plus a
`kit::check_box`, the same anchored-panel shape the status bar's Size popover uses. Every control on
it persists through `UiSettings` on the same `Message::SetSettings { layer: Ui }` path the rest of
the layer takes — `md_width`, `md_density`, `md_minimap` and `md_minimap_side` are all fields there
now, closing a gap the width/density pills opened with: before this, the two were `AppState` fields
that moved the preview but were never written down, so a restart always came back on Readable and
Comfortable regardless of what the reader had picked. `md_minimap` is `plan_minimap` generalised —
the plan modal's thread strip and the standard viewer's own minimap (below) answer to the one
flag and the one side setting now, rather than the plan surface alone.

**The same popover carries a character-size slider and a four-way text-colour picker, per
document and in memory only** (T-188) — `OpenFile::md_reading`, a `MdReading { char_scale,
text_shade }` held on the tab itself rather than in `UiSettings`: closing the tab, or restarting,
drops it, unlike every other row on this panel. `char_scale` is a multiplier over
`theme::font(Family::Content, Role::Body)`, never an absolute size of its own, and it does reach
the document — `markdown.rs::render_linked_scrollable` multiplies the body size by it before
`typography` ever sees it. `text_shade` is one of four `theme::TextColors` tokens
(`faint`/`muted`/`primary`/`strong`, the last added by this card), each with a value in both
palettes — **and it does not reach the rendered prose**: the vendored `TextView`'s own style type
carries no foreground colour a caller can set per instance, only a window-wide default installed
once from the active theme, so the picker, the per-document state and the persistence all landed
but the body still draws in the theme's own colour regardless of which rectangle is picked
(`markdown.rs::typography`'s own doc comment states this beside the line-height and tracking gaps
it already reported). A **"Make default, system-wide"** button writes the tab's own pair into
`UiSettings.md_char_scale_default`/`md_text_shade_default`, through the same
`Message::SetSettings { layer: Ui }` path every other row on this popover already persists by — no
new wire message, because that layer is exactly the interface-owned, opaque-to-the-host blob this
needed. **There is no per-project counterpart.** `ubiq-proto` carries no message that scopes a
settings blob to one project, and this card stops at reporting the shape one would need —
`ui/viewer/md_options.rs`'s own module doc comment — rather than adding it.

**The standard viewer draws the same block-shaped minimap the plan surface does, when `md_minimap`
is on** — `ui/viewer/mod.rs`'s `markdown_preview`, over `markdown::structure_marks`. Before T-134
this strip drew a heading-only tick and the plan surface's own minimap drew one mark per source
line; both read as noise next to the reference minimap's handful of legible bars, so both now draw
the same block shapes through the same `ui::document::mark_style` palette — `structure_marks` walks
the source's own top-level blocks directly, since a standard preview is one `TextView` rather than a
block per heading and has no host block index to read the way the plan surface does. A block's
fraction down the strip is its own byte offset over the document's length, the same honest
approximation the plan surface's own minimap places every mark by — there is no per-heading layout
to measure here in the first place. Clicking or dragging the strip scrubs the tab's
own `OpenFile::md_scroll`, an external `ScrollHandle` the preview hands to
`markdown::render_scrollable` in place of the text view's internal one (which the component library
keeps private and gives no caller a way to move). The strip sits on whichever side
`md_minimap_side` names, `Left` by default; the plan modal's own minimap answers to the same
setting, landing between the document and the thread rail rather than past it when set to `Right`,
so the rail stays the outermost column.

**The navigator's headings and the minimap's shapes come from one cached parse** (T-144).
`markdown::heading_marks` and `markdown::structure_marks` are two projections of the same mdast,
and both are read from a render function — so each was running `markdown::to_mdast` over the whole
buffer on every frame, twice per frame together, for a document that had not changed since the last
one. That is not a rounding error: `to_mdast` at `ParseOptions::gfm` costs about 7.7ms on a 50KB
document and 38ms on a 150KB one **in release**, so a markdown tab with the minimap on could not
reach 60fps on a file of any size no matter what else it did. Both now take the tab's key and read
`markdown.rs`'s `walks` cache, which fingerprints the source (length plus a hash, the same cheap
stand-in the fence scan's `SCAN_CACHE` beside it uses) and redoes the parse only when the document
moved. **They stay two walks, not one**: they read the tree at different depths on purpose —
`heading_marks` recurses, so a heading inside a list or a quote is still a heading the navigator
lists, while `structure_marks` reads only the root's own children, because a block nested inside a
larger one is part of that block's shape as far as a minimap is concerned. What they wanted to
share was the parse, not the walk. The fence scan is a third walk and stays separate for a reason
of its own: it needs each fence's exact `Code::value` byte for byte, because that string is the key
the block renderer later looks its picture up by.

**Every one of those walks runs over the body, after the frontmatter split** (T-155, T-151). The
preview hands `TextView` the body and draws the frontmatter as its own collapsed bar, so a walk
over the raw source describes a document nobody is looking at: it put a phantom top-level entry in
the navigator and a phantom shape in the minimap for every document with frontmatter, and measured
every fraction against a length the rendered document does not have, so every mark landed short of
what it points at. `walks` calls `split_frontmatter` first, the same step the preview and the fence
scan take, and parses the body at `ubiq_proto::blocks::options` — the options the host splits a
document with, so the navigator and the host's block index cannot disagree about what a block is.

**A diagram is drawn in the interface, on a background thread.** A Mermaid document is just text;
the bus already carries a file's bytes, so nothing about a diagram crosses it. The window renders it
with a Mermaid implementation of its own, and it renders it **off the frame thread**: layout is
superlinear and a large graph has been measured at two seconds, which would be two seconds of dead
keystrokes in every pane. A diagram a frame found it needed is queued while the frame is built,
handed to the background executor once it is done, and lands as an update on the next frame after
that. Until then the panel says it is drawing, and the window stays live.

**A drawn diagram is cached twice: in the window, and in the project's workarea.** The key is the
source's hash, the palette, and a marker for the renderer's version — the palette because the
renderer bakes its colours into the markup, so light and dark are two pictures rather than one
recoloured. The workarea is a directory the host reserves for the interface and never reads inside,
and it arrives on every project message as an absolute path rather than being composed from the
config root, which is what makes a host on another machine a change of value rather than of code.
**The cache is disposable**: deleting the workarea costs re-renders and loses nothing.

**A diagram or a scene in a panel can be panned and zoomed.** It opens fitted to the panel, aspect
ratio preserved, with a margin. The wheel zooms about the pointer, a drag pans, a double-click or a
pinch-out to the floor restores the fit. The camera belongs to the tab, not the file, and is not
written down. A fence inside a Markdown document is not a panel: it draws inside `ui/viewer/mod.rs`'s
`diagram_frame` (`diagram.rs::draw`, `scene.rs::draw_static`), which scrolls it horizontally when it
is still wider than the reading column rather than letting it spill past the panel; the document is
what scrolls vertically.

**A fence scales down to the document's own reading measure before that, never up past its own
size** (T-185) — `diagram::scale_to_measure`, published once per document render as
`diagram::publish_measure` (the same hand-off `RESOLVED` already is, since a block renderer is
handed no `AppState` to read `md_width` from), and read back by both `diagram::draw` and
`scene::draw_static`. A picture already narrower than the measure is untouched; one wider shrinks
to it, aspect preserved. Publishing is unconditional on every render — `render_block`'s own single
blocks (the plan surface's unit) explicitly publish `None`, so a full document rendered earlier in
the same frame can never leave its measure behind for an unrelated block's fence to pick up — and
`render_split`'s right-hand pane (already narrower than the full viewer) also publishes `None`, so
a fence there still draws at its own size exactly as before this card.

**A Mermaid fence, and a standalone Markdown image, carry a corner zoom button** (T-185) — a small
`Maximize` icon, top-right, that raises `ui::viewer::zoom_modal::render` near-fullscreen with pan,
zoom (the same `state::viewport::Viewport` camera a panel already draws on, under its own key so it
does not fight the inline picture's) and a Copy-to-clipboard button
(`gpui::ClipboardItem::new_image`). `WorkbenchState::image_zoom` carries which picture, one at a
time on `state::overlay::Layer::ImageZoom`'s rung — above the plan surface, since a fence inside
its own rendered markdown can raise the same button. The button is built through `window.root`
rather than `cx.listener`, because a fence's block renderer is handed only a `Window` and an `App`.
A standalone image is a paragraph holding nothing else — `markdown::ImageBlock`, the same shape
`state::document::is_image_reference` already told the minimap apart by — intercepted through the
same `MarkdownExtensions::block_parser`/`block_renderer` hook a fence already used, so it can carry
the same measure cap; it resizes but **carries no zoom button**, because there is no decoded pixel
size in hand at parse time to seed the modal's camera with, unlike a diagram's own renderer output.
**An Excalidraw fence resizes but carries no zoom button either** — it is vector shapes painted
straight into the panel, not one picture a modal or a clipboard button could hand off whole; only a
Mermaid fence's `Arc<Image>` and a standalone image's `img(url)` are reachable today.

**An Excalidraw scene sits on Excalidraw's own white canvas.** A file that names a canvas colour
keeps it; a file that names none — `transparent`, an absent key — gets the format's default white
rather than the window's ground, so a diagram drawn on paper stays on paper in a dark window. The
scene is painted into a canvas that fills the panel; a canvas that only laid out to its content
would draw the whole scene into a few pixels at the top of the pane.

**An Excalidraw scene can be edited, in Excalidraw, and `Editor` is one of its two layout
positions.** Switching to it downloads the vendor bundle once and mounts the real component on the
file's text — inside the panel, in a child webview, on macOS and Windows; in a browser window over
the window's own loopback origin everywhere else, since the binding for an embedded browser has no
finished Unix path yet. Edits come back as the whole document and are written into the buffer the
tab already holds, so the dirty dot, `⌘S`, save-as and the version check are the ones that were
always there — there is no second save path, and pressing the chrome's own Save button or the
keyboard chord inside the webview both reach it. Preview keeps drawing the scene through the native
painter the whole time, which is why opening a `.excalidraw` file still costs no download and no
delay. With the bundle missing or unfetchable the position says why and nothing about reading the
file changes.

**A `.drawio` file opens the same way, on the mirrored draw.io webapp instead of a mounted
component.** Its Editor position holds no native painter: the webview frames the mirror's own
`index.html` rather than importing it as a module, and edits still land in the buffer exactly as
Excalidraw's do — same dirty dot, same `⌘S`, no second save path. Preview has nothing of its own to
draw from, so it shows the SVG the editor last exported, cached beside the Mermaid pictures in the
same workarea directory and surviving a restart; a document never opened in the editor reads "Open
the editor once to draw this." instead of a picture.

**A fenced diagram in a Markdown document goes through the same renderer.** A ```` ```mermaid ````
fence is drawn by the diagram viewer and a ```` ```excalidraw ```` one by the scene viewer — one
renderer per format, two call sites for each. A Mermaid fence resolves against the same cache the
panel uses, so a document with several fences fills in as each of them lands rather than waiting for
all of them. A picture is drawn at the size the renderer reads out of the SVG's `viewBox` — never
stretched past it — scaled down further to the reading measure when this document caps one, per
the zoom-modal paragraph above.

**The fence body is a cache key, so there is one parser and not two.** The document resolves a
fence's picture before the block renderer that draws it is reached, and the two halves meet in a map
keyed on the fence's own text — so both halves take that text from the same Markdown AST, through
`Fence::of`. A second, hand-rolled scanner stood on the publishing side and reconstructed each body
line by line; it ended every fence with a newline the AST does not carry, kept the indentation the
AST strips off a nested fence, and kept the `\r` of a CRLF line the AST drops. The key it published
therefore never matched the key the renderer looked up, and every fence drew the ellipsis that means
*not resolved yet* for as long as the document stayed open.

**`⌘N` opens a buffer with no file behind it, and `⌘S` asks it where to go.** The tab is titled
`untitled-1`, numbering up past whatever is already open, and it holds a real editable buffer rather
than a tab waiting on a read — there is nothing coming. Saving one raises the same single-field modal
the explorer's own gestures use, asking for a project-relative path; confirming it retitles the tab
in place — name, language, viewer — and writes the file as a creation, which the host refuses if
anything is already there — unless the user says to write over it, in the **Overwrite file** modal
the refusal raises. **The path may name folders that are not there yet**, and a creation makes them:
the field asks where in the project the buffer goes, so `shots/new/capture-1.png` is a file that has
never been anywhere rather than one that went away — which is what it used to be told it was, in the
**Not saved** modal, reading *no longer there* over a picture that had just been pasted
(`tech/transport-contract.md` owns the rule, and only a creation makes a folder). The retitle
happens on the confirmation rather than on the host's answer,
the same bet a click on an explorer row makes, so a refusal is reported on a tab that already carries
the name the user chose and the message reads correctly.

**With an image on the clipboard, `⌘N` asks which one is wanted.** The modal offers the image as an untitled picture and the text buffer the keystroke has always meant; Escape and every other dismissal take the text file, and text or nothing on the clipboard skips the question entirely. Pasting outside a text field asks nothing: the image opens as `capture-{n}.png`, numbered past whatever is already open and dirty from the start so closing it asks. The name carries its extension, which is what selects the image viewer.

**Every picture the image viewer can decode is an editable scene, capture or not.** A file whose
viewer is the image viewer grows an annotation scene over its bytes at load — the base PNG plus
elements above it — carrying the read's version into the scene, so `⌘S` writes the flatten back
over the file with the version discipline any other write carries. Editing takes the keyboard
through its panel rather than through a buffer, because the toolbar, the tools and the undo all
live at Workbench depth. The header's View/Edit toggle says which mode a tab is in: a capture
opens in Edit, a picture opened from the explorer opens in View, where the tool layer is gone and
the scene panel's own pan, zoom and fit apply as they do on every other scene panel. Bytes with no
decodable picture in them — an SVG, a format this build does not decode; the `image` crate is built
with the `png` feature only — stay read-only bytes with no toggle at all, and the same toolbar
handlers are no-ops on them rather than errors. An `.excalidraw` document is a different viewer
entirely and stays read-only here too.

**A picture's header strip carries the View/Edit toggle in place of the layout toggle, and in Edit
it grows the toolbar a capture has always had.** Eight tools — Select,
Crop, Rect, Ellipse, Arrow, Draw, Text and Copy — are icon buttons, seven drawn from the `annotate`
icon category and Copy from Lucide, naming themselves only as a tooltip; then the style controls and
the history: a colour picker for the stroke, offering the same six colours as its featured swatches
alongside the picker's own palettes, HSL sliders and hex field, a width button cycling three, a Fill
toggle that fills with the stroke colour, and Lucide's Undo, Redo and Delete. Changing a style
control with something selected restyles it as one undoable step as well as arming the next element.
Delete is the toolbar's button and has no keystroke. The tools sit
on a transparent layer over the picture, and a gesture the layer declines reaches the viewport
underneath, so the pan, zoom and fit every scene panel has keep working beside the tools rather than
behind a mode.

**The base image is never touched, and undo is elements rather than pixels.** Every tool appends,
moves or removes one element above the base; crop only narrows what is shown, measured against the
whole picture, so dragging wider brings back what an earlier crop took. The undo stack holds
element snapshots and a crop's two bounds, so a freehand stroke costs its points rather than a
second bitmap. `⌘Z` and `⌘⇧Z` reach it, bound at Workbench depth so a text buffer's own undo wins
while one holds the keyboard.

**Select and Copy are the two gestures over a picture rather than onto it.** Select on an element
picks it up and a drag moves it, committed on the release as one step; Select on empty space is
declined so the drag pans instead, and the selection stands. Copy drags a rectangle and writes that
region — flattened, annotations included — to the system clipboard as a PNG, which is how a piece of
a capture reaches somewhere else. Text opens the one-field modal at the click and annotates where
the click was; an empty string leaves the question up rather than annotating nothing.

**A save writes a flattened PNG and never the scene, and only when the tab is dirty.** A picture
opened from the explorer that nobody annotated declines the save rather than re-encoding the file
over itself for nothing. The flatten decodes the base, crops it to the scene's bounds, and paints
the annotations over it at scene scale — scene units are base pixels. Because the base is held
whole behind them, a covering rectangle is real redaction only in the flatten, which is why PNG is
the only thing an editable picture writes. The write itself is the untitled path unchanged for a
capture: `⌘S` raises the same single-field modal, and the flatten goes out over
`WriteProjectFile` as a creation, with the version discipline any other write carries once the
first one has answered.

**`⌘S` writes the active file back, and names the version it read.** A buffer that is whole but has
never been read from disk — the far side of a save-as the host refused — carries no version, so its
save is a creation rather than a refusal: that is what keeps a retargeted tab savable for the rest of
the session instead of bricked behind every later ⌘S sending nothing and saying nothing. A guest tab
carries the same version discipline as any other — the far side of a save-as never applies to one,
since it was always read from a real path. Every other way a save can decline — the file moved
under it, a truncated read, a diff, a binary, a buffer whose bytes have not arrived — is said out
loud rather than swallowed, in the **Not saved**
modal: it names the reason, says the edits are still in the tab, and its one button only dismisses
it. A version-less write that lands on a path already taken is the one refusal with a question in
it — the **Overwrite file** modal — and confirming it re-sends the same bytes asking the host to
write over what is there, the only place Ubiq offers to do that (`tech/transport-contract.md` owns
the message). Ubiq is not the only thing editing these files, and the agents in the panes are the
other one: for a version mismatch there is no merge, and resolving a conflict is the user's.

**A tab with nothing typed into it follows the file on disk; a dirty one is left alone.** A clean
tab whose file changed goes back to reading and comes back with what is there now, its cursor and
scroll put back where they were rather than reset to the top — so the reread is invisible, whether
the change is the app's own save echoing back through the file watcher or a real edit from outside.
A tab with unsaved edits is not touched at all — not reloaded, not marked, not asked about — and the
save it eventually attempts is still refused if the file moved under it, which is where the user
hears about the collision.

**A file that cannot be edited honestly is not offered for saving.** A read the host cut short at its
byte ceiling is readable and unsavable, because writing a prefix back would shorten the file. A file
whose bytes are not text says so instead of drawing them.

**A dirty tab asks before closing.** The first click on its × turns the tab into a question; only a
second one discards the edit. Bringing the tab forward again withdraws the question. The panel comes
back to its group to ask, because the dock takes a closed tab out before the window hears about it.

**A right-click on a tab raises a menu over the window — a file's, a terminal's or a chat's.** A
file's is Close, Close Others, Close Left, Close Right, Close All, Copy Full Path, Copy link, Open
in Finder, Save, Word Wrap and Pin (or Unpin) — the two *closes* and the surround closes anchoring
on the tab that was clicked, Copy Full Path copying the file's project path to the clipboard, Open
in Finder revealing it (or its folder) in the system's file manager, Save writing the file behind
one tab rather than only the active one, and a dirty tab in a bulk close still asked for rather than
silently closed. A terminal's or a chat's is Rename…, Hide, Close and Pin (or Unpin) — Rename raises
the same single-field prompt every other rename in the window uses, seeded with the tab's current
label. **A chat tab attached to a live agent renames the agent, not the tab**: the typed name goes
out as `Message::RenameConversation` (`../tech/transport-contract.md`) rather than into `tab_names`,
because the agents column, the bench and every other surface reading `WorkAgent::name` would
otherwise keep showing the old one. A terminal's rename, and a chat tab attached to nothing, still
go nowhere but `tab_names`: a pane's id dies with its process and an unattached chat's is reminted
every run, so a saved name there would only ever point at nothing. **Hide and Close are two
different endings.** Hide takes the tab down and leaves the thing behind it running — a terminal's
harness, a chat's conversation — reattachable from the new-pane menu's Detached group or by
pointing a chat tab at the conversation again. Close is the real end: a terminal's kills its harness
behind a confirm — "Panes and terminals" is where that lives — and a chat's raises the same Delete
confirm its own lifecycle menu does, or simply hides when the tab is attached to nothing — there is
no conversation there to delete.

**Pin means protected from the ×, and nothing else.** A pinned tab's × is withheld, and every bulk
close a file's menu offers skips a pinned file outright. On the menu itself a pinned file's Close
row is left off rather than drawn and disabled; a pinned terminal or chat keeps its Close row —
ending the harness or the conversation underneath is still the user's call regardless of pinning —
and loses only Hide, for the same reason a file loses Close. A pinned tab draws a small pin glyph before
its label, in the accent colour, and pinning changes nothing about what the tab does: it still
resizes, still loses focus, still updates. A pinned file is written down — the one tab identity that
survives a restart is a file's own tab key — and a pinned terminal or chat is not, for the same
reason a typed-over name is not: nothing a saved id would name is still there to restart onto.
**Pinned tabs are not moved to the front of the strip** — the component library's dock exposes no
tab-reorder call — so pinning changes whether a tab can close and nothing about where it sits;
`G216` names the gap.

The menu is the window's one open
menu, painted at the window root because the dock's skin cannot name `AppState`, and it is dismissed
by a click outside it or by escape.

## Contract

**A guest tab's read never touches the bus.** It is read with `std::fs` and never resolves through
`ProjectTree`, `ReadProjectFile` or a project id at all — see *Behaviour* above and `D54`. Its save
does cross the bus, as `WriteHostFile`: an absolute path and no project id, on the host browse
family the picker's own directory listing lives in — see `tech/transport-contract.md`.

**Files cross the bus too.** The explorer and the editor are projections of the host's answers, not
state of their own: `ProjectTree`, `ReadProjectFile` and `WriteProjectFile` going out, and
`ProjectTreeListing`, `ProjectFileContents`, `ProjectFileWritten` and `ProjectFileError` coming back.
Every one of them names a project and a path inside it, and each answers only the window that asked.
`ProjectFilesChanged` is the exception in both respects: the host sends it unasked, from its watch on
the folder the window has open, and the window answers it with `ProjectTree`, `ReadProjectFile` and
`RefreshProjectGit` of its own.
The blob behind what a project remembers grows the arrangement, the open files, the active one, the
expanded folders and the selected row, and the host neither parses nor validates any of it.

## Implementation

`state/explorer/` holds every piece of tree logic and no frame, which is what makes it testable in
`crates/ubiq/tests/explorer.rs`. `merge()` puts one directory's listing into the tree, matching
entries by name so that a folder re-listed keeps the children and the expanded flags below it, an
entry that has gone is dropped with its subtree, and a new one arrives shut and unlisted — which also
makes an unsolicited listing harmless — which is what the filesystem watch relies on.
`is_folder_listed()` is the question the watch's answer needs: whether the tree already holds that
folder listed, and so whether re-asking for it can change anything. `toggle()` answers whether flipping a folder open means the
host has to be asked. `expanded()` is what gets written down and `reopen()` is what reads it back,
opening the folders a blob named as each of their parents arrives. The order the host sorted a
listing in is kept rather than sorted again, so two windows on one project cannot disagree.
`rows()` is the two arrangements — tree and list — and `press()` is every key, so the tests walk
the cursor without a window. `unlisted_for_cache()` is the folders the background fill still cannot
see into, skipping the walk's skip set so a cache does not list `node_modules`. `drawn_rows()` is
what the panel paints: open folders when the field is empty, the last background hits when it is
not, never a walk of the cache on the frame. `ExplorerMenu` is
the right-click: `menu_entries()` is what it offers, and the four actions that wait on the host are
present and not ready. `ui/explorer.rs` draws through `ui/kit/files.rs`, the same chrome the picker
uses, and tints, badges and dots a row from `GitStatus`. `apply_git()` takes a `GitWorkingTree` and
projects each pair onto that enum; `merge()` re-paints so an expand is not unmarked until the next
refresh. A stale generation is discarded. An untracked or ignored directory in the map is inherited
by every child `paint_git()` walks, which is what makes expanding a new folder mark the files
inside it — except across a nested repository's boundary, where `paint_nodes` resets the inherited
status to `None` at the nested root, since that folder's children already carry their real status
in the merged map. `apply_git()` also keeps `repos: Vec<GitNested>` in a `git_repos` map keyed by
each nested repository's project-relative root, cleared by `clear_git()`; `ui/explorer.rs` reads it
to draw the branch label after a folder's name, and `state::git::head_label` — also what
`ui/status_bar.rs` calls for the project's own branch — turns a `GitHead` into that same string.

`state/editor.rs` names the component library, unlike its neighbours, because a file's buffer *is*
its state: `FileBody` is either `Loading`, the `Text` of a buffer with the bytes the host sent beside
it, the `Diff` the host computed, `Binary`, or a `Failed` read. Dirtiness is that comparison against
the host's bytes, cached off the buffer's own change event rather than recomputed per frame.
`FileLanguage::of()` picks a highlighter from the path's extension, and anything it does not
recognise opens as plain text, which is the general case rather than a fallback. `Subject` is what a
tab is looking at — the file, or a comparison made from it — and `tab_key()` and `from_tab_key()`
are what keep those two tabs rather than one: the key is the path for the file itself and the path
behind a prefix for a comparison, and it is what the saved arrangement and the view prefs name a tab
by.

`ui/editor.rs` is what one file draws and the two things its tab asks of it. `render_file()` is the
file panel's body, `label()` and `state_colour()` are what the dock's tab says and the colour of the
dot beside it, `highlighter_language()` is the one place our language enum meets the highlighter's,
and `render()` is what is left of the centre panel — in Git mode as much as in IDE mode, since both
put the same panel in the middle: `ui::mark::alone`, the big, faint brand mark on the no-file page.

`ui/mark.rs` is the mark itself, a module rather than a private helper on the screen that had it
first, because five screens draw it: `alone()` is the mark on an otherwise empty page, and
`backdrop()` wraps a page's existing empty state with the mark painted under it — `ui/agents/mod.rs`'s
"No columns" page, `ui::kb::centre`'s "No document open" page and `ui/teams/graph.rs`'s empty graph.
**Two layers, because one of them moves.** The ring is an `svg()` and the cubes are an `img()`, and
the split is forced: GPUI can only transform an `svg()`, and it draws one by rasterising the markup
to a single-colour alpha mask tinted with `text_color` — exactly what the ring is, and exactly what
the cubes are not, since each cube's three faces are three shades. So `assets/logo-ring.svg` is one
file for both themes with `theme::mark` deciding its colour, and `assets/logo-white-cubes.svg` and
`assets/logo-blue-cubes.svg` are a file per theme drawn in full colour. A rotation turns an element
about its own bounds' centre, so the ring's file carries `viewBox="-29.25 7.65 1024 1024"` — its box
recentred on the ring rather than on the mark — and `RING_DX`/`RING_DY` shift that box back over the
cubes, which sit in the mark's own `0 0 1024 1024` box. The triple click is taken over the `cube`
group's bounds rather than the whole 200px square.

`turns()` and `grow()` are the curves, over `SPIN`'s 2400ms: the wind-up eases to a stop so the
release is a release, and the two turns run on a back-out cubic whose overshoot is tuned down from
the usual ten percent to two — ten percent of two turns is most of another one, which is a second
spin and not an arrival — landing about seventeen degrees past the mark and walking back onto it.
`workbench.mark` is a `MarkState` of `spin`, `spinning` and `until`. `spinning` is what mounts the
animated element at all: GPUI plays a one-shot animation when the element carrying it first appears,
so an always-mounted spinning ring would spin every time the user opened an empty page — the still
ring is a different element, and the spinning one exists only for the length of a spin. `spin` is
part of the animated element's id, so a spin asked for mid-spin restarts the curve. `app/mark.rs`
holds the two mutators: `spin_mark` bumps the number, sets the deadline and starts one waiter per
run of spins — the waiter re-reads the deadline, so a spin asked for while it sleeps extends the run
instead of starting a second waiter — and `watch_mark_idle`, started once from `AppState::new` in
`app/boot.rs`, is the idle spin. The watch is deliberately blind: it does not ask whether the mark
is on screen, because a spin on a window showing something else costs one repaint and mounts
nothing, and the alternative is a second copy of every screen's idea of "empty". Its interval is
randomised from `std::collections::hash_map::RandomState`'s seed rather than by pulling in a
random-number crate.

There is no tab strip here — the dock's groups draw those. A body that is not a buffer goes to
`ui/viewer/`, whose `mod.rs` holds the layout toggle and the frame every viewer's body is drawn in
and dispatches on `ViewerKind`: `diff.rs`, `markdown.rs`, `diagram.rs`, `scene.rs` and `image.rs`.
The buffer's text is cloned out of its entity only in the branches that read it — the general case,
`Editor`, draws off the entity — and `markdown.rs` keeps its fence scan and body beside the tab
key, fingerprinted by the source's length and hash, so a frame that changed nothing rescans
nothing. The scan is a parse: `fences()` runs the text view's own Markdown parser over the same
body the view is handed, and its unit tests assert the fence text it publishes is byte for byte the
text the view's block parser sees.
A diagram or a scene in a panel is wrapped by `viewer/viewport.rs`, which is the hits and the
wheel; `state/viewport.rs` is the camera they share — fit, zoom about a point, pan, reset — and
is what `tests/viewport.rs` asserts, because none of it needs a frame. A fence still draws
through `diagram.rs` and `scene.rs` directly, at the picture's own size. None of them reaches a
path or a handle; the camera is keyed by the tab and lives on the window.

The capture is four modules over that machinery and one dependency line. `app/capture.rs` is the
platform half: `capture_offered()` is the `cfg`, the setting and `cx.is_screen_capture_supported()`
together, `shoot_window()` is the whole of taking the picture — it streams one frame off
`cx.screen_capture_sources()` and hands `Option<Vec<u8>>` to whatever function pointer it was
given, `None` where the platform, the setting or the decode said no — and `capture_window()` is
`shoot_window()` with `open_untitled_image()` as that callback, which is what the capture button
still does. The feedback balloon is `shoot_window()`'s other caller: `app/feedback.rs`'s
`open_feedback()` passes a callback that raises the modal from the picture instead, on
`state/feedback.rs`'s `FeedbackForm` and drawn by `ui/feedback.rs`. `decode_window_png()`,
`crop_rect()` and `crop_png()` are the crop and the encode — the frame becomes RGBA in
`frame_rgba()` on the capturer's own thread, from `zed_scap::frame::Frame` on
Windows and Linux and from a `CVImageBuffer` on macOS, which is why `gpui` and `gpui_platform` carry
the `screen-capture` feature in `crates/ubiq/Cargo.toml` and `core-video` and `zed-scap` sit behind
target `cfg`s there. The pixels land in `app/editor.rs`'s `open_untitled_image()`, and
`clipboard.rs`'s `next_capture_name()` numbers the tab. `state/image_edit.rs` is the buffer:
`ImageEdit::new()` builds the scene of `state/scene.rs` elements over the base bytes, `push()`,
`commit_shape()`, `commit_draw()`, `append_text()`, `commit_move()`, `restyle_selected()`,
`delete_selected()` and `set_crop()` each leave one `EditOp` for `undo()` and `redo()`,
`display_scene()` adds the drag in progress for the painter, and `flatten()`/`flatten_rect()` are
the only decode-and-re-encode in the interface — display stays GPUI's. `app/image_edit.rs` is the
toolbar's handlers, the three gestures and `set_image_editing()` — the View/Edit switch, which drops
the selection and any pending drag on the way into View — every handler a no-op through
`ensure_image_edit()` on a tab whose bytes never decoded into a scene. `ui/viewer/image_edit.rs`
draws the strip and, in Edit, the tool layer over it, and converts a window point to a scene point
through the stored panel and `scene::live_with_overlay()`. `FileBody::ImageEdit` is what
`state/editor.rs` holds it in, `set_image()` is what a read builds it with, carrying the read's
`FileVersion` in, and `editable_image()`, `savable()`, `touch_image()` and `saved()` are where a
non-text body meets the tab's own lifecycle.

`state/diagrams.rs` is the Mermaid renderer and its disk tier, and it is the only place in the
interface that names `merman`. `render()` is one source in and one picture out, sized by `view_box()`
off the SVG's own `viewBox`; `key()` is the content address — the source, the palette and the
renderer's version marker, which moves with the `=` pin in `Cargo.toml`; `cache_dir()` joins the
workarea the host sent with the interface's own subdirectory; `Disk` reads and atomically writes one
entry there, swallowing every IO failure as a cache miss; and `resolve()` is the whole of what runs
on the background executor. `AppState::diagram()` answers what the window holds and queues what it
does not, `drain_diagram_asks()` hands the queue to `cx.background_spawn` once the frame is built —
never from inside one — and `diagram_drawn()` takes each answer back by its key and notifies.

The same module holds a second, read-only tier under `key_for(EXPORTED, …)` for a format the
interface has no renderer for — draw.io's `.drawio` scenes: `resolve_exported()` only ever reads the
disk tier, answering "not exported yet" on a miss rather than rendering anything, and `keep_exported()`
is the one writer, filing the SVG a web panel exported and rejecting markup with no `viewBox`.
`AppState::exported_preview()`, `drain_exported_asks()` and `keep_exported()` in `app/web_panel.rs`
mirror `diagram()`/`drain_diagram_asks()`/`diagram_drawn()` on that tier, and `ui/viewer/diagram.rs`'s
`exported()` sits beside `render()` to draw it.

The file path through the two halves: `select_file()` opens a tab, queues its panel and sends
`ReadProjectFile`; `open_diff()` opens a tab on a comparison and sends `DiffProjectFile`;
`toggle_folder()` sends `ProjectTree` when a folder has never been listed; `fill_explorer_cache()`
sends the same message at `CACHE_DEPTH` for folders the background fill cannot see into yet, from
project open, not from a keystroke; `schedule_explorer_filter()` debounces the field and
`spawn_explorer_filter()` walks a snapshot on the background executor;
`save_active_file()` sends
`WriteProjectFile` with the version the read came with. Holding a project also sends `ProjectGit`
and `RefreshProjectGit { full: true }`, a successful save and a pane exit send the full refresh
again, and `ui/status_bar.rs` prints the overview's branch. `ProjectFilesChanged` is answered in the
same three currencies: `RefreshProjectGit { full: true }` when it reports the git directory, one
`ProjectTree` per deduplicated parent directory the tree already holds listed — the root plus
`ExplorerState::expanded()` when the batch is truncated and names no path — and one
`ReadProjectFile` per open tab that is neither dirty nor already loading, every open tab being a
candidate when the batch is truncated, put back to reading by `OpenFile::reload()` — after `set_restore()` has captured the
outgoing buffer's `selected_range()` and scroll offset, so the fresh `Entity<EditorState>` the
answering contents build is put back where the old one was read rather than opened at the top.
`crates/ubiq/tests/files_changed.rs` is what asserts each of those without a frame, cursor position
included. `activate_file()` and `closed_file_panel()`
are the other direction — the dock deciding which tab is in front and which has gone, which the
editor learns from it rather than the other way round. `activate_kb_doc()` and `closed_kb_panel()`
in `app/kb.rs` are the same pair for a `PanelKind::Kb`, moving the explorer's highlight and closing
the document's own tab respectively rather than anything in `state/editor.rs` — `wip/kb.md` has the
knowledge base's own half of the tab machinery. Contents cannot become a buffer where they
arrive, because a buffer needs a window and a message does not come with one, so they queue and
`attach_arrived_files()` drains them in `render` — the same device the dock's own edits and the
pending focus use, and the one `fill_task_form()` uses for the task panel's fields. `take_editor_focus()`
is where that pending focus lands: a tab whose viewer draws nothing for the layout on screen —
Markdown or Mermaid in `Preview`, or Excalidraw or draw.io in any layout, per
`ViewerKind::shows_buffer()` — has no buffer
to hand the keyboard to, so it goes to `workbench_focus` instead, a handle `track_focus`ed on the
workbench root beside the `Workbench` key context. Skipping that and leaving focus on the previous
tab's now-unmounted buffer is not merely stale: GPUI's key dispatch falls back to the window's own
root when the focused node was not painted this frame, and the whole `Workbench` context —
`close_active_editor()` included — is unreachable from there until some other tab takes focus. It exists for
that reason and no other: `set_value` needs a window and a message does not come with one, so a
selection change, a project switch and a `TaskChanged` for the open task each leave a flag for the
next frame to drain. Its guard is what stops it writing over what is being typed on every frame, and
it fills from the record the host confirmed rather than from what was typed — never while a field is
open. `install_key_bindings()` binds `⌘S`, `cmd-w`/`ctrl-w` (`close_active_editor()`, the keyboard
equivalent of the active tab's ×), `cmd-=` and `cmd-shift-=` (zoom in), `cmd--` (zoom out),
`ctrl--`, `ctrl-shift--`, `cmd-alt-k` and `cmd-k` for navigation, and `cmd-shift-o` (`OpenOutline` →
`AppState::open_outline`/`reveal_outline`, bound in both `Workbench` and `Input` contexts, beside
`OpenSearch` on `cmd-shift-f`), and `cmd-alt-y`/`cmd-alt-n` (`AllowPermission`/`RejectPermission` →
`AppState::allow_permission`/`reject_permission`, in both contexts for the same reason: a permission
prompt blocks the turn and the chat composer holds the keyboard while it is up), and `cmd-v`/`ctrl-v`
(`PasteClipboardImage` → the clipboard's image as an untitled picture, in `Workbench` only so a
field's own paste wins the tie — and a focused terminal wins it too, since `gpui-terminal` nulls
both paste chords in its own deeper `Terminal` context and the image goes to the harness instead),
and `cmd-1`..`cmd-9`/`ctrl-1`..`ctrl-9` (`ProjectSlot1..9`/`RailSlot1..9` →
`AppState::activate_project_slot`/`activate_rail_mode_slot`, eighteen actions and bindings for the
digit shortcuts) in the `Workbench` key context, then the file picker's, the navigator's and
the explorer's keys — each bound for the surface and for the field inside it, after the component
library's own so they win — and the binary calls it beside its own quit binding.

## Failure

| What happens | Result |
|---|---|
| The last editor tab is closed | The centre panel comes back where it was left and says no file is open, and the status bar reports no caret and no language |
| A saved arrangement names a file | It is rebuilt from the payload beside it, at the tab and the layout it carried. A file panel with no payload names no tab and is dropped, like a terminal |
| A saved arrangement's file panel names a tab the project no longer opens | The panel is hidden rather than drawn, so it keeps its place and comes back if the tab is opened again |
| A file panel is dragged to a border region | It is moved back to the centre on the same edit. The open files are the centre in IDE mode, so a file on a border would leave nothing behind it |
| A folder's listing cannot be read | The tab or the row says so; the rest of the tree is untouched. A missing or refused path is the interface's cue to ask the host to probe the project again |
| A file's bytes never arrive | Its tab says it is reading, and keeps saying so. Nothing else waits on it |
| A file's read fails | The tab shows the reason in the danger colour instead of a buffer |
| A file is too large for one read | What arrived is drawn and can be read; saving it is refused, because writing a prefix back would shorten the file |
| A file is not text | The tab says so rather than drawing bytes as characters |
| A file changed on disk since it was read | The save is refused, the file is left alone, and the tab and the status bar say so. Ubiq offers no merge |
| A save fails for any other reason | The same report, cleared by the next edit |
| The host cannot watch the project's folder | Logged, and the project has no watch: the tree and the git state go stale until something asks for them, which is every other path in this table unchanged |
| The project is not a repository | The explorer's rows stay unmarked and the status bar prints no branch. That is an ordinary answer, not an error |
| A dirty tab is closed | The panel returns to its group with the tab turned into a question, and takes a second click. Bringing it forward withdraws it |
| Contents arrive for a project the window no longer holds | Dropped. For one it holds but is not showing, they are put in their tab, which is there on the next switch |
| Contents arrive for a tab that has been closed | Dropped, so nothing reopens under the user |
| A remembered folder no longer exists | It is dropped from the restore rather than waited on; the rest of the tree opens |
| A remembered file no longer exists | Its tab opens and reports the failed read, so the loss is visible rather than silent |
| A diagram has been asked for and has not come back | The panel says it is drawing. The window keeps redrawing and keeps taking keystrokes; nothing waits on it |
| A diagram's source will not render | The renderer's own sentence is shown in the danger colour above the source, which is the only place it is any use. Nothing is written down, and the next ask renders again, because it may follow the edit that fixed it |
| A Markdown fence has not resolved yet | The block draws an ellipsis and fills in on the frame its picture lands. The rest of the document is drawn |
| The workarea cannot be read or written | Every failure is a cache miss, never an error: the diagram is rendered again and the picture is drawn. A half-written entry is a miss too, because it carries no usable size |
| The palette is switched with a diagram on screen | The picture is rendered again for the new palette. The two palettes are two entries, so switching back is a cache hit |
| A window has no project yet | A diagram still renders, with the memory tier alone. There is no workarea to write to until the catalogue has arrived |
| The capture switch is off, the platform decodes no frame, or the backend does not answer | The titlebar has no capture button and the keystroke does nothing. There is nothing to dismiss and nothing to report |
| The balloon is pressed where no capture is possible | The feedback modal opens all the same, with no screenshot to show or remove |
| A build has no feedback destination compiled in | The balloon is still offered; the modal opens and its Send stays dimmed, naming the reason under the button |
| An answer names a report the window is no longer holding | Discarded — the modal was closed, or a second report started, since it was sent |
| The platform refuses the capture, offers no display, or sends a frame this build does not read | The reason is logged and no tab opens. A capture that cannot land is not a half-drawn picture |
| The window is minimised or wholly off its display | The capture is refused rather than saved as whatever the compositor had. A window hanging off an edge is clamped to the frame and captured |
| An untitled picture's bytes carry nothing decodable | The tab stays read-only bytes: it draws what it can and still saves, it just never annotates |
| A tool gesture lands before the panel is measured | The gesture is declined. The next one, with a camera to convert through, lands |
| A shape or a stroke is dragged to nothing | Nothing is committed and nothing goes on the undo stack. A copy region dragged to nothing writes nothing to the clipboard |
| A picture does not flatten | The save is not attempted and the tab says the picture did not flatten. Nothing is written, so nothing is half-written |

## Related docs

- [`workbench.md`](./workbench.md) — the rail, the dock, the panels and the status bar this mode is drawn inside
- [`workbench-git.md`](./workbench-git.md) — the Git screen, which reaches the same diff renderer
- [`../tech/components.md`](../tech/components.md) — the viewer, the diff renderer and the file picker as components
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and the component conventions
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the file family on the wire
- [`../backlog.md`](../backlog.md) — what this mode still lacks
