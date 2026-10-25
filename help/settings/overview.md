---
id: settings
title: Settings
summary: The Settings window and the sections it is divided into.
keywords: [settings, preferences, options, configure]
order: 10
status: current
related: [settings-appearance, workbench-titlebar, settings-project]
---

Settings holds everything Ubiq keeps for the whole application rather than for one project. Open it
with the **Settings** button ![Settings](../img/ui/titlebar-settings.png) in the [title bar](workbench-titlebar).
A list of sections runs down the left and the section you pick fills the right. Close it with the
close button at the top right, or by clicking outside it. Switches and pickers take effect as you
change them, and a form that adds something has its own **Save**.

What belongs to one project, such as its name, colour and task board, is in
[Project settings](settings-project), opened from the project's own **Project settings** button.

## The parts

| Section | What it holds |
|---|---|
| **Appearance**, **Size** | Theme, accent, text brightness, window and text size; see [Appearance and size](settings-appearance) |
| **File explorer**, **Editor**, **Search** | How files open, how tabs close, what searches skip; see [File explorer, editor and search](settings-files-and-search) |
| **Harnesses**, **Isolation** | The harnesses on this machine, the identities they sign in as, and confining agents; see [Harnesses and isolation](settings-harnesses) |
| **Agent definitions** | Saved setups an agent starts from; see [Agent definitions](agents-definitions) |
| **Skills**, **MCP servers** | The skills and tool servers an agent definition or a start can give an agent; see [Skills](settings-skills) and [MCP servers](settings-mcp-servers) |
| **Assistance** | The model that names conversations and drafts short text; see [Assistance](settings-assist) |
| **Connectors** | Your signed-in identities at GitHub, GitLab and similar; see [Connectors](settings-connectors) |
| **Hosts**, **SSH profiles**, **Drones** | Other machines, how to sign in to them, and what runs on them; see [Hosts, SSH and drones](settings-hosts) |
| **Tools**, **Command line** | Commands you can run from the title bar, and the `ubiq` launcher; see [Tools and command line](settings-tools) |
| **Feedback** | Where a report from the feedback dialog is sent; see [Sending feedback](feedback) |
| **Updates** | The version running, the release channel, and whether Ubiq checks and downloads on its own, with **Check now**; see [Updates](settings-updates) |
| **Diagrams** | How diagrams are checked and drawn: **Quality profile**, **Palette** and **Motion**; see [Diagrams and shell integration](settings-diagrams) |
| **Shell integration** | On Windows only: adds *Open in Ubiq* to the file manager's right-click menu; see [Diagrams and shell integration](settings-diagrams) |

## What you can do here

- Change the look and size of the window: [Change the theme and interface size](how-to-change-theme-and-size).
- Add a GitHub or GitLab identity: [Sign in a connector](how-to-sign-in-a-connector).
- Add an account for a harness: [Accounts](agents-accounts).
- Reach another machine: [Connecting to a host](remote-connect).

## When something goes wrong

- **A section shows an error banner.** Ubiq reports the reason it was refused at the top of
  Harnesses, Assistance and Connectors; correct the entry and try again.
- **A control is dimmed.** The section says why: for example confinement is available on macOS and
  Windows only.
- **I cannot find a project's setting here.** It is in [Project settings](settings-project).
