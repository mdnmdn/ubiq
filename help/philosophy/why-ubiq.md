---
id: philosophy-why-ubiq
title: Why Ubiq
summary: Why several coding agents belong in one window, and what Ubiq gives you beyond a row of terminal tabs.
keywords: [why, tmux, multiplexer, agents, window, side by side, parallel]
order: 10
status: current
related: [philosophy-real-terminals, getting-started, guides-two-agents-side-by-side]
---

## What it is

Ubiq is one window that runs several AI coding agents side by side, each in its own
[pane](panes), next to the project's files, history, documents, database and task board. Think of a
terminal multiplexer such as tmux, with every pane made for an agent rather than a shell.

You start Claude Code in one pane and Codex in another, watch both, and type into whichever needs
you. Exactly one pane has focus and receives your keystrokes; the others keep drawing, so you can
read one agent while another thinks.

A [workspace session](sessions) gathers the agents for one piece of work under a name and a folder.
Close the window and come back tomorrow: the arrangement is still there, and you do not start
every agent again by hand.

## Why it works this way

Ubiq is built for the moment you are watching agents work and stepping in — reading one pane while
another runs, redirecting a run that has gone sideways, comparing two harnesses on the same task.
That is why what an agent shows you on screen matters more here than a tidy transcript, and why
one agent failing never takes its neighbours down with it.

A terminal and a shell already run agents. Ubiq adds what they leave to you:

- **Several agents at once, with one in focus.** Each tab shows which agent runs there and what
  state it is in, so you know without reading its output.
- **A unit of work that lasts.** A workspace session keeps its agents and layout between visits,
  and you can leave one without ending anything.
- **Runs put together for the task.** You choose the account, skills and MCP servers an agent
  starts with, instead of inheriting whatever is installed on the machine.
- **One place to set them all up.** Every harness keeps its settings its own way; Ubiq gives you
  one set of controls over all of them.

[Getting started](getting-started) takes you from launch to a first agent at work, and
[Two agents side by side](guides-two-agents-side-by-side) shows the case Ubiq is made for.
