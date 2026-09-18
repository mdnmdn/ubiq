---
id: panes
title: Panes and terminals
summary: A pane is a real terminal running one agent — how it is named, how focus and resize work.
keywords: [terminal, tab, focus, resize, alternate screen]
order: 10
status: current
related: [sessions, workbench-panels]
---

## A pane is a terminal, not a text buffer

Every harness Ubiq hosts is a full-screen terminal program: it takes over the alternate screen,
addresses the cursor absolutely, and expects raw keystrokes back — arrows, Ctrl and Alt chords,
pasted text. A pane is where you watch that happen: a real terminal emulator with a tab naming which
agent is running and whether its harness is still alive.

## Focus

Exactly one pane holds focus at a time and receives your keystrokes; every other pane keeps drawing
regardless. Click a pane, or use the keyboard shortcuts listed in the titlebar's overflow menu, to
move focus between the agents you are running side by side.

## Resize

Dragging a pane's border resizes the terminal and tells the harness underneath it, so the program
redraws at the new size rather than believing it is still the old one. A pane that resizes visually
without the harness knowing is the classic way a terminal multiplexer corrupts its own screen — Ubiq
propagates every resize through.

## Closing

Typing `exit` or sending Ctrl+D inside a pane ends its harness and closes the pane's tab with it. The
tab's **×** either hides the pane (the harness keeps running, and reopening the panel reattaches to
it) or closes it outright after a confirmation — which one depends on the kind of pane, and the
tab's right-click menu always offers both.
