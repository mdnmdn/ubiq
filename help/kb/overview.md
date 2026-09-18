---
id: kb-overview
title: The knowledge base
summary: A project's documents, read from one or more sources that need not be inside the project.
keywords: [knowledge base, kb, documents, wiki, sources]
context: [rail.kb, panel.ubiq.kb-explorer]
order: 10
status: current
related: [workbench-explorer]
---

## More than one source

The knowledge base is the documents half of a project — markdown, plain text, and eventually
diagrams and images — read from **sources** that need not live inside the project's own folder at
all. A source is a folder you point at, a git repository Ubiq clones and keeps in sync, or an
internal wiki Ubiq keeps for the project itself. Each has its own name, its own read-only or
read-write access, and its own filter of which files it shows.

## The explorer

The knowledge base's explorer lists every source's documents in one tree, the same way the file
explorer does for a project's own folder, and opens a document into the centre when you pick one.
Sources with several kinds mixed together still show as one tree — you pick a document, not a
source.

## Keeping a git source in sync

A git source clones once and refreshes on request: fetching and resetting to whatever branch is
actually checked out, so a source you point at a different branch later starts a fresh checkout
rather than fighting the old one.

This page covers what a knowledge base source is; the write path — creating, renaming and deleting a
document from the explorer's own menu — is still being written up.
