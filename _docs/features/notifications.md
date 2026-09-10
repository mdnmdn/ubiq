---
id: feat-notifications
title: Notifications
kind: feature
status: draft
summary: One bell in the titlebar over a host-owned history — a level, an origin and an optional link per notification, a badge that counts the unread, a flash that carries a click straight to where it points, and mute rules by scope, level and duration that also decide what the desktop hears.
read_when: you are raising a notification from a subsystem, changing the bell, the notification list or a mute rule, or wiring an event that should reach the user without a screen open
updated: 2026-09-10
verified: 2026-09-10
code_anchors: [crates/ubiq-proto/src/notifications.rs, crates/ubiq-host/src/notifications/mod.rs, crates/ubiq-host/src/notifications/os.rs, crates/ubiq/src/state/notifications.rs, crates/ubiq/src/app/notifications.rs, crates/ubiq/src/app/wire.rs, crates/ubiq/src/ui/notifications.rs, crates/ubiq/src/ui/titlebar.rs]
depends_on: [tech-architecture, tech-transport, feat-workbench]
review_cycle: monthly
---

# Notifications

## Purpose

Ubiq runs several agents at once and the user is looking at one of them. A turn that ended, a
permission a harness is waiting on, a clone that failed — each happens on a screen nobody is
watching, and without somewhere to put it, the only way to find out is to go and look.

The bell in the titlebar is that somewhere. Anything in Ubiq can raise a notification; the bell
counts what has not been read, flashes when something arrives that the user has not silenced, and
opens onto the list. A notification can carry a link, so it is not only a report but a way back to
what it is about. And because a notification the user does not want is worse than no notification,
every one of them is addressed — a family, an actor, a category — so it can be silenced at exactly
the width the user meant.

## Behaviour

### What one is

A notification has a **level** (`Info`, `Warning`, `Error`), an **origin**, some **text** and an
optional **link**.

The origin is three fields, widest first:

| Field | What it names | Example |
|---|---|---|
| `family` | Which part of Ubiq raised it. A closed set | `Agents`, `Terminal`, `Git` |
| `actor` | Who inside that family. Optional | a specific agent |
| `category` | What kind of event. Optional | `"tool call"` |

The origin is not decoration: **it is the addressing scheme a mute rule matches on**, which is why
`family` is a closed enum rather than a string. A menu lists the families, and a rule that named one
by spelling would be a rule that silences nothing.

The level decides the colour — `Info` is `info()`, `Warning` is `warning()`, `Error` is `danger()`.
There is no icon per level: the bell is the icon, and status is colour, the same rule every other
screen follows.

### Raising one

Two flags travel with a request and **both default to `false`**.

- **`muted`** — arrive quietly. The badge counts it; the bell does not flash.
- **`os`** — also tell the operating system. This is the one thing here that leaves Ubiq, so
  nothing gets it by accident.

A raiser builds a `NotificationRequest` and hands it over; it does not decide anything else. The
id, the timestamp, whether it ended up silenced, and who is told are all the host's.

### Muted, and what muting is for

**Muting is about interruption, never about hiding.** A muted notification is still filed, still
appears in the list, and still counts against the badge. What it does not do is flash the bell or
reach the desktop.

`Notification.muted` is the host's verdict, not the raiser's wish: it is the request's own `muted`
OR'd with every rule in force.

### Mute rules

A rule is a **scope**, a **level ceiling** and a **duration**.

| Scope | Silences |
|---|---|
| `All` | everything |
| `Family` | one family |
| `Actor` | one actor inside one family |
| `Category` | one category inside one family |

The ceiling silences everything **at or below** it, so muting a scope at `Warning` still lets its
errors through, and muting at `Error` silences the lot. Durations are a closed set — 5 minutes, 15
minutes, 1 hour, 8 hours, or until it is turned back on — because both halves have to agree on what
"15 minutes" means.

**One rule per scope.** Muting a scope that already has a rule replaces it, which is what makes
"mute this agent for an hour" idempotent from a menu. A rule whose moment has passed is dropped
rather than kept as an inert row, so the list the user reads is the list that is in force.

The list offers the **narrowest scope a row supports** — its category, else its actor, else its
family — because that is nearly always the one meant, and everything wider is one more click.

### The bell

- **The badge is the unread count.** Read and dismissed are different things: reading clears the
  badge and keeps the row; dismissing removes it.
- **An unmuted arrival flashes the bell for three seconds**, blinking in the level's colour. A
  muted one only moves the number.
- **Clicking a flashing bell that carries a link goes straight there** and does not open the list.
  That is the whole point of the flash: the notification that just arrived is the one the user is
  reacting to, so the click belongs to it rather than to the list behind it.
- Clicking it otherwise opens the list. Escape closes it.

### The list

Newest first, capped at 200. Each row shows its level as colour, its origin, its text, how long ago
it arrived, and whether it is unread or was silenced. A row with a link follows it when clicked, and
marks itself read on the way. Per row: tick read, dismiss, mute. Across the list: mark all read,
clear all, mute everything. The rules in force are listed above the notifications with a control
that lifts each one.

### The operating system

`notify-rust` is the whole implementation — the Freedesktop D-Bus service on Linux, macOS's own
notification centre, a WinRT toast on Windows. Ubiq writes no platform code.

A desktop with no notification service is **ordinary, not a fault**: it is a debug line, and the
bell in the window carries the same text either way. A notification reaches the desktop only when
the request asked for it and no rule silenced the result, and it is sent **once** however many
windows are open — which is the reason the rules live in the host and not in a window.

