---
id: inbox-detached-panes
title: Notes — what detaching a pane does not cover
kind: proposal
status: proposal
summary: In-session detach landed as D103 — closing a terminal panel keeps the harness running and reattaching picks its live emulator back up. These are the two larger things it deliberately does not do, and what each would cost — reattaching from a different window, and outliving the application.
read_when: you are asked why a detached pane dies with its window, or whether a harness can outlive Ubiq
updated: 2026-09-10
depends_on: [feat-panes, feat-sessions, feat-chat, tech-architecture, tech-decisions]
---

# Notes — what detaching a pane does not cover

**In-session detach is built.** `D103` is the decision and
[`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) is the behaviour: closing
a terminal panel detaches, the harness keeps running under the host, and reopening a panel over its
still-live emulator reattaches with the screen intact. It needed no message and no host change,
because the emulator was never owned by the panel and the host is never told a panel closed.

This document is what that deliberately left out. Both items below were considered and scoped out,
and neither is a gap in the thing that shipped.

## 1. Reattaching from a different window

A detached pane can be reattached only by the window that owns it. `owners: HashMap<PaneId,
ClientId>` in the coordinator is total onto a client, not optional, and no message moves a pane
between owners — ownership is written when the pane is created and only ever removed.

What it would take: an unowned state in that map, a message to claim a pane, and a rule for what
happens when two windows want the same one. The awkward part is not the plumbing but the question
underneath it — a pane's bytes route to one client, so a claim is a handover rather than a share,
and the window that loses it has to be told. Worth doing if the multi-window case turns out to be
how people actually work; nothing about the current design blocks it.

## 2. Outliving the application

**A detached pane still dies with its window, and everything dies with the application.**
`client_gone` kills every pane its client owned, and the comment there says the kill is deliberate:
"without this, every closed window leaves a live harness behind." On top of that the application
quits when its last window closes. Neither was touched.

This was raised as the original shape of the request and ruled out: what was wanted was surviving a
panel close, not a shutdown. Recording why it is more than one more step, in case it comes back:

**It is not a pane feature.** An application that keeps running with no window needs a posture — a
menu-bar presence, a dock-only life, a preference — and a way back in that is not launching it
again. That is a decision about what Ubiq is.

**It would break the reason in-session detach was cheap.** Keeping the emulator alive works because
the window is still there to hold it. A harness that outlives every window has no emulator, so the
screen problem comes back in full: the host would have to retain the bytes it has never retained,
and a ring truncated past the escape that entered the alternate screen replays absolute-positioned
output onto the wrong screen. There is also no redraw primitive to fall back on — `Pty::resize`
passes its size straight through with no dedup, so resizing to the size the harness already believes
is a `SIGWINCH` most TUIs answer by doing nothing. The workable shape would be a ring for the
scrollback plus a resize-to-different-and-back to make the live screen the harness's own drawing.

**It would change what the run-directory sweep means.** The sweep deletes what a previous process
left because "no pane from a previous process is still running", which holds only while a pane
cannot outlive its process.

**A conversation would want the same thing.** `end_conversation` stops the harness before it reads
the persistence flag, so a persistent conversation parks its run directory and its process still
dies — `Agents::park_agent` says "the process is gone". An unowned state and a host that outlives
its windows is exactly what would make persistence mean "still running" for the first time, so this
is one change serving both faces rather than a pane feature.

## Related docs

- [`../tech/decisions.md`](../tech/decisions.md) — `D103`, the decision this is the remainder of
- [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) — what detach does
- [`../features/chat.md`](../features/chat.md) — persistence and revival, the conversation half
