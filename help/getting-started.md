---
id: getting-started
title: Getting started
summary: From the first launch to one agent working in one project — open a folder as a project, start an agent, and know where to go next.
keywords: [setup, first run, first launch, open project, add folder, new agent, welcome, onboarding]
order: 10
status: current
related: [guides-first-project-and-agent, projects, agents-new-agent, workbench-titlebar]
---

This page takes you from the first launch to one agent working in one project. [Your first project and your first agent](guides-first-project-and-agent) walks the same ground in more detail.

## What you see on first launch

Ubiq opens one window with no project in it. The centre says **No project open**, explains that Ubiq works in a project, and offers one button, **Add a project…**. The project picker in the title bar says **No projects yet.**, and the status bar reads `no project`.

Nearly everything in Ubiq works against a [project](projects): a folder on your machine that its agents run in. **New agent** and **New terminal** appear in the title bar once a project is open.

## Open a project

1. Press **Add a project…** in the centre of the window, or the **Add a project** button in the [title bar](workbench-titlebar).
2. Choose the folder you want to work in, and press **Add**.
3. Project settings opens on **General**, titled **New project**. Check the **Project name**, pick a **Color** if you like, and press **Create**.

The project opens in this window, with the [rail](workbench-rail) down the left edge and the project's name in the title bar. Ubiq remembers it, so the project picker offers it on every later launch.

To start from a repository instead of a local folder, open the chevron beside **Add a project** and pick **Clone a project…**; [Clone a project](how-to-clone-a-project) walks through it. **Open remote project…** in the same menu reaches a folder on another machine; [Remote hosts](remote-overview) explains it.

## Start your first agent

Install the harness you want to use, and sign it in, the way its own documentation describes; Ubiq starts the program that is already on your machine.

1. Press **New agent** in the title bar. The [new agent dialog](agents-new-agent) opens.
2. Open the **Harness** tab and pick your harness. A **Default** row runs it with its own configuration, exactly as it runs in a terminal. A dimmed row is a harness Ubiq cannot find on this machine.
3. Leave the model and the permission mode as they are, or change them. Type a first task in **Initial prompt** if you want the agent to start working at once.
4. Press **Start**.

The agent appears as a conversation: a column in [Agents mode](agents-starting), or a chat tab in the right region on other screens. Type into its composer to talk to it, and answer its questions as they come. **Start in terminal** runs the same harness in a terminal [pane](panes) instead, where you type exactly as you would in any terminal.

## Find your way around

- The rail switches what the centre of the window is for: **IDE** for files, **Git** for the repository, **Agents** for your agents, and the rest; see [Rail modes](workbench-modes).
- Every panel has a tab you drag to move it or split it beside another; see [Panels and the dock](workbench-panels).
- Press **F1** on any screen for its page in this manual, and **⇧F1** to point at a single control; see [The help panel](workbench-help).

## Where to go next

- [Your first project and your first agent](guides-first-project-and-agent) — the same first run, step by step.
- [Harnesses](concepts-harnesses) and [Conversations](concepts-conversations) — what you started, and what you can do with it later.
- [Accounts](agents-accounts) — run a harness under more than one identity.
- [Troubleshooting](troubleshooting) — when a harness will not start or something looks wrong.
