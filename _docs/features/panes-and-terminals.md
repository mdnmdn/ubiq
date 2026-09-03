---
id: feat-panes
title: Panes and terminals
kind: feature
status: draft
summary: What a pane shows, how exactly one of them holds focus, how a resize reaches the harness, and how a pane is moved around the window's dock.
read_when: you are changing where a pane sits, pane focus, resize, pane chrome, or how terminal bytes reach the screen
updated: 2026-09-03
verified: 2026-09-03
code_anchors: [crates/ubiq/src/app.rs, crates/ubiq-proto/src/bus.rs, crates/ubiq/src/ui/terminal.rs, crates/ubiq/src/state/dock.rs, crates/ubiq/src/ui/dock/mod.rs, crates/ubiq/src/ui/dock/skin.rs, crates/ubiq/src/ui/new_pane_menu.rs, crates/ubiq-host/src/coordinator.rs, crates/ubiq-host/src/pty/mod.rs, crates/ubiq-host/src/shells.rs, vendor/gpui-terminal/src/view.rs, vendor/gpui-terminal/src/render.rs, vendor/gpui-terminal/src/input.rs, vendor/gpui-terminal/src/mouse.rs, vendor/gpui-terminal/src/clipboard.rs]
depends_on: [tech-transport]
review_cycle: monthly
---

# Panes and terminals

## Purpose

A pane is where the user watches an agent work. It is a real terminal emulator with a tab naming it:
which agent, and whether its harness is still running. Panes are what the user arranges, what focus
selects between, and what a resize has to be propagated through — and getting the last of those
wrong is the classic way a terminal multiplexer corrupts a screen.

## Behaviour

**One pane, one workspace, one pseudo-terminal.** The mapping is total in both directions, and the
pane ID is what ties them together across every message.

**A pane belongs to a project, and starts in its folder.** There is no pane without one: the
project's folder is the only thing a pane's working directory can be, so a window holding no project
draws no panes and spawns none. The interface never names the folder — it sends the project's id, and
the host resolves the path from the record. An optional `rel_path` starts the harness in a
subdirectory of the project instead of at its root.

**A spawn into a folder that is not there is refused before a pseudo-terminal exists.** A project
whose folder is missing, is not a directory or cannot be read answers an error against the project,
not against a pane — so nothing empty is left on screen — and the picker's row is marked from the
probe that refusal just made.

**Entering a project opens no pane of its own accord.** The project's files and the arrangement it
was left in come back, and nothing is asked to run: the first pane appears when the user asks for
one, so a window that opens on a project starts with no harness running.

**The pane region opens empty and put away, and opening it starts a pane.** A fresh window's bottom
region holds nothing — the console is not installed in it and there is no pane until one is asked
for — so what it gives the window is its size and its tab strip. Bringing it on screen from the
titlebar's switch starts a pane in it, the platform's default shell, because a region that exists to
hold panes and opens onto a bar of nothing has not answered what the switch was asked for.

**A pane's tab is its program and a number.** `zsh 1`, `zsh 2`, `fish 1` — each program numbered in
its own sequence, per project, from the lowest number no pane of that program is using. Closing
`zsh 2` gives that name back to the next one rather than counting upwards for ever.

**The `+` opens the platform's default shell; the chevron beside it says what else can run
here.** A bare click starts `$SHELL` — `COMSPEC` on Windows — which is what a terminal application
starting no particular program means. The chevron opens a menu of every shell the machine actually
has, the default one marked, and picking one starts a pane running that shell instead. The list is a
fixed set of known shells the host checked for — `zsh`, `bash`, `fish` and `sh`, or PowerShell and
the command processor on Windows — not a launcher for anything on disk, and a shell that is not
installed is not offered. Above the shells, and separated from them, the menu offers every agent
harness the harness library knows; picking one starts a composed agent rather than a program, which
[`sessions-and-workspaces.md`](./sessions-and-workspaces.md) describes. A harness whose binary is
not on this machine is offered as an unavailable row rather than left out, because the row is how a
user learns it could be there. Below a separator — everything above it starts something — one row puts
the console on screen, which is the one thing on that menu that is not a pane. The `+` needs a
project and is not drawn without one; the chevron is drawn either way, and with no project the
console is the only row it offers, because a shell that cannot be started is not worth a row.

