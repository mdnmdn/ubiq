---
id: feat-panes
title: Panes and terminals
kind: feature
status: draft
summary: What a pane shows, how exactly one of them holds focus, how a resize reaches the harness, and the layout modes panes are arranged in.
read_when: you are changing pane layout, focus, resize, pane chrome, or how terminal bytes reach the screen
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [crates/ubiq/src/app.rs, crates/ubiq/src/bus.rs, crates/ubiq/src/ui/terminal.rs, crates/ubiq/src/orchestrator.rs, crates/ubiq/src/pty/mod.rs]
depends_on: [tech-transport]
review_cycle: monthly
---

# Panes and terminals

## Purpose

A pane is where the user watches an agent work. It is a real terminal emulator with chrome around
it: which agent, in which state, in which folder. Panes are what the window arranges, what focus
selects between, and what a resize has to be propagated through — and getting the last of those
wrong is the classic way a terminal multiplexer corrupts a screen.

## Behaviour

**One pane, one workspace, one pseudo-terminal.** The mapping is total in both directions, and the
pane ID is what ties them together across every message.

**A pane exists because the coordinator says it does.** Asking for one is a request; the tab and its
emulator are drawn on the answer. A harness that fails to start produces an error against a pane
that was never drawn, so nothing empty is left on screen for the user to close.

**Closing a pane kills its harness.** The child is signalled and reaped, and the tab and its
emulator go with it. Closing is the only thing that kills a harness — an agent left alone keeps
working whether or not anyone is looking at its pane.

**At most one pane holds focus.** Keystrokes go to that pane's harness and nowhere else, and while
the dock draws its one non-pane tab — the log console — no pane holds the keyboard at all, so a
terminal nobody can see cannot be typed into. Unfocused panes keep drawing — an agent working in the background stays visible — but take no input. Focus is
shown on the pane's border, and that signal is not shared with any other meaning, because it has to
be readable at a glance across a window of panes.

**Bytes are forwarded, never interpreted.** Output from the harness goes straight into the pane's
emulator; keystrokes from the focused pane go straight to the harness. Ubiq forms no opinion about
either. That covers what would otherwise be a long list of special cases: arrow keys, Ctrl and Alt
chords, bracketed paste, mouse reporting, and the alternate screen.

**A resize is not complete until the harness knows.** Changing a pane's geometry means computing the
new size in character cells, telling the coordinator, setting the pseudo-terminal's size, and
letting the kernel signal the harness so it redraws. A pane that resizes visually while its harness
still believes the old dimensions is the failure this rule exists to prevent.

**Geometry is measured in cells, not pixels.** The conversion happens once, in the UI, where the
font metrics are known. Everything downstream speaks columns and rows.

**A pane starts at 80×24 and is told the truth a frame later.** The harness has to be started before
the emulator has been given any bounds to measure, so it begins at the conventional size and is
resized as soon as the first measurement exists. A harness that starts at the wrong size and is
immediately resized draws correctly; one that never learns its size does not.

**An exited harness leaves its pane.** The pane keeps its final screen, shows that the process
ended, and stops accepting input. The user closes it when they are done reading it; nothing
disappears on them.

**Pane chrome is two rows at most.** Identity and state on the first, context on the second.
Everything else on screen belongs to the harness: Ubiq gives the emulator the background, foreground
and cursor colours its surface is drawn in, and leaves the sixteen ANSI colours to the emulator's
own defaults, because those are the colours the harness is choosing between.

**Layout is one of four modes:** a single pane filling the window, a vertical split, a horizontal
split, or a grid. Closing the focused pane moves focus to another pane rather than leaving the
window with none.

## Contract

The pane family of the transport contract: `TerminalOutput`, `TerminalInput`, `TerminalResize`,
`Focus`, `PaneExited` and `PaneError`. A pane's own lifecycle uses two of the session family,
`SpawnWorkspace` with its `WorkspaceSpawned` answer, and `CloseWorkspace`. Variant names, payload
fields, the byte-sequence rule and the per-pane ordering guarantee are owned by
[`../tech/transport-contract.md`](../tech/transport-contract.md).

Output is chunked as it was read, not as lines. The emulator reassembles partial escape sequences;
the pane does no buffering of its own beyond what backpressure requires.

## Implementation

**A pane is a tab in the workbench's bottom dock.** The dock's tab strip is the pane list plus one
tab that is not a pane: its `+` spawns one, clicking a tab focuses it, and the tab's dot says whether
the harness is still running. The log console is the last tab, is never closed, and carries no pane
ID — [`logs.md`](./logs.md) has it. A pane tab is what the strip's `+`, its close buttons and its
dots are about; the console borrows the strip and nothing else.

