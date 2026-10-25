---
id: panes
title: Panes and panels
summary: What a panel is, what a pane is, and why every agent gets a real terminal.
keywords: [terminal, tab, focus, dock, split, resize, shell]
order: 20
status: current
related: [workbench-panels, how-to-hide-or-close-a-tab, how-to-split-and-move-panes, glossary]
---

## What it is

Everything in the window with a tab is a *panel*: a file, the explorer, the logs, this help, a chat. You
drag a panel's tab to move it, split it beside another, or group it with one. Ubiq fixes no arrangement,
and [Panels and the dock](workbench-panels) shows where each kind of panel may sit.

A *pane* is the panel that holds one terminal. Each agent you start in a terminal, and each shell, runs
in its own pane. The tab shows the agent's name and a dot that says whether its harness is still
running. Right-click the tab and pick **Rename…** to type over the title.

Exactly one panel has *focus* and receives your keystrokes. Panes that do not have it keep drawing, so
an agent you are not typing into stays visible. Press **⇧Esc** / **Shift+Esc** to hand the keyboard back to the
window without closing anything; click the pane to take it again.

## Starting and ending a pane

The **+** at the end of a pane's tab strip opens your default shell in the current project. The
chevron beside it lists the panes you hid, under **Detached**, then every shell on your machine, then
**Logs**. An agent starts from the [new agent dialog](agents-new-agent) instead. A pane always starts
in its project's folder, so with no project open there is nothing to start.

Typing `exit` or pressing **Ctrl+D** inside a pane ends its harness and closes its tab.

A tab's **×** does what your **Closing a tab** setting says. **Hide** takes the tab away and
leaves the harness running; reopening it from the chevron's **Detached** group brings the same screen
back mid-sentence. **Close** ends the harness after a confirmation. The tab's right-click menu
always offers both — see [Hide or close a tab](how-to-hide-or-close-a-tab).

## Why it works this way

Harnesses draw the whole screen, move the cursor around and read every key, so they only look and
behave as intended in a real terminal. That is why an agent gets a pane rather than a chat box
([Real terminals](philosophy-real-terminals) says more).

Moving a pane never restarts its harness, and resizing one tells the harness, which redraws at the
new size. Switching to another project leaves that project's panes running behind the scenes.
