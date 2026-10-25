---
id: workbench-all-projects
title: All projects
summary: The window listing every project Ubiq knows, filtered as you type, to open one here, in a new window, or forget it.
keywords: [projects, switch project, open project, list, history, forget, others]
order: 110
status: current
related: [projects, workbench-rail, workbench-titlebar]
---

All projects lists every folder Ubiq has opened as a project, including ones no window holds now.
The project picker in the [title bar](workbench-titlebar) shows only the nine most recent from its
**History**; when there are more, its last row, **Others…**, opens this list. [Projects](projects)
explains what a project is.

## The parts

| Part | What it does |
|---|---|
| Filter field | Narrows the list as you type, on a project's name or any part of its folder |
| Row | A dot in the project's colour, its name, and the end of its folder path; hover either for the full path |
| **Open in this window** | Brings the project into this window, and switches to it if it is already here |
| **Open in a new window** | Opens the project in a window of its own |
| **Forget** | Removes the project from Ubiq's list; the folder on disk is untouched |

Opening a project closes the list. A project is open in one window at a time, so **Open in this
window** takes it from another window if that is where it was.

## What you can do here

- Add a folder, clone a repository or open a remote one: the **+** menu in the
  [title bar](workbench-titlebar).
- Switch between the projects this window already holds: the badges on the [rail](workbench-rail).
- Understand what Ubiq remembers per project: [Projects](projects).

## When something goes wrong

- The list says `No projects match.`: the filter matches nothing. Clear it, or try part of the
  folder's path instead of its name.
- **Others…** is not in the picker: **History** shows up to nine projects, and the row appears only
  when there are more.
- A project you forgot is still wanted: add its folder again with **Add a project**.
- A project's folder has moved: choose **Locate** on its row in the project picker and pick the new
  place.
