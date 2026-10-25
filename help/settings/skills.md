---
id: settings-skills
title: Skills
summary: The Skills section, in Settings and in Project settings — the skills an agent can be given, and where Ubiq finds them.
keywords: [skills, skill, SKILL.md, install, repository, folder, source, agent skills]
order: 45
status: current
related: [settings, agents-definitions, settings-mcp-servers, settings-project]
---

A skill is a folder with a `SKILL.md` that tells an agent when and how to use it. This section is the
list Ubiq knows about; it gives nothing to an agent by itself. An [agent definition](agents-definitions)
or the [new agent dialog](agents-new-agent) then ticks the skills that agent should carry.

**Skills** appears twice. In [Settings](settings) it is the application's list, offered to every
project. In [Project settings](settings-project) it is a list only agents started in that project can
carry. A skill there with the same id as an application skill takes its place.

## The parts

| Part | What it does |
|---|---|
| **Add skill folder…** | Adds one skill: pick the folder that holds a `SKILL.md`; the row says **Linked** and the skill stays where it is |
| **Add skills directory…** | Scans a folder of skills and lists each one it finds, marked **Folder**; the folder appears under **Scanned folders** |
| **Browse repositories…** | Opens **Browse skill repositories**: a search box, **All** or one source, and **Install** beside a result; an installed skill is marked **Installed** |
| A skill row | Its name, a badge (**Installed**, **Linked** or **Folder**), its description, and **Remove** |
| **Scanned folders** | The folders you added with **Add skills directory…**, each with **Remove**, which takes its skills away too |
| **Sources** | Settings only. The repositories the browser searches, each with **Remove** |
| **Add source…** | Settings only. Adds a source: **Repository** (the clone address) and **Path**, to look under one folder of it only |
| **From the application** | Project settings only. The application's skills, read-only; one this project replaces is marked as shadowed |

## What you can do here

- Give a skill to an agent: tick it under **Skills** in the [agent definition](agents-definitions) or the
  [new agent dialog](agents-new-agent).
- Keep a skill to one project: add it in that project's **Skills** instead of in Settings.

## When something goes wrong

- **A skill from a scanned folder has no Remove.** Remove the folder under **Scanned folders**.
- **The browser finds nothing.** **No skills found** means the chosen source holds none under its
  path; try **All** or add a source.
- **A red banner sits above the list.** Ubiq reports why the last change was refused; correct it and
  try again.
