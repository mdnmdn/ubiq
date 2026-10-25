---
id: workbench-tools
title: Runnable tools
summary: The commands a project offers to run from the title bar — pick and run one, and define your own in Settings.
keywords: [tools, run, commands, make, just, mise, tasks runner, favourite, star, play, build, watch]
order: 120
status: current
related: [workbench-titlebar, settings-tools, settings-project, workbench-panels]
---

A tool is a named command that runs in a terminal pane in the project's folder: a build, a watcher, a
shell with flags. Ubiq lists the ones you define and the ones it finds in the project's `Makefile`,
`justfile` and `mise` config. The play button in the [title bar](workbench-titlebar) runs the first
one, and the chevron beside it opens the list.

## The parts

| Part | What it does |
|---|---|
| Play button | Runs the first tool in the list; its tooltip reads **Run** and the tool's name, or **No tools for this project** |
| Chevron, **Run a tool** | Opens the run picker |
| Picker filter | Narrows the list as you type; **↑** and **↓** move, **Enter** runs the row, **Esc** closes it |
| **Favorites**, **Recent** | Starred tools, then the last few you ran; the rest follow, your own before the ones found in project files |
| Row mark | Where the tool came from: **Your tool**, **Make target**, **Just recipe** or **Mise task**; hover for the label |
| Green dot | The tool's pane is running |
| **Star** / **Unstar** | Pins the tool to **Favorites** |
| **Run**, **Run again** | Starts the tool; **Run again** replaces a pane whose command has ended |
| **Stop**, **Restart** | Ends a running tool, or ends it and starts it again |

Click a tool's name to go to its pane. A pane whose command has ended stays open only if the tool
waits on exit.

Define tools in **Settings**, then **Tools**, for every project on this machine, or in a project's
settings under **Tools** for that project alone. Each row shows its command, the platforms it runs on
and what happens when it ends, with **Edit** and a **×** to remove it. **Add tool** opens the form:

| Field | What it holds |
|---|---|
| **Name** | What the tab says; a number joins it when two panes run the same tool |
| **Command**, **Parameters** | The program, and its arguments split the way a shell splits them |
| **Environment** | Extra variables, one `KEY=VALUE` per line |
| **Platforms** | **macOS**, **Windows**, **Linux**; none picked means everywhere |
| **Starting folder** | Where it runs instead of the project's own folder |
| **Wait on exit**, **Wait on error** | Keep the pane open after the command ends, or only when it fails |
| **Single instance** | Refuses a second run while the first pane is open |

## What you can do here

- Open the title bar and its other actions: [The title bar](workbench-titlebar).
- Add a machine-wide tool: [Tools settings](settings-tools).
- Add a tool for one project: [Project settings](settings-project).
- Read a finished tool's output: [Hide or close a tab](how-to-hide-or-close-a-tab).

## When something goes wrong

- The play button says **No tools for this project**: add one in settings, or add a recipe to a
  `justfile`, `Makefile` or `mise` config the project already has.
- The picker says `No tool matches`: clear the filter.
- A tool is missing: its **Platforms** may leave this system out.
- **Run** is refused: the tool is a single instance and its first pane is still open. **Stop** it or
  close that pane.
- A finished tool's pane vanished: turn on **Wait on exit** in the tool's form to keep its output.
- **Environment** reports `1 line ignored`: that line is not `KEY=VALUE`.
