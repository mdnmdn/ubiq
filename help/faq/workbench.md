---
id: faq-workbench
title: Workbench questions
summary: What people ask about tabs, panels and the window.
keywords: [tab, panel, dock, window, layout, rail, mode]
order: 20
related: [workbench-panels, panes]
---

## A panel or a whole region has disappeared. How do I get it back?

A region you close stays closed until you reopen it. Press its button in the title bar, and the region opens with that mode's own panel in it. A single panel closed from its tab is reopened from the new-pane chevron, for example **Logs**. See [Panels and the dock](workbench-panels).

## Why will a panel not stay where I dropped it?

Some kinds of panel only sit in some regions. The explorer stays in the left or right region and a file stays in the centre, so a panel dropped elsewhere goes back to its own region. Terminals, chats, the logs, search and outline can go anywhere. See [Split and move panes](how-to-split-and-move-panes).

## Can I move panels with the keyboard?

No. You move a panel by dragging its tab, and a tab's right-click menu offers **Hide**, **Close** and **Pin** but not a move. See [Split and move panes](how-to-split-and-move-panes).

## Why does my tab have no ×?

The tab is pinned, and a pinned tab cannot be closed or hidden. Right-click it and pick **Unpin**. See [Hide or close a tab](how-to-hide-or-close-a-tab).

## Where is DB mode?

**DB** is off until a project asks for it. Open the project's settings, go to **General**, and light the **DB** tile under **Modes**. See [Rail modes](workbench-modes).

## How do I switch to another project?

Click the project's name in the title bar, or click its badge on the rail when the window holds several; **⌘1** to **⌘9** jump to the nth badge. Switching project keeps the mode you are on. See [The rail](workbench-rail).

## Why did the bell's list empty after I restarted Ubiq?

The notification list and any silences you set are kept only while Ubiq runs, so a restart clears both. The list also holds only the 200 most recent. See [Notifications](workbench-notifications).
