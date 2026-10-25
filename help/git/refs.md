---
id: git-refs
title: The Branches panel
summary: The panel of branches, remotes, tags, stashes and submodules, and how picking one moves the history.
keywords: [branch, tag, remote, ref, checkout, merge, delete, stash, submodule]
context: [panel.ubiq.git.refs]
order: 40
status: current
related: [git-overview, git-history]
---

The panel on the left of [Git mode](git-overview) is titled **Branches** and lists the repository's
refs. Use it to find a branch or tag, jump the [history](git-history) to it, or switch to it.

## The parts

| Part | What it does |
|---|---|
| Search field | Narrows every section to names that match, and opens the folders that hold a match |
| **Repositories** | Only in a project with several repositories; pick one and every panel follows it |
| **Local branches** | Open on first visit; the current branch first, then `main`, `master`, `develop` and `dev`, then the rest alphabetically |
| **Remotes**, **Tags**, **Stashes**, **Submodules** | Start shut; click a heading to open or fold it |
| A row's marks | **↑** ahead and **↓** behind against the upstream; on a repository, a dot for changes and **!** for conflicts |

Names with `/` nest as folders. Click a local or remote branch to jump the history to its tip, or
double-click to check it out. Clicking a tag, stash or submodule scrolls the history to its commit.

Right-click a row for **Checkout**, **Merge**, **Delete** and **Copy name**. **Merge** and **Delete**
work on local branches and tags only, and **Merge** is unavailable on the checked-out branch and
while HEAD is detached. Stashes and submodules offer only **Copy name**.

## What you can do here

- Switch branch: double-click it, or select it and use **Checkout the selected ref** in the
  [Git strip](git-overview).
- Create a branch at HEAD with **New branch** in the strip.
- Find a commit by branch: [Browsing history](git-history).

## When something goes wrong

- **A checkout asks to discard changes.** Git refused because uncommitted changes would be
  overwritten; confirm to discard them, or cancel and commit or stash first.
- **Delete asks first**, and a branch that is not fully merged takes its own commits with it.
- **A search shows no section** when nothing in it matches; clear the field to see them again.
