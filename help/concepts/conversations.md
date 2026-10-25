---
id: concepts-conversations
title: Conversations
summary: One harness's exchange with you, which you can stop, resume, fork or keep across a restart.
keywords: [session, resume, fork, history, persistent, transcript, chat]
order: 40
status: current
related: [agents-chat, agents-columns, how-to-resume-or-fork-a-conversation, panes]
---

## What it is

A *conversation* is one harness's exchange with you: your messages, its replies and the work it did in
between. You read it as a transcript in a chat tab or in a column of Agents mode, and you answer from
the composer under it. A harness started in a terminal pane shows the same work as its own screen
instead; see [Panes and panels](panes).

A conversation is more than its tab. A chat tab's **×** hides it and the agent keeps going; the
sidebar of [Agents mode](agents-columns) still lists it, and one click brings it back. To end a conversation, open the three-dots
menu at the top left of the conversation and pick **Close**, which asks first because it deletes the
transcript.

The three-dots menu also holds the verbs for a conversation's life:

- **Stop** interrupts the turn in progress and leaves the harness running.
- **Abort** and **Unload** shut the harness down and keep the conversation; **Resume** starts it again
  where it left off.
- **Fork** copies the conversation and continues the copy as a second agent, leaving the first
  untouched.
- **Make persistent** keeps the conversation across a restart of Ubiq, for harnesses that keep their
  own history, such as Claude Code and Codex.

A harness that stops by itself, through a crash or an error, leaves its conversation unloaded, so
**Resume** and **Close** still work.

## Why it works this way

The conversation is the thing you care about, and a tab or a process is only how you see it. Keeping
them apart lets you tidy the window without losing an agent's work, and lets you resume or branch a
conversation when you want a second try. [Resume or fork a conversation](how-to-resume-or-fork-a-conversation)
has the steps.
