---
id: workbench-panels
title: Panels and the dock
summary: How panels sit in the window's four regions, how to move, split, zoom and close them, and what a tab's menu offers.
keywords: [panel, dock, drag, split, region, tab, zoom, pin, close]
order: 20
status: current
related: [workbench-modes, workbench-titlebar, workbench-logs, how-to-split-and-move-panes, how-to-hide-or-close-a-tab, panes]
---

Everything between the title bar and the status bar is the dock. It has four regions — centre, left,
right and bottom — and each holds panels grouped under tabs. You arrange it: the window picks a
starting layout for each mode, and from there the layout is yours and is remembered. A *panel* is
anything with a tab; a [pane](panes) is a panel that holds a terminal.

## The parts

| Part | What it does |
|---|---|
| Tab | The handle of a panel; click it to bring the panel forward in its group, drag it to move the panel |
| Tab **×** | Closes the panel, or hides it while its agent keeps running; for terminal and chat tabs, **Settings**, then **Editor**, then **Closing a tab** decides which |
| **+** and its chevron | At the right end of a group's tab strip: **+** starts a pane, the chevron lists what else can be opened there |
| Zoom button | At the strip's far right: fills the region with the group and gives it back |
| Region buttons | In the [title bar](workbench-titlebar): show or hide the left, bottom and right regions |
| Divider | Drag it to resize the groups either side |

Drop a tab on the middle of a group to add it as a tab there, or on a group's edge to split off a new
row or column. A panel dropped where its kind cannot sit moves back to its own region. The explorer
stays in the left or right region, a file stays in the centre, and terminals, chats, the logs, search
and outline can go anywhere.

A tab's right-click menu depends on the panel. A file tab offers **Close**, **Close Others**,
**Close Left**, **Close Right**, **Close All**, the path commands, **Reveal in Explorer**, **Save**,
**Word Wrap** and **Pin** or **Unpin**. A terminal or chat tab offers **Rename…**, **Hide**,
**Close** and **Pin** or **Unpin**, plus **Restart** on a tool that has finished. A pinned tab has no
**Close** and survives **Close Others** and **Close All**.

Some panels have a page of their own: [The file explorer](workbench-explorer), [Search](workbench-search),
[The outline](workbench-outline), [The log console](workbench-logs) and [The help panel](workbench-help).

## What you can do here

- Split a region or move a panel: [Split and move panes](how-to-split-and-move-panes).
- Keep a terminal running out of sight, or end it: [Hide or close a tab](how-to-hide-or-close-a-tab).
- Open the logs beside a terminal: [Find the logs](how-to-find-the-logs).
- Make the whole window larger or smaller: [Change the theme and interface size](how-to-change-theme-and-size).

## When something goes wrong

- A region is gone: its button in the title bar opens it again. A region you close stays closed
  until you reopen it; no mode switch brings it back.
- A region opens with one panel in it, not the one you had: closing a region's last panel empties it,
  and the button fills it again with that mode's own panel.
- A panel will not stay where you dropped it: its kind cannot sit in that region, so it went home.
- A tab has no **×** action you expected: pinned tabs cannot be closed. Pick **Unpin** first.