**The body below the strip is the emulator.** `crates/ubiq/src/ui/terminal.rs` draws it: `render()`
for the dock, `body()` for the focused pane's `TerminalView` or the "no pane" line where there is
none, and `config()` for the `TerminalConfig` every pane is built with. The strip carries the
actions of whichever tab is active, and `select_dock_tab()` on `AppState` is the one place a click
on it is resolved into a pane or the console. The view comes from the
vendored `gpui-terminal`, which parses the bytes with `alacritty_terminal` and draws the screen. It
holds no path, no process handle and no descriptor: it is constructed from a `Read` and a `Write`
that are ends of the bus, which is what keeps the UI honest about a pane being an ID plus a byte
stream. The frame around it belongs to [`workbench.md`](./workbench.md), the palette it is given to
[`../tech/ui-and-design.md`](../tech/ui-and-design.md).

**`crates/ubiq/src/bus.rs` is the seam.** `pair()` opens the two channels the halves talk over.
`pane_output()` makes one pane's byte stream: the sender goes to the window's router, and the
matching `PaneOutput` is the blocking `Read` the emulator consumes on a thread of its own — dropping
the sender is how it learns the stream is over. `PaneInput` is the `Write`, and a keystroke written
into it leaves as `TerminalInput` for that pane ID.

**`AppState` in `crates/ubiq/src/app.rs` owns the panes, the focused pane and the layout mode**, plus
one emulator and one output sender per pane ID. Two tasks it starts in `for_project()` do the rest:
a router draining the bus into `receive()`, and one carrying a measured geometry into
`resize_pane()`. Every mutator ends by requesting a redraw — one that skips it is a pane that stops
updating.

The paths through the two halves, in call order:

| What the user does | ui → state → orchestrator → pty |
|---|---|
| Opens a pane | `spawn_pane()` sends `SpawnWorkspace`; the coordinator resolves the agent type, `pty::spawn` opens a pseudo-terminal and starts the child, and the answer `WorkspaceSpawned` reaches `open_pane()`, which builds the tab and the emulator |
| Types | the emulator writes into `PaneInput`, which posts `TerminalInput`; the coordinator finds the pane's `Pty` and writes to the pseudo-terminal |
| Watches output | `Pty::forward_output` puts a reader thread on the pseudo-terminal, sending `TerminalOutput` in fixed chunks; `receive()` hands the bytes to the pane's output sender, and the emulator reads them |
| Resizes | the emulator measures its own bounds and its resize callback sends `TerminalResize`; `Pty::resize` sets the size and the kernel signals the harness |
| Clicks another tab | `focus_pane()` sends `Focus` and queues the keyboard, which `take_focus()` hands over on the next frame, because focusing needs a window |
| Closes a pane | `close_pane()` sends `CloseWorkspace` and drops the tab and its emulator; the coordinator kills the child, and the thread `pty::reap` left waiting on it collects the exit |

`crates/ubiq/src/orchestrator.rs` holds one `Pty` per pane ID and nothing about layout or colour;
`crates/ubiq/src/pty/` is the only place a descriptor or a process lives. Its reader thread and the
bus's channels are unbounded, because a UI that has fallen behind must never stall the reader — a
stalled reader stalls the harness.

## Failure

| What happens | Result |
|---|---|
| The UI cannot keep up with output | The pane's queue grows; nothing is dropped and the coordinator's reader is never blocked |
| Output is not valid UTF-8 at a chunk boundary | Passed through as bytes; the emulator handles the split sequence |
| A resize arrives for an unknown pane | Ignored; a pane that has gone is not an error the user needs |
| The harness exits | `PaneExited`; the pane persists, showing its last screen, and its output stream ends |
| The harness cannot be started | `PaneError` against a pane ID the UI never drew; no tab appears |
| The focused pane closes | Focus moves to another pane, or to none if it was the last |

## Related docs

- [`sessions-and-workspaces.md`](./sessions-and-workspaces.md) — what a pane is a view of
- [`workbench.md`](./workbench.md) — the shell the dock sits in
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the message set, in full
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and the chrome conventions
- [`../tech/architecture.md`](../tech/architecture.md) — why the UI holds no pseudo-terminal

## Next steps

- Split, close and navigate panes from the keyboard.
- Zoom a pane to fill the window and back.
- Scrollback search within a pane.
- Give a subagent's pane visible parentage, so a spawned agent reads as belonging to its parent.
