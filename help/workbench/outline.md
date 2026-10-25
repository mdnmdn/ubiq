---
id: workbench-outline
title: The outline
summary: The Outline panel — the functions, types or headings of the open file, and jumping to one.
keywords: [outline, symbols, headings, table of contents, functions, definitions, navigate]
context: [panel.ubiq.outline]
order: 70
status: current
related: [workbench-editor, workbench-panels, workbench-bookmarks]
---

The Outline panel lists the definitions in the file you have open, in the order they appear: the
functions, types and constants of a source file, or the headings of a Markdown file. Click a row and
the [editor](workbench-editor) jumps to that line. It opens in the left region and follows the active
tab.

## The parts

| Part | What it does |
|---|---|
| Row | One definition: a short mark for its kind, its name, and its line number on the right |
| Lit row | The definition the caret is in, so you can see where you are in a long file |
| Kind mark | `fn` function, `m` method, `C` class, struct or enum, `I` interface or trait, `T` type, `mod` module, `c` constant or field, `!` macro, `p` property, `#` heading |

The panel reads the file as you edit it and refreshes a moment after you stop typing. It understands
Rust, Python, JavaScript, TypeScript, Go, Java, C, C#, Swift and Markdown.

Open it with **⌘⇧O**, which also works with the caret in a field. Closing the panel's tab removes it,
and the same key brings it back.

## What you can do here

- Move around a long file in the editor: [The editor](workbench-editor).
- Save a line you will come back to: [Bookmarks](workbench-bookmarks).
- Find a word across the project instead: [Search](workbench-search).
- Move the panel to the right or bottom: [Split and move panes](how-to-split-and-move-panes).

## When something goes wrong

- The panel says `No file open.`: open a file, and the list fills.
- It says `No outline for this file type.`: the file's language is not one the outline reads.
- It says `No definitions in this file.`: the file is one of those languages but has nothing to list.
- `No definitions in this file.` for a very large file: the outline skips files over 512 KB.
- A long file shows only part of its definitions: the list stops at 2000.
