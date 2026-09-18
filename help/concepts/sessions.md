---
id: sessions
title: Sessions and workspaces
summary: A session is a named piece of work with a home folder; a workspace is one running agent inside it.
keywords: [session, workspace, attach, detach]
order: 20
status: current
related: [panes, projects]
---

## A session is a place to put your agents

A developer running several agents at once needs somewhere to put them. A **session** is that
place — a named piece of work with a home folder, holding the agents serving it. You attach to a
session to see and drive its workspaces, and detach without ending anything: the agents keep running
whether or not you are looking at them.

## A workspace is one agent

A **workspace** is one running instance of one agent inside a session: which harness, which working
directory, how big its terminal is, and whether the process is still alive. A session holds many
workspaces; each owns exactly one pseudo-terminal and one child process, drawn as one [pane](panes).

## Why the distinction matters

Closing the window does not lose the arrangement, and reopening Ubiq does not mean respawning every
agent by hand — the session remembers what was running, and you pick up where you left off.
