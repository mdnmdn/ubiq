---
id: feat-panes
title: Panes and terminals
kind: feature
status: draft
summary: What a pane shows, how exactly one of them holds focus, how a resize reaches the harness, and the layout modes panes are arranged in.
read_when: you are changing pane layout, focus, resize, pane chrome, or how terminal bytes reach the screen
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [crates/ubiq/src/app.rs, crates/ubiq/src/ui/mod.rs, crates/ubiq/src/pty/mod.rs]
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

**Exactly one pane holds focus.** Keystrokes go to that pane's harness and nowhere else. Unfocused
panes keep drawing — an agent working in the background stays visible — but take no input. Focus is
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

**An exited harness leaves its pane.** The pane keeps its final screen, shows that the process
ended, and stops accepting input. The user closes it when they are done reading it; nothing
disappears on them.

**Pane chrome is two rows at most.** Identity and state on the first, context on the second.
Everything else on screen belongs to the harness — the terminal body is drawn by the agent's own
output and Ubiq applies no styling to it.

**Layout is one of four modes:** a single pane filling the window, a vertical split, a horizontal
split, or a grid. Closing the focused pane moves focus to another pane rather than leaving the
window with none.

## Contract

The pane family of the transport contract: `TerminalOutput`, `TerminalInput`, `TerminalResize`,
`Focus`, `PaneExited` and `PaneError`. Variant names, payload fields, the byte-sequence rule and the
per-pane ordering guarantee are owned by
[`../tech/transport-contract.md`](../tech/transport-contract.md).

Output is chunked as it was read, not as lines. The emulator reassembles partial escape sequences;
the pane does no buffering of its own beyond what backpressure requires.

## Implementation

**A pane is a tab in the workbench's bottom dock.** The dock's tab strip is the pane list: its `+`
spawns one, clicking a tab focuses it, and the tab's dot says whether the harness is still running.

`AppState` in `crates/ubiq/src/app.rs` owns the panes, the focused pane and the layout mode.
`spawn_pane()` creates a pane and gives it focus; `close_pane()` removes one and moves focus if it
held it; `resize_pane()` records new dimensions; `focus_pane()` moves focus to an existing pane.
Each ends by requesting a redraw — a mutation that skips that is a pane that stops updating.

Rendering is `crates/ubiq/src/ui/terminal.rs`: the tab strip, and a body that names the focused pane
and its geometry and nothing more. That body is the seam the terminal emulator drops into — it holds
no path, no process handle and no descriptor, because the UI knows a pane only as an ID plus a byte
stream. The frame around it belongs to [`workbench.md`](./workbench.md). Colours come from theme
tokens; see [`../tech/ui-and-design.md`](../tech/ui-and-design.md).

On the other side of the bus, `crates/ubiq/src/pty/` owns the streams: a reader per pane forwarding
output, a writer taking input, and the resize call on the pseudo-terminal master. The reader must
never be blocked by a UI that has fallen behind — a stalled reader stalls the harness.

## Failure

| What happens | Result |
|---|---|
| The UI cannot keep up with output | Frames are dropped or coalesced. The coordinator's reader is never blocked |
| Output is not valid UTF-8 at a chunk boundary | Passed through as bytes; the emulator handles the split sequence |
| A resize arrives for an unknown pane | Ignored; a pane that has gone is not an error the user needs |
| The harness exits | `PaneExited`; the pane persists, showing its last screen |
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
