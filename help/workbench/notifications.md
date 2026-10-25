---
id: workbench-notifications
title: Notifications
summary: The bell and its list — what Ubiq tells you while you are elsewhere, and how to silence a source for a while.
keywords: [notifications, bell, mute, silence, alerts, do not disturb, unread, clear]
order: 100
status: current
related: [workbench-titlebar, agents-starting, workbench-panels]
---

Ubiq runs several things at once and you are looking at one of them. When another has something to
say, such as a turn that ended, a permission an agent is waiting on or a clone that failed, it raises
a notification. The **Notifications** bell ![Notifications](../img/ui/titlebar-notifications.png) in
the [title bar](workbench-titlebar) counts the unread ones and opens the list.

## The parts

| Part | What it does |
|---|---|
| Bell and count | The number is how many notifications are unread; past 99 it reads `99+` |
| Flash | A new notification flashes the bell for a few seconds in the colour of its level, information, warning or error |
| Row | The part of Ubiq that raised it, the agent or kind of event if known, how long ago, and the text; an unread row has a dot |
| Check button on a row | Marks that notification read; it is absent once read |
| Silence button on a row | Starts a silence for that notification's narrowest source; its tooltip reads **Silence** and the source |
| Bin button on a row | Removes that notification from the list |
| **Mute everything** | Starts a silence for every source |
| **Mark all read** | Clears the count and keeps the rows |
| **Clear all** | Removes every row |
| **Silenced** | Lists each silence in force, with the level and when it ends; the eye button, **Turn this back on**, lifts it |

A row that carries a link takes you there when you click it, and marks itself read. While the bell is
flashing for a notification with a link, clicking the bell goes straight to that place and skips the
list; its tooltip names the place. The list holds the 200 most recent, newest first, and **Esc**
closes it.

To silence a source, press its silence button. A panel asks two things: the level, **Info**,
**Warning** or **Error**, so that the silence covers that level and everything quieter, and how long:
**5 minutes**, **15 minutes**, **1 hour**, **8 hours** or **Until I turn it back on**. A silence on
**Warning** still lets errors through. Silencing a source again replaces its earlier silence.

A silenced notification is still filed, still in the list and still counted. It does not flash the
bell or reach the desktop.

## What you can do here

- Start an agent whose turns will notify you: [The new agent dialog](agents-new-agent).
- Find the bell and the other title bar actions: [The title bar](workbench-titlebar).
- Read what Ubiq recorded about a failure: [The log console](workbench-logs).

## When something goes wrong

- The bell never flashes for an agent: open the list and look for its source under **Silenced**,
  then press **Turn this back on**.
- The list says `Nothing has come up.`: nothing is waiting, or **Clear all** removed it.
- The count stays after you read everything: press **Mark all read**.
