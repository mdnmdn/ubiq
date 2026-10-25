---
id: settings-project
title: Project settings
summary: The settings that belong to one project — General, Remote, Tasks, Knowledge base, Tools and the rest.
keywords: [project settings, modes, remote, drone, boards, lanes, mission term, search excludes, index, storage, task sync]
order: 90
status: current
related: [settings, projects, workbench-modes]
---

Project settings holds what belongs to one [project](projects) rather than to the whole application.
Open it with the **Project settings** button beside the project's name in the
[title bar](workbench-titlebar); it opens on **General**, and **Save changes** and **Cancel** are at the
foot. A folder opened without being kept has a **Keep this project** button there instead, which
opens **General** alone: name the project and create it, and the other sections appear.

## The parts

| Section and setting | What it changes for you |
|---|---|
| **General**: **Color**, **Project name**, **Description** | The project's colour on the title chip and rail, its name, and the text given to every agent as context |
| **General**: **Project path**, **Rail initials** | The folder, which cannot be changed here, and up to three letters for the rail badge |
| **General**: **Modes** | Which of the project's destinations the rail shows; the last one on cannot be turned off |
| **General**: **Project data** | Keeps the project's tasks and settings in **Ubiq's config folder** or **In the project (.ubiq/)**, moving them when you switch |
| **General**: **Keep an index**, **Mission term**, **Search excludes**, **Repositories** | This project's own index level, word for a mission and search skips, each overriding the application's, and the git repositories found inside it |
| **Tools** | Commands run in this project's folder, on top of the machine-wide ones; see [Tools and command line](settings-tools) |
| **Agent definitions** | Which saved setups a start in this project is offered, or **Use the global agents** only |
| **Skills**, **MCP servers** | Skills and tool servers only this project's agents can be given |
| **Tasks** | **Shown** and **Shut when empty** for each lane of the board, and the **Boards** list with **Shown**, **Delete** and **New board** |
| **Task sync** | Binds the board to a board kept somewhere else |
| **Remote** | **Runs here** or **On a drone**, with **Reached through** (an SSH profile), **Folder on that machine** and **Lifetime**, then **Save** |
| **Knowledge base**, **Databases** | The sources documents are read from, and the connections databases are reached by; see [Knowledge base sources](kb-sources) and [Databases](db-overview) |

## What you can do here

- Turn on **DB** or hide a mode from the rail: **General**, then **Modes**; the modes are described in
  [Rail modes](workbench-modes).
- Move a project onto another machine: [Work on a remote machine](guides-remote-machine).
- Rename the board's word for a mission or add a named board: [The Tasks board](tasks-overview).
- Choose which agent definitions a project offers: [Agent definitions](agents-definitions).

## When something goes wrong

- **Only General is listed.** The folder is not kept yet; give it a name and create it.
- **The path cannot be changed.** It is set when the folder is chosen, and this window cannot edit it.
- **A lane has gone from the board.** Its **Shown** switch is off; nothing in it was deleted or moved.
- **A board will not delete.** A board that holds tasks cannot be deleted; the default board is
  always shown.
- **A project on another machine has stopped answering.** Its row stays in the project list; see
  [Remote hosts](remote-overview) to reconnect.
