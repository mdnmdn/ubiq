---
id: git-changes
title: Reviewing and staging changes
summary: What the Changes panel shows, how to stage a hunk, and what the badges mean.
keywords: [stage, unstage, hunk, diff, revert, commit]
context: [panel.ubiq.git-changes]
order: 20
status: current
related: [git-overview, git-history]
redirects: [git/staging.md]
---

## The three lists

The Changes panel groups a repository's working tree into three lists: conflicted, modified and
untracked paths, each path shown once with a `+`/`-` count of what it touches. Clicking a path opens
it in the [diff](git-overview) below the history.

## Staging

Stage a whole file or a single hunk from its row's controls; staged and unstaged changes are
tracked separately, the way `git add` and the working tree are. The commit box at the foot of the
panel shows how many paths are staged before you write a message.

## Committing

Type a message in the commit box and commit; **Amend** replaces the previous commit instead of
adding a new one. A successful commit refreshes the refs and the history together, so the graph
above never shows a commit the changes list has already cleared.
