---
id: feedback
title: Sending feedback
summary: The feedback dialog — reporting a bug or suggesting a feature, with an optional picture of the window, and what is sent.
keywords: [feedback, bug report, suggestion, report a problem, screenshot, annotate, send]
order: 160
status: current
related: [workbench-titlebar, troubleshooting, workbench-logs]
---

**Send feedback** ![Send feedback](../img/ui/board-comment.png) in the [title bar](workbench-titlebar)
opens a dialog for telling the Ubiq team about a bug or an idea. It works with or without a project
open. Nothing is captured or sent until you press **Send**.

## The parts

| Part | What it does |
|---|---|
| **Title** | One line naming the report; **Send** waits until it is filled |
| **Type** | **Bug**, **Feature** or **Eco feature** |
| **Description** | What you did, what you expected, and what happened instead |
| **Take screenshot** | Hides the dialog, photographs the window, and brings the dialog back with the picture attached; offered only where the system allows it |
| **Paste image** | Attaches the picture on the clipboard; **⌘V** / **Ctrl+V** does the same while the caret is not in a text field |
| Picture preview | Shows what will be sent, with **Edit** to annotate or crop it and a **×** to remove it |
| Destination line | At the foot of the dialog: says where the report goes before you send it |
| **Cancel**, **Send** | Close without sending, or send the report |

A picture is optional, and a report without one is valid. **Edit** opens the same drawing tools
an image tab has, in a dialog over the form; **Apply** replaces the picture with your edit, and
**Cancel** or **Esc** drops the edit and keeps the original. The title, type and description are kept
while you take or edit a picture.

When the report goes through, the foot of the dialog becomes a receipt, `Sent` with a reference where
there is one. An **Open** button appears if the report has a page, and **Done** closes the dialog.

## What you can do here

- Find the button beside the bell and help: [The title bar](workbench-titlebar).
- Read what Ubiq recorded before you report a problem: [The log console](workbench-logs).
- Check a known symptom first: [Troubleshooting](troubleshooting).
- Look up where reports go: **Settings**, then **Feedback**.

## When something goes wrong

- **Send** is dimmed: the title is empty, a report is already on its way, or this build has no
  destination. The line under the button says which.
- **Take screenshot** is missing: this system does not allow Ubiq to photograph its window. Paste an
  image instead.
- **Take screenshot** brings back no picture: permission was refused or the capture failed, and a
  short note says so. Allow screen capture for Ubiq in the system's settings, or paste an image.
- **Paste image** says the clipboard has no image: copy a picture first, then paste.
- You want the report to go somewhere else: the destination is set under **Settings**, then
  **Feedback**.
