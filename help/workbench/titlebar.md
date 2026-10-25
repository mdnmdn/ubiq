---
id: workbench-titlebar
title: The title bar
summary: The window's top row — the project cluster, the command field, and every action in the right-hand cluster.
keywords: [titlebar, title bar, command field, navigator, new agent, new terminal, search, notifications, feedback, help, point at something, settings, remote hosts, theme, back, forward, run tool]
targets: [ui.titlebar, ui.titlebar.project, ui.titlebar.new-project, ui.titlebar.new-project-menu, ui.titlebar.nav-back, ui.titlebar.nav-forward, ui.titlebar.command, ui.titlebar.actions, ui.titlebar.region-left, ui.titlebar.region-bottom, ui.titlebar.region-right, ui.titlebar.new-agent, ui.titlebar.hidden-agents, ui.titlebar.new-terminal, ui.titlebar.new-terminal-menu, ui.titlebar.search, ui.titlebar.notifications, ui.titlebar.feedback, ui.titlebar.help, ui.titlebar.point-at, ui.titlebar.settings, ui.titlebar.overflow, ui.titlebar.theme]
order: 15
status: current
related: [workbench-rail, workbench-modes, workbench-panels, workbench-notifications, workbench-tools, getting-started]
---

The title bar is the window's top row, in three clusters from left to right: the project, the
command field and the actions. Buttons with a split edge are pairs: the main button does the common
thing, and its narrow chevron lists the alternatives.

## The parts

| Part | What it does |
|---|---|
| Project name | Shows the open project, with the window's letter when you have several windows; click to switch project, add or clone one |
| **Project settings** | The three-dot button beside the name; on a temporary project it is **Keep this project** |
| **Open** `<address>` ![Open](../img/ui/titlebar-browser.png) | Opens the project's repository page in your browser; shown only when the project has a remote with a page |
| Play button and **Run a tool** | Runs the project's first tool, and its chevron lists the rest; see [Runnable tools](workbench-tools) |
| **Add a project** | Adds a folder on this machine; its chevron, **Open a project**, adds **Clone a project…** and **Open a remote project…** |
| Back, Forward | Step through where this window has been, across projects; **⌃-** / **Ctrl+-** and **⌃⇧-** / **Ctrl+Shift+-**. Their tooltips name the place |
| Command field | Marked **⌘K**: type to jump to a file, a task, an agent or a recent place, or to search |
| Region buttons | Three buttons ![Left panels](../img/ui/titlebar-panel-left.png) ![Bottom panels](../img/ui/titlebar-panel-bottom.png) ![Right panels](../img/ui/titlebar-panel-right.png) that show or hide the left, bottom and right regions |
| **New agent** | Starts an agent; its chevron, **Agents not on screen**, brings back agents still running without a tab |
| **New terminal** | Opens a shell in the project's folder; its chevron, **Run in a new pane**, lists other shells, **Detached** panes and **Logs** |
| Search ![Search](../img/ui/titlebar-search.png) | Opens the [Search panel](workbench-search); **⌘⇧F** |
| **Notifications** ![Notifications](../img/ui/titlebar-notifications.png) | The bell, with the unread count; see [Notifications](workbench-notifications) |
| **Send feedback** ![Send feedback](../img/ui/board-comment.png) | Opens the [feedback dialog](feedback) |
| **Help** ![Help](../img/ui/titlebar-help.png) | Opens the manual on this screen's page; **F1** |
| **Point at something (⇧F1)** ![Point at something](../img/ui/titlebar-point-at.png) | Describes whichever control you point at; see [the help panel](workbench-help) |
| **Settings** ![Settings](../img/ui/titlebar-settings.png) | Accounts, agents, connections and the rest; its chevron, **More**, holds **Remote hosts**, **Connect to a remote host**, **Explore the project in browser**, **Capture this window**, **Help** and **Point at something…** |
| Theme ![Light](../img/ui/titlebar-theme-light.png) ![Dark](../img/ui/titlebar-theme-dark.png) | Switches between the light and dark palettes |
| **About Ubiq** ![About Ubiq](../img/ui/titlebar-about.png) | The version, updates and licences |

**New agent** and **New terminal** need an open project. The command field is one habit for three
jobs: with nothing typed it offers where you were, and typed text offers matches. **⌘⏎** searches the
project for what you typed. Press **⌘K** from anywhere to reach it, and **⌘P** / **Ctrl+P** to filter
the explorer instead.

## What you can do here

- Jump to a file, a task or a project by typing: [Use the navigator](how-to-use-the-navigator).
- Start an agent: [Starting an agent](agents-starting).
- Hide, show or rearrange regions: [Panels and the dock](workbench-panels).
- Switch to light or dark, or change size: [Change theme and size](how-to-change-theme-and-size).

## When something goes wrong

- **New agent** and **New terminal** are missing: open a project first with **Add a project**.
- **Back** or **Forward** looks faint: there is nowhere to go in that direction.
- **Capture this window** is missing from **More**: it is switched off under **Settings**.
- A bell that flashes goes straight to its place when clicked; to open the list, wait for it to stop.
- The title bar shows no repository button: the project has no remote Ubiq can link to.
