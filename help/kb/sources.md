---
id: kb-sources
title: Knowledge base sources
summary: Add, filter, protect and remove the folders, repositories and wikis a project's documents are read from.
keywords: [source, folder, repository, wiki, add source, clone, branch, password, filter]
order: 30
status: current
related: [kb-overview, kb-documents, settings-project, guides-knowledge-base]
---

Sources are managed in the project's settings, under **Knowledge base**. Reach them from the gear
in the **Documents** panel's header, or with **Add KB** while the project has none. The section lists
every source, with **Add source** under them.

## The parts

**Add source** opens a form with these rows. Rows that do not apply to the chosen kind are inert.

| Row | What it does |
|---|---|
| **Name** | What the **Documents** panel calls the source; filled in from the folder or repository until you type your own |
| **Kind** | **Folder** is read where it lies, **Git repository** is cloned and refreshed by Ubiq, **Wiki** is Ubiq's own and starts empty |
| **Folder** | Browse for the folder; it is chosen on the machine the project runs on, not necessarily this one |
| **Repository URL** and **Check** | Check reaches the repository and reports **Reached** with its branch count |
| **Branch** | Which branch the checkout follows; **The repository's default** unless the check offers others |
| **Store** | Where a repository is kept: **Cache**, **Ubiq's project folder** (the default) or **Inside the project** |
| **Access** | **Read only** or **Read and write**; a wiki is always writable |
| **Password protected** | Wiki only: encrypts every document under a password, fixed once saved |
| **Cancel**, **Add source** | Discard the form, or add the source |

In the list, each source also has a filter field of space-separated patterns such as `*.md *.excalidraw`;
only matching files are shown, and it applies when you press **Enter** or leave the field. The
**×** at the end of a row removes the source.

## What you can do here

- Add a folder of notes, a docs repository or a wiki: use **Add source** above.
- Keep a repository current: right-click it in **Documents** and pick **Get latest version**.
- Protect a wiki: tick **Password protected**, then set the password the first time you open it.
  The unlock window offers **Remember in keychain**, on by default, and a locked wiki has **Change
  password** on its row.
- Walk through it with an agent: [the knowledge base guide](guides-knowledge-base).

## When something goes wrong

- **Check does not report Reached.** Correct the repository URL, which must be an `https://`
  address, and check again.
- **A source shows `failed`.** Its folder is gone or its fetch failed; fix the source, then click it.
- **A forgotten wiki password cannot be recovered.** Ubiq says so before you set it.
- **A wiki's password setting cannot be changed back** once the source is saved.
