---
id: workbench-explorer
title: The file explorer
summary: The project's file tree, in IDE mode's left region — open, rename, create and delete from it.
keywords: [explorer, file tree, sidebar, rename, delete]
context: [panel.ubiq.explorer]
order: 30
status: current
related: [workbench-editor, workbench-search]
---

## The tree

The explorer draws the project's folder as a tree in IDE mode's left region, open the moment IDE
mode is entered rather than something you have to summon. Clicking a file opens it in the
[editor](workbench-editor); right-clicking a file or folder offers rename, delete, reveal and
creating a new file or folder alongside it.

## Filtering

A filter field narrows the tree to matching names as you type, without changing what is on disk. It
remembers what you last typed per project, so returning to a large tree does not mean re-filtering
it.

## Following changes

The explorer watches the folder it is showing, so a file created or removed outside Ubiq — by the
agent running in a pane beside it, or by a command in another terminal — appears or disappears on
its own.
