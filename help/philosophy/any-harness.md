---
id: philosophy-any-harness
title: Any harness
summary: Which agent harnesses Ubiq runs side by side, and why the choice of harness and model stays with you.
keywords: [claude code, codex, gemini, gemini cli, opencode, copilot, copilot cli, model, provider, vendor]
order: 30
status: current
related: [agents-starting, settings-harnesses, philosophy-why-ubiq]
---

## What it is

Ubiq runs five harnesses: Claude Code, Codex, Gemini CLI, opencode and GitHub Copilot CLI. You can
run any of them side by side in one window, on the same project or on different ones, and compare
how two harnesses handle the same task.

Each harness runs unchanged. Ubiq does not stand in for it or talk to a model in its place: the
harness you start is the one doing the work, with its own commands, its own screen and its own
[permission modes](agents-permissions), shown under their own labels.

When you [start an agent](agents-starting), you pick the harness, the account it signs in as, the
model, the effort level and the permission mode. An agent definition saves a harness, a model,
skills and MCP servers as a named setup, so you can start the same setup again in one pick.

## Why it works this way

Harnesses and models change fast, and the best one for a task today may not be the best tomorrow.
Ubiq leaves that call with you. It adds the window, the layout and the controls around a harness,
and leaves the harness itself, and the model behind it, to your choice.

Each harness also keeps its settings in its own place and its own way. Ubiq gives you one set of
controls over all of them, and each run starts with its own throwaway configuration. Your real
harness settings stay as you left them, so the harness works the same when you run it outside
Ubiq.

[Harnesses and isolation](settings-harnesses) lists the harnesses available to you and their
accounts.
