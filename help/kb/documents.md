---
id: kb-documents
title: Reading a document
summary: Open a knowledge base document in its own tab, read it beside your work, and save edits where the source allows.
keywords: [document, read, markdown, edit, save, kb, tab]
context: [panel.ubiq.kb.doc]
order: 20
status: current
related: [kb-overview, kb-sources, workbench-panels]
---

A document you open from the [knowledge base](kb-overview) gets a tab of its own, so you can keep
several open at once, each with the dock's usual tab strip: a dot for unsaved edits, drag to move,
pin and close. With no document open, the centre says **No document open** and points you to the
**Documents** panel.

A document opens in the same viewer the editor uses for a project file: markdown is rendered, text and
code are highlighted, images are shown, and diagrams open in their own editor. A picture from the
knowledge base cannot be annotated.

## The parts

| Part | What it does |
|---|---|
| The tab | The document's name; hover for its path in the source |
| The dot on the tab | Unsaved edits |
| **Close** | On the tab's right-click menu; closes the tab |
| **Reveal in Explorer** | On the tab's menu; finds the document in the **Documents** tree |
| **Save** | On the tab's menu; writes the document back to its source |
| Pin row | On the tab's menu; pins or unpins the tab |

## What you can do here

- Edit a document in a source that allows writing, then save it with **⌘S** in KB mode, which saves
  every document with unsaved edits, or with **Save** on the tab's menu.
- Find where an open document lives: **Reveal in Explorer**.
- Create, rename or delete documents from the tree's right-click menu: [the knowledge base](kb-overview).
- Arrange tabs beside an agent: [panels](workbench-panels).

## When something goes wrong

- **Save does nothing.** The source is read-only, so its documents open for reading and highlighting
  but cannot be written back. Change the source's access in [Knowledge base sources](kb-sources).
- **A write failed.** The reason appears on the document.
- **The tab closed by itself.** Its source was removed from the project.
- **A document looks stale.** A repository source refreshes only on request: right-click the source
  and pick **Get latest version**.