**A shell pane is a login shell.** It is started the way the user's own terminal starts one, so
`.zprofile`, `.zlogin` and `.profile` run and a pane's `PATH` is the `PATH` the user has everywhere
else. Without it a tool that is genuinely installed reports as `command not found` in a pane while
working in Terminal.app, because Ubiq launched from Finder inherits a `PATH` that nothing has set up
yet. Only a shell started with no arguments is treated this way: a harness, or a shell handed a
command to run, is started as itself.

**Which shells exist is the host's answer, asked for and never assumed.** The interface may not look
on disk, so it asks — as it attaches, and again every time the menu opens, which is what makes a
shell installed since the window opened available without a restart. It asks for the agent types the
same way, and for the same reason.

**A pane's environment is whatever started it.** A shell inherits Ubiq's own, plus the `TERM` and
`COLORTERM` every pane is given. A composed agent adds the variables that point it at the throwaway
configuration it was provisioned into — without them the harness reads the user's real
configuration instead — and a confined one replaces the environment entirely, because its policy
computed the whole of it and inheriting Ubiq's would put back what the sandbox took out.

**A project's panes stay alive while another project is on screen.** A window can hold several
projects, and switching between them swaps which project's panes are drawn; the ones behind keep
running and keep their scrollback, because nothing is killed under the user. Their panels are hidden
rather than removed, so they come back where they were left. A project *leaving* the window is
different: its panes are closed with it, since a pane's working directory is that project's and no
other window can adopt an emulator.

**A pane exists because the coordinator says it does.** Asking for one is a request; the panel and
its emulator are drawn on the answer. A harness that fails to start produces an error against a pane
that was never drawn, so nothing empty is left on screen for the user to close.

**Closing a pane kills its harness.** The child is signalled and reaped, and the panel and its
emulator go with it. Closing a pane's tab is what closes the pane, and it is the only thing that
kills a harness — an agent left alone keeps working whether or not anyone is looking at its pane. A
tab on the agents screen is a different tab: its close benches the agent and the harness keeps
running. A
panel displaced by a whole arrangement being installed over it has not been closed and its harness
is untouched.

**The keyboard belongs to the focused panel.** Exactly one panel in the dock holds it. When that
panel is a terminal, keystrokes go to that pane's emulator — a terminal panel's focus handle *is*
its emulator's, so there is nothing in between — and from there to the harness, except the
intercept set below. When it is anything else — the console, the explorer, the chat, the centre —
no pane holds the keyboard at all, so a terminal nobody can see cannot be typed into. Unfocused
panes keep drawing — an agent working in the background stays visible — but take no input.

**Bytes are forwarded, never interpreted, except a closed intercept set.** Output from the harness
goes straight into the pane's emulator; keystrokes from the focused pane go straight to the harness
unless they are one of: platform copy (`Cmd+C` on Mac, `Ctrl+Shift+C` elsewhere), platform paste
(`Cmd+V` / `Ctrl+Shift+V`), or a defocus chord (`Shift+Escape`, `Ctrl+Escape`, `Cmd+Escape`).
`Ctrl+C` is SIGINT on every platform. Bare Escape is `\x1b` to the harness. Copy with no selection
is consumed and does nothing; paste wraps the clipboard in bracketed paste. Tab and Shift+Tab reach
the harness: the emulator's `Terminal` key context suppresses the window's focus-cycle bindings.
Special keys, Ctrl and Alt chords, mouse reporting and the alternate screen are otherwise the
emulator's. Enter is `\r`; Shift+Enter is `\x1b\r` — the sequence Claude Code's own
`/terminal-setup` binds Shift+Enter to — so a harness can tell "newline" from "submit" without
kitty-protocol negotiation, which this emulator does not track.

