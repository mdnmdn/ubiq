---
id: workbench-status-bar
title: The status bar
summary: The strip along the bottom of the window — the open file and its kind, git state, counts for Agents and Tasks, update notice, size and version.
keywords: [status bar, footer, vim, file kind, viewer, size, update, version, branch, ahead behind]
order: 60
status: current
related: [workbench-titlebar, workbench-editor, workbench-modes, how-to-change-theme-and-size]
---

The status bar is the window's bottom row. It reports facts about what is on screen and draws a
missing fact as missing, so what it shows depends on the mode you are in. Most of it is read-only;
the file-kind chip, the **VIM** chip, the update notice and the size control are things you click.

## The parts

| Part | What it does |
|---|---|
| File name | In **IDE** mode, the active file's name, or `no file open` |
| File-kind chip | Beside the name: the file's language. Click it to draw the file in another viewer, and type to narrow the list |
| Git readout | Right of centre: the branch, `↑` ahead and `↓` behind counts, then `!` conflicted, `M` modified, `S` staged and `U` untracked paths. Absent when the project is not a repository |
| **VIM** | Faint when Vim keys are off, and the mode name when on. Click it to turn Vim keys on or off |
| Update notice | **Update available**, **Restart to update** or **Installs on quit**; click it to open the update settings |
| **Size** ![Size](../img/ui/size-text-large.png) | Opens the size panel; its tooltip names the current preset, or **Custom** |
| Version | The build's version; hover for the full string |

In **Agents** mode the left side counts columns, agents, grouped columns and agents on the bench,
then agents by state: running, waiting, ended and error. In **Tasks** mode it counts cards per
column, finished sub-tasks out of all sub-tasks, and blocked cards; a task source that syncs shows its
state first. With no project open the bar reads `no project`.

The size panel sets two things: how big the whole window is drawn, and how big its text is. It has
**Presets**, an interface slider, a text slider, **Text brightness** buttons, **Save preset…** and
**Reset**. The change applies at once.

## What you can do here

- Change the size, brightness or palette: [Change theme and size](how-to-change-theme-and-size).
- Read the open file in another viewer: [The editor](workbench-editor).
- Read what the git letters mean: [The Changes panel](git-changes).
- See what each mode is for: [Rail modes](workbench-modes).

## When something goes wrong

- No file name: the mode is not **IDE**, or no file is open. Open one from the
  [explorer](workbench-explorer).
- No git readout: the project is not a git repository.
- The file-kind chip is missing: it appears only in **IDE** mode with a file open.
- No update notice: it shows only while an update is available or waiting to install.
- An agent counts as `error` in **Agents** mode: open its column to read the conversation.
