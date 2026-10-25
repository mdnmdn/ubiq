---
id: how-to-find-the-logs
title: Find the logs
summary: Open the log console to see what Ubiq has been doing.
keywords: [logs, console, debug, errors, diagnostics]
order: 90
related: [workbench-logs, troubleshooting]
---

Use this when something misbehaves and you want to see what Ubiq recorded.

1. Press the chevron beside **New terminal** in the title bar. Its tooltip is **Run in a new pane**.
2. Pick **Logs**. The panel opens in the bottom region; you can drag it anywhere.
3. To read one part of Ubiq, set the subsystem picker, for example to **Harness** for anything about starting agents.
4. To see more or less, set the level picker. It starts at `DEBUG`, and `WARN` shows warnings and errors.
5. To keep up with new records, light the arrow-down button, which follows the newest.
6. To start fresh before you reproduce a problem, press **Clear**.

Each record is one line: the time, the level, the part that wrote it and the message.

The panel keeps the most recent 5000 records and says how many older ones were dropped. The log is not kept after Ubiq quits, and what a harness prints in its own pane is never in it. [The log console](workbench-logs) explains each control, and [Troubleshooting](troubleshooting) matches symptoms to causes.