## Contract

The [notification family](../tech/transport-contract.md#the-notification-family), which owns every
message fact here. Six variants UI → host — `RaiseNotification`, `ListNotifications`,
`ReadNotifications`, `DismissNotifications`, `MuteNotifications`, `UnmuteNotifications` — and two
host → UI, `NotificationRaised` and `NotificationsState`, **both broadcast**: the bell is in every
window, and two badges that disagree is the bug broadcasting rules out by construction.

The records — `Notification`, `NotificationRequest`, `UbiqLink`, `MuteScope`, `MuteRule`, `MuteFor`,
`Notifications` — are listed in the same document and defined in
`crates/ubiq-proto/src/notifications.rs`.

## Implementation

**The contract** — `crates/ubiq-proto/src/notifications.rs`. The records above, plus the two
predicates the whole feature turns on: `MuteScope::covers` (does this scope address that origin)
and `MuteRule::silences` (does this rule silence that notification, now). Both are pure and both
are tested here rather than in the host, because they are the contract's own arithmetic.

**The host** — `crates/ubiq-host/src/notifications/mod.rs` holds `Centre`, the one place a
notification is filed: `raise`, `list`, `read`, `dismiss`, `mute`, `unmute`. It holds no bus and
answers in `Reply`s, the same shape the catalogue uses, which is what makes it testable without a
host. `Coordinator` owns one and dispatches the six variants to it.

`crates/ubiq-host/src/notifications/os.rs` is the one path out. `post` spawns a thread and returns:
showing a desktop notification is a round trip to a service, and the coordinator's thread is the
thread that drains every pseudo-terminal — blocking it to draw a toast would stall a harness.

**The interface** — `crates/ubiq/src/state/notifications.rs` mirrors the host's state and adds what
only a window has: whether the list is open, when the flash ends, and what the flashing notification
points at. It never edits the host's half: every change is a message out and a fresh state back,
because two windows drawing from their own edits drift and two redrawing from one broadcast cannot.

`crates/ubiq/src/app/notifications.rs` holds the behaviour — the bell click and its two outcomes,
the read/dismiss/mute sends, `follow_notification_link`, and `flash_bell`. An arrival mid-flash
pushes the deadline out rather than starting a second timer, because two loops flipping the same
flag cancel each other and the bell sits still.

`crates/ubiq/src/ui/notifications.rs` draws the list, and `crates/ubiq/src/ui/titlebar.rs` draws the
bell: `bell()` is a helper of its own rather than a bare `kit::icon_button`, on the precedent
`nav_control` sets in the same file, because a count over the glyph and an alternating tint do not
fit the kit's square button. The badge is drawn over the button so the strip's spacing does not
shift when a count appears. The icon bundle has no `BellOff`, so a mute control is `Eye`/`EyeOff`.
`ui/shell.rs` mounts the list, and `AppState::cancel_dialog` has two rungs for it — the picker
before the list it is drawn in.

`crates/ubiq/src/ui/sink/style.rs` has a Notifications group in the kitchen sink that raises one of
each shape. `G198` narrows to the two producers still missing: a pane exiting non-zero, and a clone
that failed.

**Raising one from anywhere** is `AppState::raise_notification(NotificationRequest)` in the
interface and a `Centre::raise` call in the host. Nothing raises a notification by drawing one. The
`ConversationUpdate` arm in `crates/ubiq/src/app/wire.rs` is the first real caller: it raises a
warning, `Family::Agents`, when a conversation wants permission — any conversation, a delegate's
own included, since the ask still blocks a turn somebody has to answer — and an info when the main
agent's own turn ends, narrowed away from a delegate's by the work record's `parent`, since a
delegate's turns fold into the transcript rather than closing with their own `TurnEnded`. Both
carry an `UbiqLink::Agent`.

**A permission ask is the one that is never skipped, and the one that leaves the window.** The turn
ending is not raised where something on screen is already drawing that conversation — it reports
something that has already happened, and the surface has said it. An ask is a question that blocks
the turn until somebody answers it, and a surface drawing it may be behind another window, on
another screen, or scrolled away from the prompt: being on screen is not being read. So it is
raised whatever is drawn, and it is the only real producer that sets `os`, so the desktop is told
as well as the bell. A mute rule on `Agents · permission` is how somebody who does not want that
turns it off.

## Failure

- **The desktop refuses, or has no notification service.** A debug line. The bell has the same text.
- **A link names something that has gone** — a pane that closed, a project that was forgotten. The
  row still draws; following it does nothing rather than opening something wrong. A link naming an
  agent in a project this window is not pointed at is the one case that can land somewhere wrong
  rather than nowhere — `G199`.
- **The history fills.** The oldest fall off at 200. A bell is a recent-events list, not a journal —
  what happened is in the log sink, which is where an investigation goes.
- **A rule outlives its usefulness.** Every rule has a duration and the longest one is explicit
  about being indefinite, so a silenced Ubiq is always a state the user chose and can see listed.
- **The host restarts.** The history and the rules go with it — they are held in memory. See
  `backlog.md`.

## Related docs

- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the notification family and its
  records
- [`workbench.md`](./workbench.md) — the titlebar the bell sits in
- [`logs.md`](./logs.md) — the sink a diagnostic goes to when it is not something to interrupt for
- [`../backlog.md`](../backlog.md) — what is not wired yet

## Next steps

- Raise the remaining real notifications: a pane exiting non-zero, a clone failing.
- Persist the history and the mute rules across a restart.
- A per-family default in settings, so a user can decide once that agents notify and files do not.
