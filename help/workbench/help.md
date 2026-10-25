---
id: workbench-help
title: The help panel
summary: How to open the manual inside the window, find the page for the screen you are on, and point at a control for its one-line help.
keywords: [help, manual, F1, point at, contents, follow mode, documentation, balloon]
context: [panel.ubiq.help]
order: 130
status: current
related: [workbench-titlebar, getting-started, workbench-panels, troubleshooting]
---

The help panel is this manual, inside the window. Press **F1** on any screen and it opens on the page
for that screen, in the right region, without leaving your work. **Point at something** is the other
way in: it tells you about one control instead of the whole screen.

## The parts

| Part | What it does |
|---|---|
| **Help** ![Help](../img/ui/titlebar-help.png) | In the [title bar](workbench-titlebar), and **F1**: opens the page for where you are |
| **Point at something** ![Point at something](../img/ui/titlebar-point-at.png) | **⇧F1**: waits for you to point at a control and says what it is |
| **Back**, **Forward** | Step through the pages you have read, as a browser does |
| **Contents** ![Contents](../img/ui/view-tree.png) | Shows or hides the list of every page, grouped in folds; click a page to open it |
| Page title | Names the page you are reading |
| **Follow mode** | When ticked, the panel switches page as you change mode or screen |
| Links | A link to another page opens it in the panel |

A page marked as a draft carries a note at the top saying it may trail the build you are running. A
page listed greyed in **Contents** has not been written, and opens a notice saying so.

**Point at something** draws a box around whatever the pointer is over and a balloon beside it with the control's name and one or two sentences. Move the pointer to read another.
Clicks do nothing in this mode, so you can point at a button without pressing it. Press **Esc** or
**Done** to leave. A control Ubiq has named but not yet described says `This place has a name but no
description yet.`; an unnamed area shows nothing.

The title bar's **More** menu, beside **Settings**, has **Help** and **Point at something…** too.

## What you can do here

- Find your way around the title bar and its buttons: [The title bar](workbench-titlebar).
- Make the panel follow you from screen to screen: tick **Follow mode**.
- Put the panel somewhere else, or beside another: [Split and move panes](how-to-split-and-move-panes).
- Start from the beginning: [Getting started](getting-started).

## When something goes wrong

- The panel says help is not built for this binary: the manual is not part of this build, and release
  builds carry it.
- The page does not change as you move around: **Follow mode** is off. Tick it.
- A page says it is not written yet: pick another from **Contents**.
- Pointing at an area shows nothing: that area has no name yet. Try the control inside it.
