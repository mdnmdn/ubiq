---
id: philosophy-real-terminals
title: Real terminals
summary: Why every agent gets a real terminal rather than a chat box, and what a chat tab adds beside it.
keywords: [terminal, tui, chat, transcript, screen, full screen, log]
order: 20
status: current
related: [panes, agents-chat, philosophy-why-ubiq]
---

## What it is

Every agent in Ubiq can run in a real terminal [pane](panes). The harness runs there exactly as it
would in your own terminal: its colours, its full-screen layout, its menus and every key you press,
including arrows, Ctrl and Alt chords, paste and the mouse.

The same run can instead start as a conversation in a [chat tab](agents-chat): a transcript of what
the agent was asked and what it concluded, with its tool calls drawn as blocks and a composer
underneath. You allow or deny what the agent asks to do in the transcript, answer its questions as
a form, and attach files and images to a message.

## Why it works this way

The harnesses Ubiq runs are full-screen programs. They take over the whole screen, move the cursor
anywhere on it and redraw when its size changes. Shown as a scrolling log, they would make no
sense, because they are not writing a log — they are drawing a screen. A real terminal shows each
harness the way its makers meant it to be seen.

That is also why a resize reaches the harness: when you drag a pane's border, the harness redraws
at the new size instead of drawing for the old one. And why typing `exit` or pressing **Ctrl+D**
closes the tab — the harness has ended, so its pane ends with it.

A terminal shows what an agent is doing now; a chat tab is for reading back what it did. The
transcript sits beside your code, lets you answer prompts without hunting through a screen, and
shows what the run has spent in its footer. You can open as many chat tabs as you like and move
them anywhere in the window. A chat tab's **×** hides it rather than ending the agent, because the
conversation outlives the tab.

Pick the terminal when you want the harness exactly as it is, and the chat tab when you want to
read, answer and attach. [The new agent dialog](agents-new-agent) is where you choose.
