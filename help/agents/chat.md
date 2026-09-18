---
id: agents-chat
title: The chat tab
summary: A read-only perspective on a conversation, shown beside the code rather than in another window.
keywords: [chat, conversation, transcript, tab]
context: [view.chat]
order: 40
status: current
related: [agents-starting]
---

## What a chat tab shows

A harness in a terminal shows what an agent is doing; a chat tab shows what it was asked and what it
concluded, rendered beside the code rather than in another window. It appears in IDE mode, Tasks and
the Teams screens — anywhere a conversation is part of the work being done.

## Many tabs, one conversation

A chat tab is a perspective on a conversation the host owns, not a conversation of its own. Several
tabs can look at the same run, each attached to a different point in its history or to none at all;
closing one ends nothing. The relation holds in one direction only — deleting the underlying
conversation closes every tab that was looking at it, because there is then nothing left to show.