**The pointer is the emulator's when the harness has asked for it.** A harness that enables SGR
mouse reporting owns clicks, drags and the wheel. When reporting is off, a click-drag selects
text (double-click a word, triple-click a line), release copies the selection, and a click with no
drag on an OSC 8 or `http(s)://` URL opens it. The wheel in the alternate screen becomes arrows; in
the normal screen it moves the pane through scrollback. An OS file drop always pastes quoted
absolute paths as bracketed paste, including while mouse reporting is on.

**A defocus chord releases the keyboard without sending `Focus`.** The pane keeps drawing and its
tab stays; `blur_panes()` clears pending focus and the emulator's focus handle is blurred. Clicking
the pane gives it the keyboard again. The host still records the last focused pane.

**A resize is not complete until the harness knows.** Changing a pane's geometry means computing the
new size in character cells, telling the coordinator, setting the pseudo-terminal's size, and
letting the kernel signal the harness so it redraws. A pane that resizes visually while its harness
still believes the old dimensions is the failure this rule exists to prevent.

**Geometry is measured in cells, not pixels.** The conversion happens once, in the UI, where the
font metrics are known. Everything downstream speaks columns and rows.

**A pane's text scales with its project.** The terminal font size is the active project's — the same
value the file editor and the explorer tree are drawn at — and an emulator already open is dressed to
match when it changes rather than waiting for a restart. A zoom that only reached the next pane to
open would not be a zoom, so `AppState::set_ui_font_size()` reconfigures every emulator it holds.

**A pane starts at 80×24 and is told the truth a frame later.** The harness has to be started before
the emulator has been given any bounds to measure, so it begins at the conventional size and is
resized as soon as the first measurement exists. A harness that starts at the wrong size and is
immediately resized draws correctly; one that never learns its size does not.

**An exited harness closes its pane.** Typing `exit` or sending EOF (Ctrl+D) ends the child, the
coordinator reports `PaneExited`, and the tab goes with it — the same close path as the tab's ×.
Closing a tab is still what kills a harness that has not already ended.

**A pane's chrome is its tab.** The title says which agent, and the dot beside it says whether the
harness is still running. The pane itself carries that same state on the coloured left edge every
surface in Ubiq is identified by. Everything else on screen belongs to the harness: Ubiq gives the
emulator the background, foreground and cursor colours its surface is drawn in, and leaves the
sixteen ANSI colours to the emulator's own defaults, because those are the colours the harness is
choosing between.

**A pane is a panel, and the user arranges it.** Panes sit in the window's dock like every other
area: dragged into a tab beside another pane, into a row or a column beside one, into the centre
region or the bottom. Splitting and tabbing are that one gesture, and Ubiq fixes no arrangement of
its own. Where a pane may go is the placement rule in [`workbench.md`](./workbench.md) — the centre
or the bottom, never a border.

**A move does not restart the harness.** A dragged tab re-parents a panel by id, so the emulator,
its byte stream and the pane ID all survive the move: the same view, on the same bytes, under the
same child. **A pane ID never changes**, which is why every message about the pane goes on working
across a drag.

**A move is a resize.** A pane laid out in a new rectangle measures itself and posts a
`TerminalResize`, so the harness learns its new size the way it learns about any other geometry
change. Nothing arranges that specially — it is the same measurement a divider drag makes.

**A pane that is not the displayed tab of its group is not laid out.** It is not measured and not
resized, and it keeps the geometry its harness was last told — which is the truth about it, because
the two agree. Its output goes on arriving and its emulator goes on consuming it, so a pane's stream
never stalls on whether anyone is looking at it.

**Closing the focused pane moves focus to another pane** rather than leaving the window with none.

## Contract

The pane family of the transport contract: `TerminalOutput`, `TerminalInput`, `TerminalResize`,
`Focus`, `PaneExited` and `PaneError`. A pane's own lifecycle uses two of the session family,
`SpawnWorkspace` with its `WorkspaceSpawned` answer, and `CloseWorkspace`. `SpawnWorkspace` carries a
`project_id` that is not optional and an optional `rel_path`, and it can answer `ProjectError`
instead — a refusal names the project, because there is no pane yet to name. Which shell a pane runs
is `SpawnWorkspace`'s existing `agent_type`, and the menu's own rows come from `ListShells` and its
`ShellList` answer, whose `ShellInfo` carries a label, a program and whether it is the default. Variant names, payload
fields, the byte-sequence rule and the per-pane ordering guarantee are owned by
[`../tech/transport-contract.md`](../tech/transport-contract.md).

