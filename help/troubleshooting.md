---
id: troubleshooting
title: Troubleshooting
summary: Symptom, cause and fix for the problems people meet most — a harness that will not start, a stuck agent, a pane that looks wrong.
keywords: [problem, broken, not working, error, stuck, help missing, not found, sign in, login, expired, blank, garbled, resize]
order: 900
status: current
related: [faq-troubleshooting, how-to-find-the-logs, workbench-logs]
---

Find the heading that matches what you see. Each one says why it happens and how to get past it, and links the page that explains the area in full. Short answers in question form are in [Troubleshooting questions](faq-troubleshooting).

## New agent and New terminal are missing

No project is open, and an agent or a shell always runs in a project's folder. Press **Add a project** in the title bar and choose a folder; [Getting started](getting-started) walks through it.

## A harness is dimmed, or cannot be chosen

Ubiq cannot find the harness's program on this machine.

1. Install the harness the way its own documentation describes.
2. If it is installed somewhere Ubiq does not look, open **Settings**, then **Harnesses**, and set its **Command** to where it lives. **Check** beside it tries the command without saving.

[Harnesses and isolation](settings-harnesses) describes the section.

## A harness is missing from the start menus

A tool with both a native and a registry version offers only the native one until you switch the other on. Open **Settings**, then **Harnesses**, and turn on its **Enable … ACP** switch. See [Harnesses and isolation](settings-harnesses).

## An agent fails to start

The harness stopped before it could answer, often because it could not sign in. The conversation shows the error and a **Retry**.

1. Read the error in the transcript.
2. If it is about signing in, renew the account as described in the next heading.
3. Press **Retry**, or send a prompt, which retries too.

[Inside a conversation](agents-conversation) has the conversation's own controls.

## A sign-in has expired or fails

The login an account points at has run out, or never finished.

1. Open **Settings**, then **Harnesses**, and find the account under **Identities**.
2. Press **Check** on the harness row to read the login's state.
3. Press **Re-authenticate** and finish the harness's own sign-in in the terminal inside the dialog.

Closing the dialog during a sign-in abandons it; start again. On macOS, **Check** can say `missing` for a login kept in the keychain while the agent still works. See [Accounts](agents-accounts).

A GitHub or GitLab connection that says expired is a connector, not an account: disconnect it and connect again in [Connectors](settings-connectors).

## An agent seems stuck

It is often waiting for your answer to a request that has scrolled out of view.

1. Look for the **NEEDS YOU** strip above the conversation's footer, and click its label to jump to the question.
2. If nothing is waiting, press **Stop** in the composer.
3. If **Stop** gets no answer, use **Abort** from the conversation's three-dots menu, then **Resume**.

See [Permission modes](agents-permissions) and [Inside a conversation](agents-conversation).

## An agent has gone from the screen

Its tab was hidden, and the agent is still running.

- A terminal pane: open the chevron beside **+** on a tab strip and pick it under **Detached**.
- A conversation: click its row in the sidebar of [Agents mode](agents-starting), or open **Agents not on screen** beside **New agent** in the title bar.

## Closing a tab ended the agent

That kind of tab is set to **Close**. To keep agents running when you close their tabs, open **Settings**, then **Editor**, then **Closing a tab**, and set it to **Hide**. See [Hide or close a tab](how-to-hide-or-close-a-tab).

## A pane looks wrong after a resize

Ubiq passes every new size to the harness, and the harness redraws itself at that size. A harness that has not redrawn yet catches up when its size changes again: drag the divider beside the pane a little. Moving a pane never restarts its harness. See [Panes and panels](panes).

## A pane is blank after its project moved to another window

The harness keeps running, but what it had already drawn does not come across with the project. A full-screen harness redraws as soon as the pane is resized, so drag the divider beside it; a shell shows only what it prints from then on. See [Projects](projects).

## A tab came back empty after a restart

Only an agent marked persistent comes back on its own. Another conversation reopens by itself when restoring is switched on, and otherwise waits for you to restore it. See [The chat tab](agents-chat) and [Conversations](concepts-conversations).

## A panel or a region has gone

Closing a panel's tab removes it, and a region you close stays closed. Press that region's button in the title bar to open it again with the mode's own panel. See [Panels and the dock](workbench-panels).

## The help panel has no page to show

- **It says help is not built for this binary.** This build does not carry the manual; release builds do.
- **F1 opens the welcome page.** The screen you were on has no page of its own. Pick its area from **Contents**.
- **A page says it is not written yet.** Pick another from **Contents**.
- **Pointing at an area shows nothing.** That area has no name. Point at a control inside it.

See [The help panel](workbench-help).

## A remote host has stopped answering

Its row stays in the project list while Ubiq retries. Open **Remote hosts** from the **More** menu beside **Settings**, press **Test** to see why, then **Connect** or **Reconnect now**. See [Remote hosts](remote-overview).

## Something else is wrong

Open the log console: the chevron beside **New terminal**, then **Logs**. Set the level picker to `WARN` to see only warnings and errors. See [Find the logs](how-to-find-the-logs).
