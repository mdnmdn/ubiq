---
id: workbench-search
title: Search
summary: Finding text across a project's files from the Search panel, and opening a result at its line.
keywords: [search, find, grep, content search, regex, case, whole word, results]
context: [panel.ubiq.search]
order: 40
status: current
related: [workbench-explorer, workbench-editor, workbench-titlebar, how-to-search-the-project, settings-files-and-search]
---

The Search panel finds a word, phrase or pattern in the files of the open project. It sits in the
bottom region and shows the results grouped by file. Clicking a result opens the file in the
[editor](workbench-editor) on that line.

## The parts

| Part | What it does |
|---|---|
| Query field | Type what to find and press **Enter**; the caret is here when the panel opens |
| **Aa** | Matches upper and lower case as typed |
| **W** | Matches whole words only |
| **.\*** | Reads the query as a regular expression |
| File row | A file with its hits under it; click the name to open the file at its first hit |
| Hit row | One matching line with the match marked; click it to open the file at that line |
| Status line | The count, such as `12 files · 40 hits`, or the reason the search stopped |

A lit toggle is on. Changing a toggle applies to the next search and leaves the current results as
they are. The panel draws the first 500 hits and says `shown first 500` when the search found more;
`truncated` means the search itself stopped early, so narrow the query.

Open the panel from the title bar's **Search** button ![Search](../img/ui/titlebar-search.png), or
press **⌘⇧F**. The title bar's command field gets there too: type a term and press **⌘⏎**, or pick the
first row of the [navigator](how-to-use-the-navigator), which offers what you typed as a search
`in project`. Either way Ubiq switches to IDE mode and runs it.

## What you can do here

- Search step by step, with the options explained: [Search the project](how-to-search-the-project).
- Find a file by name instead of by contents: [The file explorer](workbench-explorer).
- Choose what every search skips: [Files and search settings](settings-files-and-search).
- Leave one folder out of search from the tree: right-click it, then **Exclude from search**.

## When something goes wrong

- The panel says `Type a query to search`: nothing has been searched yet, or the field is empty.
- `bad query:` followed by a reason: the regular expression is not valid. Turn **.\*** off or fix it.
- `the project's folder has gone`: the folder was moved or deleted. Locate it in the project picker.
- A file you expect is not in the results: search skips what the project and **Settings** exclude, and
  folders you excluded from the tree. Remove the exclusion with **Add to search** in the tree's menu.
- Results stop at 500: narrow the query, or turn on **W** or **Aa**.
- No project is open: search has nowhere to look. Open a folder first.
