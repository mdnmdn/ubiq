---
id: workbench-bookmarks
title: Bookmarks
summary: Write down a place — a line in a file, or another screen — with one key, and return to it from the explorer or the navigator.
keywords: [bookmark, favourite, saved place, pin, mark line, adrift, remember]
order: 140
status: current
related: [workbench-explorer, workbench-editor, how-to-use-the-navigator]
---

A bookmark is a place you wrote down so you can go back to it. Press **⌘⌥K** with the caret on a line
and that line is bookmarked; press it again on the same place and the bookmark goes. Bookmarks belong
to the project, so each project keeps its own list.

## The parts

| Part | What it does |
|---|---|
| **⌘⌥K** | Adds a bookmark at your current place, or removes it if it is already there |
| **Bookmarks** | A fold-out list at the head of the [explorer](workbench-explorer), above the filter, with a count; it appears once the project has a bookmark |
| Row | A bookmark's name, taken from its place; click it to go there |
| **adrift** | A warning tag on a row whose line Ubiq can no longer find |
| Highlight and count | A bookmarked line is lit in the [editor](workbench-editor), and the file's tab shows how many bookmarks it holds |

A bookmark on a line remembers the line's text, not only its number. After you edit the file above
it, Ubiq looks for that text a couple of hundred lines either way and moves the bookmark with it. When
the text is gone, the bookmark turns **adrift**. Ubiq leaves it where it was and does not guess.

The [navigator](how-to-use-the-navigator) lists bookmarks under **Bookmarks**, after **Recent**, so
**⌘K** and a few letters of a name takes you to one without opening the explorer.

## What you can do here

- Jump to a bookmark from anywhere with the keyboard: [Use the navigator](how-to-use-the-navigator).
- Keep your place in a long file while you read another: [The editor](workbench-editor).
- Find a name in a long file instead: [The outline](workbench-outline).

## When something goes wrong

- **⌘⌥K** does nothing: the screen you are on is not a place Ubiq can write down, such as a chat tab.
- A row is tagged **adrift**: the line it pointed at was changed or removed. Click it to open the
  file at the old line number, and press **⌘⌥K** on the line you meant to bookmark.
- The **Bookmarks** list is missing from the explorer: the project has no bookmarks yet.
- A bookmark is gone after you forgot the project: bookmarks go with it.
