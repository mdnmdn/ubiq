---
id: troubleshooting
title: Troubleshooting
summary: What to check when a pane will not start, a resize looks wrong, or the help panel itself is empty.
keywords: [problem, broken, not working, help missing]
order: 900
status: draft
related: [panes, agents-starting]
---

## An agent will not start

Check the account it is meant to run under in [Accounts](agents-accounts) — a harness that cannot
authenticate exits immediately, and its pane's tab shows that it stopped rather than staying blank.

## A pane looks corrupted after a resize

This should not happen; Ubiq propagates every resize to the harness underneath. If a pane's drawing
still looks wrong after resizing its region, closing and reopening that pane's tab reattaches the
same running harness to a fresh terminal view.

## This help panel is showing a stub

If the `?` button opens a page saying help content is not built, the bundle described in this
manual has not been packaged for this build — that is expected in a development checkout that has
not run the packer, and nothing else is broken.
