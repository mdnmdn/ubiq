---
id: projects
title: Projects
summary: A folder Ubiq knows about — the home every rail mode other than Control works against.
keywords: [project, folder, workspace root, open project]
order: 30
status: current
related: [sessions, workbench-modes]
---

## A project is a folder

Opening a project points Ubiq at a folder on disk. Everything a rail mode other than Control or the
kitchen sink shows — the file explorer, the Git screen, the knowledge base, the agents you start —
is scoped to that folder. Ubiq remembers which projects you have opened, and a window can hold more
than one at a time, switching between them with the badges under the rail.

## What a project remembers

Which files are open, how the panels are arranged in each rail mode, the terminals still running, and
the pinned files and unsaved buffers you left behind — all kept per project, so returning to one
restores the window the way you left it rather than a blank slate.

## Closing versus forgetting

Closing a project's window leaves the project itself in Ubiq's list, ready to reopen. Removing it
from that list — "forgetting" it — touches nothing on disk; it only stops Ubiq offering it back.
