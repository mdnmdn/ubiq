---
id: concepts-harnesses
title: Harnesses
summary: The agent programs Ubiq hosts, how an agent differs from its harness, and what each one can reach.
keywords: [claude code, codex, gemini cli, opencode, copilot, cli, agent, sandbox, confined]
order: 60
status: current
related: [agents-starting, settings-harnesses, philosophy-any-harness, panes]
---

## What it is

A *harness* is the agent program itself: Claude Code, Codex, Gemini CLI, opencode or GitHub Copilot
CLI. Ubiq hosts harnesses and does not replace any of them. An *agent* is one harness running in a
project, doing work for you, so two Claude Code agents in two projects are two agents of one harness.

You start an agent from the [new agent dialog](agents-new-agent), in one of two ways: **Start** runs it as a
[conversation](concepts-conversations), and **Start in terminal** runs it in a terminal
[pane](panes). A harness whose program is not installed on your machine is listed greyed out, so you
can see that it is missing rather than wonder where it went.

An agent runs confined unless your settings say otherwise: it can reach the project's folder and its
own working area, and the rest of your machine is closed to it. [Harnesses and
isolation](settings-harnesses) lists the options.

Ubiq never picks a model for you. Each harness brings its own models, and you choose among them per
agent.

## Why it works this way

Harnesses change quickly and differ in what they are good at. Hosting them side by side, each with its
own terminal and its own account, lets you use whichever suits the task and compare them on the same
code. [Any harness](philosophy-any-harness) explains the thinking.
