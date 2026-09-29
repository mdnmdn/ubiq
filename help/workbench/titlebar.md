---
id: workbench-titlebar
title: The title bar
summary: The window's top row — the project cluster, the command field, and every action in the right-hand cluster.
keywords: [titlebar, title bar, command field, new agent, new terminal, search, notifications, feedback, help, point at something, settings, remote hosts, theme]
targets: [ui.titlebar, ui.titlebar.project, ui.titlebar.new-project, ui.titlebar.new-project-menu, ui.titlebar.nav-back, ui.titlebar.nav-forward, ui.titlebar.command, ui.titlebar.actions, ui.titlebar.region-left, ui.titlebar.region-bottom, ui.titlebar.region-right, ui.titlebar.new-agent, ui.titlebar.hidden-agents, ui.titlebar.new-terminal, ui.titlebar.new-terminal-menu, ui.titlebar.search, ui.titlebar.notifications, ui.titlebar.feedback, ui.titlebar.help, ui.titlebar.point-at, ui.titlebar.settings, ui.titlebar.overflow, ui.titlebar.theme]
order: 15
status: current
related: [workbench-rail, workbench-modes, workbench-panels, getting-started]
---

## Three clusters

The title bar is the window's top row, and it is always three clusters left to right: **project**,
**command**, and **actions**.

## Project, on the left

- **Project** shows the window's letter and the open project's name; click it to switch project or
  to open one that is not here yet.
- **Add project** opens a folder already on this machine as a project in the window. The chevron
  beside it is **More ways to add** — clone a repository, or open a project on a remote host —
  rather than a second copy of the same action.
- **Back** and **Forward** step through this window's own navigation history — the mode, the tab and
  the place in it — with **⌃-** and **⌃⇧-**. This history is the window's, independent of the help
  panel's own back and forward.

## Command, in the middle

**The command field** is one field doing three jobs: find a file, search the project, and run a
command. **⌘P** jumps to it to go to a file; **⌘⏎** searches instead of jumping. It is worth learning
as a single habit rather than three separate shortcuts, because which of the three it does depends
only on what you type into it.

## Actions, on the right

The right-hand cluster is what the window can start, show and put away. Three region toggles —
**left panels**, **bottom panels**, **right panels** — show or hide everything docked to that side
without closing any one panel in it; closing a panel and hiding a region are different actions with
different memories, and a hidden region remembers what was in it.

Starting things:

- **New agent** starts a conversation — harness, identity, model and permission level in one
  question. See [Starting an agent](agents-starting). The narrow chevron beside it, **Agents not on
  screen**, lists the agents that are still running with nothing drawing them, and picking one
  brings it back.
- **New terminal** opens a shell in the project's folder in a pane of its own; the chevron beside it,
  **More terminals**, offers the other shells and harnesses this machine has rather than the default
  one.
- **Search** (**⌘⇧F**) searches the whole project and shows the results in their own panel, distinct
  from the command field's quick search.

Watching things:

- **Notifications** — the bell — lists everything that happened while you were elsewhere, newest
  first.
- **Theme** switches the window's palette.

Getting help, and getting heard:

- **Help** (**F1**) opens the manual on the page for wherever you are standing right now.
- **Point at something** (**⇧F1**) is the same question the other way round: it waits for you to
  point at a control and tells you about that control, rather than the screen around it.
- **Send feedback** attaches a picture of the window to whatever you type, so a report always shows
  what you were looking at.
- **Settings** opens accounts, agents, connections and everything else the application keeps. Its
  chevron, **More**, holds the commands wanted occasionally rather than every session — **Remote
  hosts** and **Connect to a remote host**, **Explore the project in browser**, **Capture this
  window** and **Help**.
