---
id: settings-tools
title: Tools and command line
summary: The Tools and Command line sections — the runnable tools you define and installing the command-line launcher.
keywords: [tools, command line, cli, launcher, install, ubiq command, path, run, environment, parameters]
order: 80
status: current
related: [settings, workbench-tools, settings-project]
---

**Tools** is the list of named commands you can run in a terminal pane on this machine, such as a
build, a watcher or a shell with flags. **Command line** installs the `ubiq` command, so you can open
a folder or a file in Ubiq from a shell. Both are sections of [Settings](settings); a project can
add tools of its own in [Project settings](settings-project), run on top of these.

## The parts

| Setting | What it changes for you |
|---|---|
| A tool row | The tool's name and the command it runs, with **Edit** and a remove button |
| **Add tool** | Opens the form below the list |
| **Name** | What the tab says; a number joins it when two panes run the same tool |
| **Command**, **Parameters** | The program, and its arguments split the way a shell splits them |
| **Environment** | Extra variables for the run, one `KEY=VALUE` per line |
| **Platforms** | Where the tool runs: **macOS**, **Windows**, **Linux**; none picked runs everywhere |
| **Starting folder** | Where it runs instead of the project's own folder, chosen on the machine it runs on |
| **Wait on exit**, **Wait on error** | Keep the pane open after the command ends, or only when it fails, so its output stays readable |
| **Single instance** | A second run is refused while the first pane is still open |
| **Save tool** / **Cancel** | Keeps the form's tool (**Add tool** while creating one), or discards it |
| **The `ubiq` command** | **Install** writes a small launcher into a folder on your `PATH`, **Update** replaces one that launches another build, **Remove** takes it away |

## What you can do here

- Run a tool: [Runnable tools](workbench-tools).
- Keep a tool with one project rather than the whole machine: [Project settings](settings-project).
- Open a folder from a shell with `ubiq .`, or a file with `ubiq README.md`; Ubiq uses the window
  that already holds it when there is one.

## When something goes wrong

- **The command-line button is dimmed.** No folder on this machine can hold the command; the line
  beside it says so.
- **The line says not on PATH.** Ubiq wrote the command, but your shell does not look there; add
  the folder to your shell profile.
- **The line says it launches another build of Ubiq.** Press **Update**.
- **I want to take the command away.** Press **Remove**; it deletes only the script Ubiq wrote, never
  a `ubiq` you put there yourself.
- **A tool does nothing on this system.** Its **Platforms** leave this system out.
