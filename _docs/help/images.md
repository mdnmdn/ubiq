---
id: help-images
title: Help images
kind: help
status: draft
summary: How inline button icons and screenshots for help pages are made, named, sized, themed and referenced.
read_when: you are adding an icon or a screenshot to a help page
updated: 2026-10-09
verified: 2026-10-09
code_anchors: [help/manifest.toml, _tools/helpbundle.py, _tools/icons.py]
depends_on: [wip-help, wip-help-content]
---

# Help images

This document owns inline button icons and screenshots in the manual: how they are made, named, sized, themed and referenced from a page.

## Contents

The manual carries two kinds of picture. A button icon sits inside a sentence and names a control by its shape. A screenshot shows an area of the window and sits between paragraphs. They have different sources, different sizes and different rules.

| | Button icon | Screenshot |
|---|---|---|
| Job | name a control that has no label | show a result the words cannot |
| Source | the interface's own SVG, exported by a script | a capture of the running window |
| Where | inside a sentence, a list item or a table cell | on its own line, after the step or heading it shows |
| Folder | `help/img/ui/` | `help/img/<section>/` |
| Made by | `just help-icons` | by hand |

## 1. Button icons

A page that names a control with no label gives its tooltip in bold and then the icon: "Press **Notifications** ![Notifications](../img/ui/titlebar-notifications.png) to open the list." The icon is the same drawing the button carries, so it cannot drift from the interface.

### 1.1 Source and export

- **Source.** `assets/icons/<name>.svg`, listed in `assets/icons/icons.yaml`. An icon is never redrawn for help. A glyph that does not exist is missing from the interface too, and adding it is `ubiq-icons` work.
- **Export.** `just help-icons` rasterises every drawn icon through resvg, the rasteriser the interface uses, into `help/img/ui/<name>.png`. `just help-icons <name> …` exports only the named icons. The PNGs are committed, so `just help-check` and `just help-bundle` see them like any image.
- **Regenerate** after any change to an icon, and commit the PNGs with it. Never edit a PNG by hand.
- **Name.** The icon's registry name, so `titlebar-notifications` is `help/img/ui/titlebar-notifications.png`. The registry is the list of what exists.

### 1.2 Size and tint

- **Size.** 28 by 28 pixels, which is twice the 14-pixel interface icon, on a transparent background.
- **Tint.** One colour, `#6b6b80`, the `text.muted` token of the light palette in `crates/ubiq/src/theme.rs`. The interface tints an icon with a theme token, but a PNG has one colour and the help panel follows the theme. This mid-grey keeps at least 3:1 contrast on the dark palette's ground and about 4.7:1 on the light one. The export script reads the value from `theme.rs`; a palette change moves the tint on the next `just help-icons`.

### 1.3 Referencing an icon

Use the markdown form, with the icon's tooltip as the alt text:

```
![Notifications](../img/ui/titlebar-notifications.png)
```

The path is relative to the page, so a page one folder down starts with `../img/ui/`. `just help-check` verifies that the file exists. The panel draws a markdown image on the text line, centred vertically, at three quarters of the line height, about 14 pixels, and the markdown form carries no size.

When 14 pixels is too small to tell the icon apart, write an inline `<img>` with a size:

```
<img src="../img/ui/git-stash.png" width="16" height="16" alt="Stash">
```

The panel honours `width` and `height` on an inline `<img>`. Use 16 for a dense icon and nothing above 18, or the line grows. `just help-check` does not read HTML, so a mistyped `src` passes unnoticed; open the page after writing it. Use the HTML form only where the markdown size fails, because only the markdown form is checked.

An icon sits in a sentence, a list item or a table cell, next to the bold name of the control. It never stands alone in a cell, which draws as an empty row, and never alone in a paragraph, which draws as a full-width block.

## 2. Screenshots

A screenshot goes where the result of a step is not obvious, in a guide's step list or under the heading of the area it shows. A page that explains an area in words and a table needs none.

### 2.1 What to capture

- **Format.** PNG at 2×, so the pixel size is twice the size on screen.
- **Theme.** Dark, unless the page is about theming.
- **Window.** The default window size, on a default layout.
- **Content.** The demo project from `_data/config`. No personal path, account name, email, token or host name is on screen.
- **Crop.** To the area the page is about. A full-window capture appears only on `index` and on a guide's first step.

### 2.2 Name, alt text and reference

Name a screenshot `help/img/<section>/<page-id>[-<n>].png`, where `<section>` is the page's folder and `<page-id>` its id without the folder. A page with several screenshots numbers them from 1 in reading order: `git/changes-1.png`. Reference it with markdown, on its own line:

```
![The Changes panel with two staged paths and one unstaged](../img/git/changes-1.png)
```

The alt text says what the picture shows, in a sentence a reader who cannot see it could use. "Screenshot of the panel" says nothing.

### 2.3 Taking one

On macOS, run `screencapture -o -l <window-id> out.png`, which captures the window without its shadow. Crop afterwards.

On Linux the app runs headless. Start `Xvfb :99`, set `XDG_RUNTIME_DIR`, run `target/debug/ubiq` with `DISPLAY=:99`, drive it with `xdotool`, and capture with `import -window root out.png`. The full recipe, with the packages it needs, is in [`wip/help-content.md`](../wip/help-content.md) under finding T0.1.

### 2.4 Keeping them current

A screenshot goes stale when the screen changes. [`coverage`](./coverage.md) lists, in its Screenshot column, the page that carries a shot of each area, so a change to an area finds the pages to re-shoot. Add the page to that column in the same change that adds the screenshot, and update the column when the screenshot goes.

## Related docs

- [`wip/help.md`](../wip/help.md) — how the help machinery works
- [`wip/help-content.md`](../wip/help-content.md) — the programme that writes the manual
