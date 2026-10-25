---
id: how-to-search-the-project
title: Search the project
summary: Find text across a project's files.
keywords: [search, find, grep, regex, regular expression, content]
order: 50
related: [workbench-search, workbench-explorer]
---

Use this when you need to find where something is written in a project. Search needs an open project.

1. Pick **IDE** on the rail.
2. Press **⌘⇧F**, or the **Search** button in the title bar. The Search panel opens in the bottom region with the caret in its query field.
3. Type what to find and press **Enter**.
4. Turn on **Aa** to match case, **W** to match whole words, or **.\*** to read the query as a regular expression, and search again.
5. Click a hit to open the file on that line, or a file's name to open it at its first hit.
6. To leave a folder out of every search, right-click it in the explorer and pick **Exclude from search**.

The status line shows the count, such as `12 files · 40 hits`.

You can also type a term in the title bar's command field and press **⌘Enter**. The panel draws the first 500 hits and says `shown first 500` when there are more, so narrow the query. Search results are not ranked. [The Search panel](workbench-search) lists its messages, and [the explorer](workbench-explorer) finds files by name.
