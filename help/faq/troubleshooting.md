---
id: faq-troubleshooting
title: Troubleshooting questions
summary: Quick answers to the problems people meet most, each pointing at the full fix.
keywords: [problem, error, not working, stuck, missing]
order: 40
related: [troubleshooting, workbench-logs]
---

## An agent will not start. What should I check?

Check the account it runs under; a harness that cannot sign in exits at once, and its tab shows that it stopped. A harness drawn dimmed in the start menus is not on this machine, so install it or set its **Command** in **Settings**, then **Harnesses**. See [Troubleshooting](troubleshooting).

## New agent and New terminal are missing or do nothing. Why?

Both need an open project. Open a folder with **Add a project** in the title bar. See [The title bar](workbench-titlebar).

## My agent seems stuck. What do I do?

It may be waiting on a request scrolled out of view; the **NEEDS YOU** strip above the footer catches that. Otherwise press **Stop**, and if nothing answers, use **Abort** and then **Resume**. See [Inside a conversation](agents-conversation).

## My search does not find a file I know is there.

Search skips what the project and **Settings** exclude, and any folder you excluded from the explorer's tree. Remove an exclusion with **Add to search** in the tree's menu. See [The Search panel](workbench-search).

## Why is Commit greyed out?

It needs a message and at least one staged path, or **amend** ticked. See [The Changes panel](git-changes).

## A closed tab took its agent with it.

That tab kind's setting is **Close**. Set it to **Hide** under **Settings**, then **Editor**, then **Closing a tab**. See [Hide or close a tab](how-to-hide-or-close-a-tab).

## The bell never flashes for an agent.

Its source may be silenced. Open the list, look under **Silenced**, and press **Turn this back on**. See [Notifications](workbench-notifications).

## Where can I see what Ubiq has been doing?

In the Logs panel, opened from the chevron beside **New terminal**. It is not kept after Ubiq quits. See [Find the logs](how-to-find-the-logs).