Output is chunked as it was read, not as lines. The emulator reassembles partial escape sequences;
the pane does no buffering of its own beyond what backpressure requires.

## Implementation

**A pane is one panel in the window's dock.** `PanelKind::Terminal` in
`crates/ubiq/src/state/dock.rs` carries the pane ID, and `crates/ubiq/src/ui/dock/mod.rs` holds the
panel itself. Three of its answers are this document's: its focus handle is the emulator's, so
giving the panel the keyboard puts keystrokes on the harness with nothing in between; `set_active()`
calls `focus_pane()` when the displayed tab is a terminal and `blur_panes()` when it is not, which
is what makes "no pane holds the keyboard unless a terminal is focused" true by construction; and
`on_removed()` waits a turn before it calls `close_pane()`, guarded by `on_added_to()`, because the
library reports a closed tab and a displaced panel the same way and only one of them kills a
harness. Which regions a terminal may sit in, and the tab, its dot and its close, belong to
[`workbench.md`](./workbench.md).

**The panel's body is the emulator.** `crates/ubiq/src/ui/terminal.rs` draws it: `pane()` takes a
pane ID and draws that pane's `TerminalView`, or the line a panel whose emulator has gone shows, and
`config()` is the `TerminalConfig` every emulator is built with — taking the font size alongside the
geometry, so a pane's text follows its project's own (`AppState::set_ui_font_size()` rebuilds it
from a fresh `config()` when the size changes). The pane is named rather than found
through focus, because every pane has a panel of its own and which of them the user is typing into
is the dock's answer. The view comes from the
vendored `gpui-terminal`, which parses the bytes with `alacritty_terminal` and draws the screen. It
holds no path, no process handle and no descriptor: it is constructed from a `Read` and a `Write`
that are ends of the bus, which is what keeps the UI honest about a pane being an ID plus a byte
stream. The palette it is given is
[`../tech/ui-and-design.md`](../tech/ui-and-design.md)'s, including the selection and link tokens.

Copy, paste, OSC 52, mouse selection, hyperlinks and file drops are the emulator's: `TerminalView`
intercepts the copy and paste shortcuts, writes bracketed paste and OSC 52 replies to the pane's
`Write`, drives alacritty's `Term::selection`, and paints selection and link underlines in
`vendor/gpui-terminal/src/render.rs`. It installs the `Terminal` key context, and
`install_key_bindings()` nulls Tab, Shift+Tab and the window copy chord in that context so they are
not stolen by the shell's focus cycle. Ubiq only adds the defocus chord: `open_pane()` sets
`with_key_handler` so Shift/Ctrl/Cmd+Escape calls `window.blur` and `blur_panes()`, and does not
send `Focus`. A `PaneExited` is `close_pane()`.

**The `+` that opens a pane sits at the right end of the tab strip.** Opening a terminal is chrome
rather than a group's own action: `crates/ubiq/src/ui/dock/skin.rs` draws the control in
`render_tab_bar` on any strip whose group holds a terminal or the console — the `NewPane` closure
`AppState::for_project` hands the skin — and a new pane's panel joins that group. It is drawn only
with a project open, because a pane runs in a project's folder. `NewPane` carries two closures: the
click, which is `spawn_pane(None, ..)`, and the chevron, which hands `AppState` the point the click
landed on and nothing else. `crates/ubiq/src/ui/new_pane_menu.rs` paints the menu over the window,
for the reason the file tab's menu is painted there — the skin does not name `AppState`, so it
cannot draw a menu with state in it. **The rows themselves are `WorkbenchState::new_pane_rows()`**,
which both the drawing and the pick read: a menu matched by position cannot have two lists.
`pick_new_pane_menu()` maps a row back — a shell is `spawn_pane(Some(program), ..)`, the separator
is a row and does nothing, and the console is `reveal_console()`, which is `dock::reveal()`: a
panel already in the tree has its region brought back and its tab brought forward, and one that is
not is added to its home region first. `AppState::toggle_region()` is where opening an empty pane
region starts a pane, and `pane_title()` is where a tab gets its number.

