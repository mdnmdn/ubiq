---
id: workbench-explorer
title: The file explorer
summary: The project's file tree in IDE mode's left region — open, filter, create, rename, move and delete files from it.
keywords: [explorer, file tree, sidebar, rename, delete, move, copy, duplicate, filter, hidden files, follow]
context: [panel.ubiq.explorer]
order: 30
status: current
related: [workbench-editor, workbench-search, workbench-bookmarks, workbench-file-dialogs, workbench-panels]
---

The explorer draws the project's folder as a tree in the left region of IDE mode, and it is open the
moment you enter the mode. Click a file to open it in the [editor](workbench-editor). A file or
folder created or removed outside Ubiq, by an agent in a pane or a command in another terminal,
appears or disappears in the tree on its own.

## The parts

| Part | What it does |
|---|---|
| Tree and list buttons | Switch between the nested tree and a flat list of every file |
| **+** | Opens a menu of **New file**, **New folder**, **New Excalidraw** and **New draw.io**, **Paste** and **Collapse all** |
| Follow button | Cycles its three states: off, **Revealed once — click to lock** and **Following the active editor**, which keeps the tree on the file in the active tab |
| Collapse button | Folds every open folder |
| Eye | Shows or hides hidden files |
| **Bookmarks** | A fold-out list above the filter, drawn only once you have a [bookmark](workbench-bookmarks) |
| Filter field | Narrows the tree to matching names as you type; **⌘P** / **Ctrl+P** puts the caret in it |
| Letter on a row | The file's git state: its letter, in the colour the path has in [Git mode](git-overview) |
| Branch mark on a folder | The folder is its own repository or submodule; hover for its branch |

Click a file once to open a preview tab that the next click replaces; double-click it to keep the tab.
Drag a row onto a folder to move it; Ubiq asks first, and the question has a box to stop asking for
ten minutes. A `…` after a folder means it is still loading, and a `+` means its listing was cut short.

Right-click a row for **Open**, **Open with**, **Open diff vs HEAD**, **New file** and its siblings,
**Copy**, **Cut**, **Paste**, **Duplicate**, **Copy path**, **Copy full path**, **Copy link**,
**Open in Web**, **Refresh**, **Exclude from search**, **Rename** and **Delete**. On a Markdown file it
also offers **Open in annotation mode**.

With the caret in the tree, the arrow keys move and fold, **Enter** opens, **Tab** jumps to the filter,
**Delete** or **Backspace** deletes, and **⌘C** / **Ctrl+C**, **⌘X** / **Ctrl+X** and **⌘V** / **Ctrl+V**
copy, cut and paste.

## What you can do here

- Find text inside files, not names: [Search the project](how-to-search-the-project).
- Create, rename, move and delete, and what each question asks: [File dialogs and the file picker](workbench-file-dialogs).
- Keep a place you return to often: [Bookmarks](workbench-bookmarks).
- Hide what the tree and search skip: [File explorer, editor and search](settings-files-and-search).

## When something goes wrong

- The tree says no project is open: open a folder first with **Add a project** in the
  [title bar](workbench-titlebar).
- The filter shows **Nothing matches**: it matches names, not file contents, and waits for three
  characters before it searches. Use [Search](workbench-search) for contents.
- A file you expect is missing: if it is a hidden file, press the eye to show hidden files.
- The explorer is gone: closing its tab removes it, and the left-region button in the title bar
  brings it back.
