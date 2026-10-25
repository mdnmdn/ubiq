---
id: workbench-editor
title: The editor
summary: Opening, reading, editing and saving files as tabs in IDE mode's centre, and drawing a file in another viewer.
keywords: [editor, tab, save, open file, markdown, preview, split, image, mermaid, excalidraw, draw.io, vim, word wrap, unsaved]
context: [panel.ubiq.file]
order: 50
status: current
related: [workbench-explorer, workbench-status-bar, workbench-file-dialogs, workbench-outline, workbench-bookmarks, workbench-panels]
---

Every file you open in IDE mode is a tab in the centre of the window, one tab active at a time. The
tab draws the file with the viewer its kind needs: a text editor for code and plain text, and
separate viewers for Markdown, Mermaid, Excalidraw, draw.io and images.

## The parts

| Part | What it does |
|---|---|
| Tab | Shows the file's name; a dot on it changes colour when the file has unsaved edits, is saving, or failed to save |
| Preview tab | A tab opened by one click in the [explorer](workbench-explorer); the next single click replaces it, and editing the file keeps it |
| Viewer layout toggle | On Markdown, Mermaid, Excalidraw and draw.io tabs: **Source**, **Preview**, **Split**, **Editor** or **Annotation**, whichever that viewer offers |
| File-kind chip | In the [status bar](workbench-status-bar): names the file's language and lists the viewers you can draw it in |
| Tab right-click menu | **Close** and its variants, **Save**, **Word Wrap**, **Reveal in Explorer**, **Pin**; see [Panels and the dock](workbench-panels) |

Double-click a file in the explorer, or hold **⇧** while you click, to open it as a tab that stays. Pick a different viewer for one tab from the file-kind chip; the choice lasts until you close
the tab. A file drawn by a viewer that has no source toggle, such as an image, is drawn alone.

Save with **⌘S** / **Ctrl+S**. Close the active tab with **⌘W** / **Ctrl+W**. **⌘N** / **Ctrl+N**
opens a new untitled file, which asks where to save it the first time. **⌘⌥K** bookmarks the line
the caret is on; see [Bookmarks](workbench-bookmarks). Pasting a picture outside a text field opens
it as an untitled image tab.

Vim keys are off until you switch them on in **Settings**, then **Editor**, then **Vim mode**; the **VIM** chip in the status bar turns
them on and off as well, and shows the mode you are in.

## What you can do here

- Find text in the project and land on its line: [Search the project](how-to-search-the-project).
- Jump to a heading or a function in the open file: [The outline](workbench-outline).
- Rename, delete or create files, and answer the questions saving raises: [File dialogs and the file picker](workbench-file-dialogs).
- Review what an agent changed in a file: [Review and commit](guides-review-and-commit).

## When something goes wrong

- **Not saved** appears with a reason: your edits are still in the tab, so fix the cause and save again.
- **File already exists** appears on save-as: pick **Overwrite** only if you mean to replace that file.
- **Unsaved changes** appears when you close a tab with edits, or reload a file that has them:
  **Discard** closes the tab without saving, and **Reload** replaces your edits with what is on disk.
- A file will not open: the tab says why, either the file is unreadable or it is not a kind Ubiq opens.