**`crates/ubiq-host/src/shells.rs` is the only place that knows what a shell is.** `available()`
checks a fixed candidate list against `PATH`, the user's login shell's own `PATH` and the usual
homes, and always includes `default_program()`, whatever it is. The other two lookups are there
because the `PATH` Ubiq itself was launched with is exactly the one that cannot be trusted: a
harness installed under the user's home is named by neither the thin environment a desktop launcher
hands over nor a fixed list of system directories. The login shell is asked once per process, with
`-lic`, because the login and interactive files are where a toolchain installer writes its
directory and a non-interactive shell never reads them.
`pty::spawn` asks the same module whether the program it was handed is a shell, and `command_for()` builds a login shell when it is: `portable-pty` prefixes argv0
with `-` only for a builder made with `new_default_prog`, which takes no program name and reads the
shell out of `SHELL`, so that is where the chosen shell is handed to it. The coordinator answers
`ListShells` straight from `available()`, and `ListAgentTypes` from `agent::Agents::types()`.

**`pty::spawn` is handed a `Program`, not a program name.** It carries the argv and the environment
a pane starts from — variables to set, variables to drop, and whether to start from an empty one at
all — because a composed agent brings its own and a confined one brings all of it. `Program::plain`
is the shell case: argv and nothing else. `crate::shells::locate` is shared with the agent registry,
so which `PATH` a program is looked up on is answered in one place.

**`crates/ubiq-proto/src/bus.rs` is the seam.** `hub()` opens the switchboard the one host answers
through, and `Hub::connect()` gives a window its own `Client` on it.
`pane_output()` makes one pane's byte stream: the sender goes to the window's router, and the
matching `PaneOutput` is the blocking `Read` the emulator consumes on a thread of its own — dropping
the sender is how it learns the stream is over. `PaneInput` is the `Write`, and a keystroke written
into it leaves as `TerminalInput` for that pane ID.

A pane belongs to the window that spawned it: the host records the owner before it answers, routes
everything that pane emits back to that window alone, and refuses a message about it from any
other. When a window goes, the host reaps the pseudo-terminals it owned — nothing else drops now
that the host outlives every window.

**`AppState` in `crates/ubiq/src/app.rs` owns one `OpenProject` per project the window holds**, and
each of those owns that project's panes and which of them is focused. The emulators do not move with
them: `terminals` stays one flat map from pane ID to emulator and output sender, because an emulator
does not care which list draws it. The dock and one panel per pane are the window's. Two tasks it
starts in `for_project()` do the rest:
a router draining the bus into `receive()`, and one carrying a measured geometry into
`resize_pane()`. `resize_pane()` searches every project the window holds rather than the one on
screen, because a pane nobody is looking at is still measured when it is laid out. Every mutator
ends by requesting a redraw — one that skips it is a pane that stops updating.

The paths through the two halves, in call order:

