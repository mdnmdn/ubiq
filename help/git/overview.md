---
id: git-overview
title: The Git screen
summary: Git mode on the rail — branches, history and uncommitted changes of a project's repository, and the strip of actions above them.
keywords: [git, repository, branch, commit, pull, push, fetch, stash, undo]
context: [rail.git]
targets: [ui.rail.mode.git]
order: 10
status: current
related: [git-changes, git-history, git-refs, git-diff, guides-review-and-commit]
---

Pick **Git** on the rail to read a project's repository without leaving the window your agents work
in. The first visit puts **Branches** on the left, **History** in the centre and **Changes** on the
right. **Diff** is a tab beside **History** and comes forward when you pick a changed path. Chat tabs
open in the right region too, so an open chat tab shares its group with **Changes**. Every panel can be
moved, hidden or resized like any other, as [panels](workbench-panels) explain.

## The parts

| Part | What it does |
|---|---|
| [**Branches**](git-refs) | Local branches, remotes, tags, stashes and submodules; picking one moves the history to it |
| [**History**](git-history) | The commit graph, searchable, with a row on top for what is not committed yet |
| [**Changes**](git-changes) | The conflicted, staged and unstaged paths, and the commit box |
| [**Diff**](git-diff) | What the picked path or commit changed |
| The branch pill | The current branch, its **↑** ahead and **↓** behind counts, and a merge or rebase in progress |
| **Checkout the selected ref** ![Checkout the selected ref](../img/ui/git-branch.png) | Checks out the ref selected in **Branches**; faint until one is |
| **Fetch all** ![Fetch all](../img/ui/git-fetch.png), **Pull**, **Push** | Fetch, pull and push, over `https` or `ssh` remotes |
| **New branch** ![New branch](../img/ui/git-branch-new.png) | Asks for a name, then **Create** makes the branch at HEAD and checks it out |
| **Stash changes** ![Stash changes](../img/ui/git-stash.png) | Shelves the uncommitted changes, untracked files included |
| **Undo last commit** | Takes the last commit back into the staged list; refused on a first commit |
| **Refresh** | Reads the repository again |
| The count and status words | How many paths changed, or what Ubiq is waiting on: **Staging…**, **Committing…**, **Loading history…** |

## What you can do here

- Read what an agent changed, stage what you keep and commit it: [Review and commit](guides-review-and-commit).
- Stage one file: [Stage a file](how-to-stage-a-file).
- Compare two commits: [Comparing two commits](git-compare-commits).
- Move the panels around: [Split and move panes](how-to-split-and-move-panes).

## When something goes wrong

- **Not a repository** — the project's folder holds no git repository, so the lists stay empty and
  the strip's buttons are faint, apart from **Refresh**.
- **A project with several repositories** shows one at a time. **Branches** then opens with a
  **Repositories** section; pick a row there and every panel follows it.
- **Branch, stash and undo are refused** while a merge or rebase is in progress. The reason is
  printed under the commit box.
- **A checkout that would overwrite uncommitted changes** asks before it discards them.
- **Nothing is rewritten without asking.** Discarding a change, a hard reset, deleting a ref and
  reverting a file to a commit each ask first.
