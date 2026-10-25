---
id: projects
title: Projects
summary: A folder Ubiq has opened, and everything it remembers about it.
keywords: [project, folder, open, forget, locate, clone, temporary]
order: 10
status: current
related: [panes, sessions, workbench-all-projects, workbench-modes]
---

## What it is

A *project* is a folder Ubiq has opened, together with everything Ubiq remembers about it. The
explorer, the Git screen, the knowledge base and every agent you start all work against the project
you are in. An agent always starts in its project's folder.

Ubiq keeps a list of every project you have opened. The project picker in the titlebar groups it
three ways: projects open in this window, projects open in another window (with that window's
letter), and history. You can add a folder, clone a repository, or open a project on a remote
machine from the picker. Dropping a folder onto the editor opens it at once as a temporary project.

A project is open in one window at a time. Picking one that another window holds moves it, and its
agents keep running.

## What a project remembers

Which files you had open and how its panels were arranged come back when you return to it. Several
projects can be open in one window; switching between them swaps the explorer, the open files and the
panes, and the panes of the projects behind keep running.

## Closing, forgetting and a missing folder

Closing a project ends its terminals and unloads its conversations, and Ubiq asks first when any are
open or a file is unsaved. A conversation you marked persistent can be resumed later; see
[Conversations](concepts-conversations).

**Forget** removes the project from Ubiq's list and everything Ubiq remembers about it. It never
touches the folder on disk. If a project's folder has moved or been unplugged, its row shows the path
as a warning and offers **Locate**, which points the project at the new folder and keeps its colour
and history. Ubiq never removes a project because its folder went missing.

## Why it works this way

A project is the unit that keeps work apart: its files, its agents and its layout stay together, so
switching between clients or repositories does not mix them up. Forgetting is separate from deleting
because the folder is yours.
