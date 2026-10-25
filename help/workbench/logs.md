---
id: workbench-logs
title: The log console
summary: The Logs panel — reading what Ubiq has recorded about itself, filtered by part and by level.
keywords: [logs, console, debug, errors, verbose, warn, diagnostics, troubleshooting]
context: [panel.ubiq.logs]
order: 80
status: current
related: [workbench-panels, how-to-find-the-logs, troubleshooting, faq-troubleshooting]
---

The Logs panel is where Ubiq writes down what it is doing: opening a pane, starting a harness,
searching, talking to a database. Open it when something misbehaves and you want to see what the
parts said at the time. It shows the most recent 5000 records, newest last, and says how many older
ones fell off the end.

## The parts

| Part | What it does |
|---|---|
| Subsystem picker | Shows one part of Ubiq, or **All**: **UI**, **Coordinator**, **PTY**, **Harness**, **MCP**, **Search**, **Database**, **Web** or **External** |
| Level picker | Sets the quietest level shown: `TRACE`, `DEBUG`, `INFO`, `WARN` or `ERROR`; `WARN` shows warnings and errors. It starts at `DEBUG` |
| Record count | `<n> records`, with `<n> dropped` once older records have been pushed out |
| Arrow-down button | Follows the newest record as it arrives; lit while it is on |
| **Clear** | Empties the log for every window |
| Row | The time, the level in a colour, the part that wrote it and the message, on one line |

Warnings and errors are drawn in their own colour. While the Logs tab is behind another, a coloured
dot on the tab and on the panel's left edge shows the loudest thing in the log, and **Clear** removes
it. Nothing else interrupts you.

To open the panel, press the chevron beside **New terminal** in the [title bar](workbench-titlebar)
(its tooltip is **Run in a new pane**) and pick **Logs**. It opens in the bottom region, and you can
drag it anywhere like any other panel. The row is offered with no project open too.

## What you can do here

- Open it and read recent activity: [Find the logs](how-to-find-the-logs).
- Put it beside a terminal and read both at once: [Split and move panes](how-to-split-and-move-panes).
- Match a symptom to a cause: [Troubleshooting](troubleshooting).
- Read only what concerns starting agents: set the subsystem picker to **Harness**.

## When something goes wrong

- The panel says `Nothing from <part> at <level> or above.`: the filter hides everything. Lower the
  level or set the subsystem picker to **All**.
- The log has no records from before an event: it keeps 5000, and the count shows `dropped` when it
  has let go of the older ones.
- A message is cut off: each record is one line, so make the panel wider to read more of it.
- The panel is not there after you closed it: pick **Logs** from the new-pane chevron again.
- Nothing about the terminal's own output appears: what a harness prints in its pane is never logged;
  only what happened around it.