| What the user does | ui → state → orchestrator → pty |
|---|---|
| Opens a pane | `spawn_pane()` sends `SpawnWorkspace`, or does nothing when the window holds no project; the coordinator looks the record up, probes its folder, resolves the working directory, then `pty::spawn` opens a pseudo-terminal and starts the child, and the answer `WorkspaceSpawned` reaches `open_pane()`, which routes on its `project_id`, builds the emulator and queues a `PanelEdit::Open`; `settle_panels()` puts the panel in the region terminals live in on the next frame, because a panel reaches the dock through a window and a message does not come with one |
| Types | the emulator writes into `PaneInput`, which posts `TerminalInput`; the coordinator finds the pane's `Pty` and writes to the pseudo-terminal |
| Watches output | `Pty::forward_output` puts a reader thread on the pseudo-terminal, sending `TerminalOutput` in fixed chunks; `receive()` hands the bytes to the pane's output sender, and the emulator reads them |
| Resizes | the emulator measures its own bounds and its resize callback sends `TerminalResize`; `Pty::resize` sets the size and the kernel signals the harness |
| Brings a pane's tab forward | the dock displays the panel and gives it the keyboard, which for a terminal is its emulator's own handle; `set_active()` calls `focus_pane()`, which sends `Focus` on the transition and no other |
| Drags a pane somewhere else | the dock re-parents the panel by id, leaving the emulator, its stream and the pane ID alone; the panel is laid out in its new rectangle, the emulator measures it, and the resize callback sends `TerminalResize` — the move and the resize are one path |
| Closes a pane | the tab's × takes the panel out of the dock; `on_removed()` defers a turn so a displaced panel is not mistaken for a closed one, then `close_pane()` sends `CloseWorkspace` and drops the emulator; the coordinator kills the child, and the thread `pty::reap` left waiting on it collects the exit |
| The harness exits | `PaneExited` reaches `close_pane()`, which queues the same panel close and sends `CloseWorkspace` so the host drops the pseudo-terminal |

`crates/ubiq-host/src/coordinator.rs` holds one `Pty` per pane ID and nothing about layout or colour;
`crates/ubiq-host/src/pty/` is the only place a descriptor or a process lives. Its reader thread and the
bus's channels are unbounded, because a UI that has fallen behind must never stall the reader — a
stalled reader stalls the harness.

## Failure

| What happens | Result |
|---|---|
| The UI cannot keep up with output | The pane's queue grows; nothing is dropped and the coordinator's reader is never blocked |
| Output is not valid UTF-8 at a chunk boundary | Passed through as bytes; the emulator handles the split sequence |
| A resize arrives for an unknown pane | Ignored; a pane that has gone is not an error the user needs |
| A pane is dragged while its harness is writing | Nothing is interrupted. The panel is re-parented by id, so the emulator, the stream and the pane ID are the same on the other side; the new rectangle is measured and the harness is told |
| A pane becomes a background tab | It is not laid out and not resized, and keeps the geometry its harness was told. Its output goes on arriving and its emulator goes on consuming it |
| A panel is displaced by an arrangement being installed over it | Its pane is untouched. Only a closed tab closes a pane |
| The harness exits | `PaneExited`; `close_pane()` takes the tab out of the dock. Focus moves to another pane, or to none if it was the last |
| The harness exits while its pane is a background tab | The same close: the tab leaves that project's dock, and the pane the user is typing into is untouched |
| The harness cannot be started | `PaneError` against a pane ID the UI never drew; no tab appears |
| A spawn is asked for with no project open | Nothing is sent; there is no folder to start a harness in |
| The shell list has not been answered yet | The menu offers the console row alone, and no separator. The list is asked for again on every open, so the next one has it |
| The pane region is opened with no project | Nothing is started; the region opens empty, and the chevron's menu still reaches the console |
| A shell is uninstalled between the list and the pick | The spawn fails the way any unstartable program does: `PaneError` against a pane the UI never drew |
| A spawn names a project whose folder has gone | `ProjectError`, and the picker's row is marked. No pseudo-terminal is opened and no tab appears |
| A spawn names a `rel_path` that escapes the project | Refused with the same `ProjectError`, before anything is opened |
| A pane is announced for a project the window no longer holds | The window closes it again rather than draw it, so no harness is left running with nothing on screen |
| A project leaves the window | Its panes are closed and their harnesses killed; the panes of the projects that stayed are untouched |
| The focused pane closes | Focus moves to another pane, or to none if it was the last |

## Related docs

- [`sessions-and-workspaces.md`](./sessions-and-workspaces.md) — what a pane is a view of
- [`workbench.md`](./workbench.md) — the dock a pane's panel sits in, where it may sit, and the agents screen's tab, whose close benches an agent instead
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the message set, in full
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and the chrome conventions
- [`../tech/architecture.md`](../tech/architecture.md) — why the UI holds no pseudo-terminal

## Next steps

- Move between panes, and close one, from the keyboard.
- Scrollback search within a pane.
- Give a subagent's pane visible parentage, so a spawned agent reads as belonging to its parent.
