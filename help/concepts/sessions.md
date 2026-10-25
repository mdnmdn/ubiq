---
id: sessions
title: Workspace sessions
summary: A workspace session is Ubiq's named grouping of agents with a home folder — and how it differs from a conversation.
keywords: [session, group, attach, detach, running agents]
order: 30
status: current
related: [panes, projects, concepts-conversations]
---

## What it is

A *workspace session* is a named grouping of agents with a home folder. It is the place where the
agents serving one piece of work live. Creating one starts nothing; you start agents into it, and
you can have several workspace sessions at once, independent of each other.

You see a workspace session by attaching to it, and you leave it by detaching. Detaching ends
nothing: its agents keep running whether or not you are looking at them, and attaching again finds
them where they were.

Closing a workspace session closes its agents. An agent that exits on its own leaves the workspace
session in place, ready for another agent.

## Not a conversation

The word *session* also appears in harnesses' own vocabulary, where it means a resumable exchange.
Ubiq calls that a *conversation*, and keeps the two apart: a workspace session holds agents, a
conversation is what one agent has said with you. See [Conversations](concepts-conversations).

An agent's working folder is its [project's](projects) folder. The workspace session's home folder is
only a fallback, and with no project open Ubiq starts no agent at all.

## Why it works this way

Running several agents on one task needs somewhere to put them that does not depend on which tab is
on screen. The grouping gives each piece of work a place that outlives any single agent in it.

A workspace session lasts while Ubiq is running. What comes back after a restart is a conversation you
marked persistent, not the grouping around it.
