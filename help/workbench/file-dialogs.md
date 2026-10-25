---
id: workbench-file-dialogs
title: File dialogs and the file picker
summary: The questions Ubiq asks about files — create, rename, move, delete, save — and choosing files or folders with the picker.
keywords: [new file, rename, delete, trash, move, save as, overwrite, unsaved, choose folder, picker, browse, attach]
order: 150
status: current
related: [workbench-explorer, workbench-editor, projects]
---

When you create, rename, move, delete or save a file, Ubiq asks in a small dialog in the middle of the
window. **Enter** answers the question and **Esc** cancels it. A separate file picker opens where Ubiq
needs you to choose files or folders, such as attaching to a task or picking a folder on a remote
machine. The [file explorer](workbench-explorer) raises most of the questions.

## The parts

| Dialog | What it asks |
|---|---|
| **New file**, **New folder**, **New Excalidraw**, **New draw.io** | A **Name**, and says where it will be made, in a folder or the project's top level; **Create** makes it |
| **Rename** | A **Name**; anything open on the file follows the new name. A box offers to rename its related files too |
| **Delete** or **Move to Trash** | Confirms removing a file or folder, and says everything inside a folder goes too; **Delete permanently** or **Move to Trash** confirms. A box offers to remove its related files as well |
| Move | Asks before a dragged file or folder moves into another folder; a box skips the question for ten minutes |
| **Save as** | A **Path** inside the project for an untitled file; **Save** writes it |
| **File already exists** | Asks before **Overwrite** replaces the file at that path |
| **Unsaved changes** | Offers **Reload** to replace your edits with what is on disk, or **Discard** to close the tab without saving |
| **Not saved** | Says why a save failed and that your edits are still in the tab |
| **Rename tab** | Renames a terminal or chat tab |
| Close window, **Close project** | Lists what is unsaved or still running and asks before **Close anyway** |

## The file picker

The picker is a larger dialog that you work in rather than answer. It has a title, a filter field,
buttons to switch between a tree and a flat list, a **×** to close it, and a corner handle to resize
it. The footer shows what to press and the confirm button, **Select** for one choice or **Add** with a
count for several. Where several can be chosen, each row has a tick box; press **Enter** to tick and
**⌘Enter** / **Ctrl+Enter** to add. A row with a `…` is still loading, and one with a `+` was cut short.

The eye in the header shows or hides hidden files. When you browse another machine, an up arrow and
the folder's path appear beside the filter. Picking a project on a remote host adds a **Keep project**
box, which keeps the project running there so a later reconnect finds it.

## What you can do here

- Create, rename, move and delete from the tree: [The file explorer](workbench-explorer).
- Save and close with edits pending: [The editor](workbench-editor).
- Add a folder as a project: **Add a project** in the [title bar](workbench-titlebar).
- Learn what a project is: [Projects](projects).

## When something goes wrong

- **Not saved** names a reason: fix it, then save again. Your edits stay in the tab meanwhile.
- The picker says `Nothing matches`: the filter excludes every row. Clear it.
- The picker's confirm button looks faded: nothing is ticked yet.
- A file you deleted is needed: **Move to Trash** keeps it in the system trash; **Delete permanently**
  does not.
