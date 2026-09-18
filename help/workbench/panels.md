---
id: workbench-panels
title: Panels and the dock
summary: How panels dock, move and close, and where the logs panel fits among them.
keywords: [panel, dock, drag, split, logs, console]
context: [panel.ubiq.logs]
order: 20
status: current
related: [workbench-modes, panes]
---

## Panels are the movable unit

Everything in the window's four regions — centre, left, right and bottom — is a panel: a terminal, a
file, the explorer, the log console, this help panel. A panel has a tab, and dragging that tab moves,
splits or groups it with another; closing a tab's **×** either hides or closes it, depending on what
kind of panel it is.

## The logs panel

Ubiq runs several things at once that can fail independently — a window, a coordinator, a
pseudo-terminal and a reader thread per pane. The **logs** panel is where every subsystem's
diagnostics land in one place: one ring buffer, read with two controls, which subsystem and how
loud. Open it like any other panel; it docks beside a terminal, in a group of its own, or wherever
you drag it, and stays there across restarts.

## Nothing is furniture

A region you close stays closed until you reopen it — no panel forces itself back onto the screen
because a mode changed. The layout you leave is the layout you get back.
