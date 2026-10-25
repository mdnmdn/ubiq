---
id: settings-files-and-search
title: File explorer, editor and search
summary: The File explorer, Editor and Search sections — what the tree hides, how tabs close and how searching behaves.
keywords: [explorer settings, editor settings, closing a tab, search settings, hidden files, vim, preview, index, exclude, mission term]
order: 30
status: current
related: [settings, workbench-explorer, how-to-hide-or-close-a-tab]
---

Three sections of [Settings](settings) govern working with files. **File explorer** is about the
[explorer](workbench-explorer) and where cloned projects land, **Editor** about the
[editor](workbench-editor) and what a tab's **×** does, and **Search** about
[project search](workbench-search).

## The parts

| Setting | What it changes for you |
|---|---|
| **Open files in previews** | A single click opens a temporary tab that the next preview replaces; double-click, **Shift**-click and **Shift+Enter** always open permanently |
| **Show hidden files** | Shows dotfiles and dot-folders in the tree, the flat list and the filter; the eye in the explorer's header is the same switch |
| **Default project folder** | Where a clone lands unless you point the clone dialog elsewhere |
| **Ephemeral folder** | Where a throwaway clone lands, and the only place Ubiq will delete a project folder from |
| **Markdown opens in** | **Preview** or **Source** for markdown files you open from now on; source, preview and split stay available on the tab |
| **Vim mode** | Modal editing in the code editor and in every multi-line box, such as the chat composer and a task description |
| **Terminal tab**, **Agent terminal tab**, **Agent chat tab** | What the **×** does on each: **Hide** keeps it running, **Close** ends it after a confirm; shells and agent terminals start on **Close**, chat tabs on **Hide** |
| **Restore conversations** | Brings back chats that were open when Ubiq stopped, loading each one's conversation again where the harness can |
| **Mission term** | The word the board uses for a task that can have children and a plan, such as mission or epic; a project can override it |
| **Excluded paths** | Comma-separated patterns every project search skips, on top of the project's own ignore rules |
| **Keep an index** | **Off** reads every file on every search; **Full text** reads only files that could match; **Full text + symbols** also records what each file defines, which jumping to a definition needs |
| **Fallback tools** | External search tools tried in order when the built-in matcher cannot answer a pattern; empty means no fallback |

## What you can do here

- Decide whether a tab's **×** keeps its agent running or ends it: [Hide or close a tab](how-to-hide-or-close-a-tab).
- Look for text across a project: [Search the project](how-to-search-the-project).
- Override the index level, the excluded paths or the mission term for one project:
  [Project settings](settings-project).

## When something goes wrong

- **A dotfile is missing from the tree.** Turn on **Show hidden files**, or press the eye in the
  explorer's header.
- **A search misses a file.** It matches **Excluded paths** or an ignore rule of the project;
  empty the field to search everything the ignore rules allow.
- **Jump to definition finds nothing.** Set **Keep an index** to **Full text + symbols**.
- **A closed tab took its agent with it.** That tab's setting is **Close**; set it to **Hide**.
  Both endings stay on the tab's right-click menu whatever it says.
- **A pattern is refused.** `find` and `fd` are never used as fallback tools, because they match file
  names, not contents.
