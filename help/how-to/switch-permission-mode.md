---
id: how-to-switch-permission-mode
title: Switch a harness's permission mode
summary: Change how much an agent may do before asking you.
keywords: [permission, approval, autonomy, plan mode, accept all, yolo]
order: 10
related: [agents-permissions, agents-starting]
---

Use this when you want an agent to ask more or less often. Each harness names its modes itself, and Ubiq shows those names.

1. To start an agent on a mode, press **New agent**, choose what to run, and pick the **Mode** row's value. **The harness's own** leaves the choice to the harness.
2. To change the mode of an agent already on screen, open the mode chip beside **Send** under its composer, and pick another. Its tooltip reads the mode's name and `permission mode`.
3. To see which mode an agent is on, open the three-dots menu at the top left of its conversation and pick **Info**.
4. To stop being asked in one conversation, pick **Accept all** from the same menu. It reads **Stop accepting all** while on; pick it again to be asked once more.
5. To make every start from a saved setup use one mode, set the mode in its [agent definition](agents-definitions).

The mode chip shows the new mode, and **Info** names it.

**Accept all** answers every permission for that conversation, so the transcript can hold tool calls you were never asked about. [Permission modes](agents-permissions) covers answering a prompt with **⌘⌥Y** and **⌘⌥N**.
