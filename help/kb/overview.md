---
id: kb-overview
title: The knowledge base
summary: KB mode on the rail — a project's documents, read from folders, repositories and wikis that need not be inside the project.
keywords: [knowledge base, kb, documents, wiki, sources, markdown, notes]
context: [rail.kb, panel.ubiq.kb.explorer]
targets: [ui.rail.mode.kb]
order: 10
status: current
related: [kb-documents, kb-sources, guides-knowledge-base, workbench-explorer]
---

Pick **KB** on the rail to read a project's documents: markdown, plain text, images and diagrams. They
come from *sources* — a folder, a git repository Ubiq keeps up to date, or a wiki Ubiq keeps for the
project — and none of them has to sit inside the project's own folder. The agents Ubiq starts for the
project can read and write the same documents.

The left panel is titled **Documents**. It lists every source's files in one tree, the way the
[file explorer](workbench-explorer) lists a project's folder. Click a document to open it in the
centre, as [Reading a document](kb-documents) describes.

## The parts

| Part | What it does |
|---|---|
| The gear in the panel header | Opens the project's settings on **Knowledge base**, where sources are managed |
| **Add KB** | Shown while the project has no sources; opens the same settings section |
| A source row | The source's name; hover for where it comes from |
| `pending`, `failed`, `locked` | A source that is not ready: still to be fetched, unreachable, or a wiki waiting for its password |
| A folder or file row | Click a folder to open or fold it, a file to open it |

Right-click a row for its menu. A source offers the file manager (named for your system) and
**Copy path**, **Get latest version** on a repository, and **Rename source**; **New file** and
**New folder** appear where the source can be written to. A wiki that has a password adds **Unlock**
or **Change password**. A folder or file offers the file manager and **Copy path**, plus **New file**,
**New folder**, **Rename** and **Delete** in a source you can write to.

## What you can do here

- Point the project at its documents: [Knowledge base sources](kb-sources).
- Read and edit a document: [Reading a document](kb-documents).
- Give an agent documents to work from: [the knowledge base guide](guides-knowledge-base).

## When something goes wrong

- **A source says `failed`.** A folder source has lost its folder, or a repository could not be
  fetched. Click the row to try again, or fix the source in the project's settings.
- **A repository looks out of date.** It refreshes only when you ask: right-click the source and
  pick **Get latest version**.
- **A source says `locked`.** Click it and enter the wiki's password.
- **New file, Rename and Delete are missing.** The source is read-only; change its access in
  [Knowledge base sources](kb-sources).
- **A change that could not be made** is shown on the document, or on the source's row when no
  document is involved.
- **Delete asks first**, and for a folder it says that everything inside goes too.
