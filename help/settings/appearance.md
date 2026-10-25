---
id: settings-appearance
title: Appearance and size
summary: The Appearance and Size sections — the theme, the theme editor, and the interface and text size presets.
keywords: [theme, colour, dark, light, size, zoom, preset, palette, accent, brightness, font]
order: 20
status: current
related: [settings, how-to-change-theme-and-size, workbench-status-bar]
---

**Appearance** sets how the window looks and which destinations the rail shows; **Size** sets how big
the window and its text are drawn. Both are sections of [Settings](settings). The **Theme** button
in the [title bar](workbench-titlebar) flips between dark and light without opening Settings.

## The parts

| Setting | What it changes for you |
|---|---|
| **Palette** | The family of neutral colours the whole window is built from |
| **Ground** | **Dark** or **Light**, within that palette |
| **Accent** | The one hue every accent-coloured element takes; the first swatch is the palette's own |
| **Text brightness** | Every text colour in the window, brighter or dimmer together |
| **Themes** | Your own palettes; **New theme…** forks the current one, and the theme editor lets you change colours, **Rename…** it or **Delete** it, with **Done** to leave |
| **Open projects in the rail** | Shows the projects this window holds as coloured badges under the mode icons |
| **Capture the window** | Turns on the title bar button and key that photograph the window into a new tab |
| **Cache token ring** | Draws a second ring beside the total-token readout, comparing cached tokens with the total |
| **Teams arrangement** | How a Teams canvas lays itself out when it first opens, in projects opened from now on |
| **Modes** | One tile per application destination, such as **Control** and **All Teams**; click one to show or hide it on the rail, and the last one on cannot be turned off |
| **Preset** | Jumps to a saved pair of sizes; four ship with Ubiq, plus any you saved |
| **Interface size** | How big the whole window is drawn: every row, icon and its text |
| **Text size** | How big text is drawn inside the window, leaving the rest as it is |
| **Saved presets** | Rename or forget a preset; the four that ship cannot be deleted, and forgetting one you saved over restores it |
| **Chrome text**, **Conversation text**, **Content text** | Nudge the text of the bars and menus, of conversations, and of the editor, explorer and terminals, each from **Smaller** to **Larger** |
| **Reset** | Puts interface and text size back to their defaults, leaving the three text nudges alone |

## What you can do here

- Switch theme, accent and size in a few steps: [Change the theme and interface size](how-to-change-theme-and-size).
- Save the current sizes as a preset from the size control at the right of the
  [status bar](workbench-status-bar), where **Save preset…** is.
- Hide the modes you do not use from the rail: [Rail modes](workbench-modes).

## When something goes wrong

- **A theme of mine looks different after an update.** A theme is a fork of a palette plus only the
  colours you changed, so it follows the palette it came from for everything else.
- **A mode is missing from the rail.** Its tile under **Modes** is off; a project's own modes are in
  [Project settings](settings-project).
- **The window is too small or too large to use.** Press **Reset** under **Size**.
- **The capture button is missing.** Turn on **Capture the window**; on Windows and X11 that switch
  is the only thing allowing a screenshot of the window.
