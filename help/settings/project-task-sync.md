---
id: settings-project-task-sync
title: Task sync
summary: The Task sync section of Project settings — binding the project's board to a board kept at a tracker, and how changes flow between them.
keywords: [task sync, trello, board, bind, import, push, pull, two-way, lanes, tracker, drift, interval]
order: 92
status: current
related: [settings-project, tasks-overview, settings-connectors]
---

**Task sync** binds this project's [task board](tasks-overview) to a board kept somewhere else, so
that its cards can be brought in as tasks and kept in step. Everything under the provider is drawn
from what that provider needs. It appears once the folder is kept as a project, and beside its name
the section shows how many tasks are linked.

## The parts

| Part | What it changes for you |
|---|---|
| **Provider** | What answers for this board: **Not bound**, or a provider the build offers (Trello) |
| **Connection** | Which signed-in identity this project calls the tracker as; the binding keeps a reference, never the token |
| **Board** | The board at the provider that this project is bound to |
| **Filter** | The provider's own fields (for Trello **Board**, **Lists**, **Labels**, **Members**) that decide which cards are offered, with a **Test** that counts the matches |
| **Lanes** | For each lane at the provider, which of Ubiq's lanes its cards land in; **—** leaves the cards where they are |
| **Kinds and priorities** | A word the provider uses, mapped to one of Ubiq's; its × undoes a mapping |
| **Direction** | **Pull only**, the default, or **Two-way**, which also writes your changes back |
| **When both sides changed** | **The remote wins** or **Ubiq wins**, for a field changed on both sides; the loser's value is kept in a comment |
| **Interval** | Seconds between passes, with a floor that the page names when it applies |
| **Import everything the filter matches** | Off by default; on, every match becomes a task without being picked |
| **What this provider accepts** | Chips for what two-way can write, such as **Push the title** or **Push comments**; a dim chip is something this tracker does not do |
| **What differs** | Shown only when tasks differ: each field with both values, **Push** or **Pull** per row and **Push all** or **Pull all** |
| **Import…** | Lists what the filter offers; you tick the cards to bring in, and nothing arrives unpicked |
| **Unbind** | Removes the binding |
| **Save binding** | Keeps your choices; the button reads **Saved** when nothing is pending |

## What you can do here

- Look at the board the cards land on: [The Tasks board](tasks-overview).
- Add the identity first: [Connectors](settings-connectors).

## When something goes wrong

- **Connection says No connection.** No signed-in identity is held that suits the provider; nothing
  syncs until one is picked.
- **Save binding is replaced by Fill every required field to save.** A required field, such as
  **Board**, is empty.
- **Two-way is not offered.** The page says **Pull only** and that the provider accepts no write.
- **A card I imported was not changed on the board.** Check **Lanes**: a lane left at **—** parks its
  tasks where they are.
- **A row under What differs says Stuck.** The provider will not take that write; no setting closes it.
