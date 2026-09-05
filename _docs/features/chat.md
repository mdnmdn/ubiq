---
id: feat-chat
title: The chat panel
kind: feature
status: draft
summary: The conversation beside the work — the chat list, the run and context readout, the transcript with its tool blocks and diffs, and the composer that hosts it.
read_when: you are changing the chat panel, a message or tool block renderer, or its composer
updated: 2026-09-05
verified: 2026-09-05
code_anchors: [crates/ubiq/src/ui/chat/mod.rs, crates/ubiq/src/ui/chat/sidebar.rs, crates/ubiq/src/ui/chat/transcript.rs, crates/ubiq/src/state/chat.rs]
depends_on: [feat-workbench]
review_cycle: monthly
---

# The chat panel

## Purpose

A harness in a terminal shows what an agent is doing; the chat shows what it was asked and what it
concluded. The panel keeps that conversation beside the code rather than in another window. It is
IDE furniture today and leaves with the mode, though it is written to be reusable by the other
screens.

## Behaviour

**Several conversations, one at a time.** The list shows each chat's title and how long ago it
moved. The selected row is marked with an accent bar and fill; clicking one swaps the transcript.
`+ New chat` raises the window's new-agent menu — the same one the agents screen's `New agent`
control does, painted by `ui::shell` so both surfaces share it. Picking a harness there starts the
conversation at once, with no naming prompt in between; that is what puts an empty conversation at
the top and selects it. That chat takes its title from the first thing said in it.
The list starts collapsed, from the panel's header, leaving the transcript the whole
height; toggling the header opens and closes it by hand, and selecting a chat — whether the list
was open to pick it or already shut — always leaves it closed again afterward.

**The run state and the context window are always visible.** An `Idle` / `Working` pill takes its
colour from the status group, never from wording alone, and a second pill carries a drawn ring, the
percentage of the context window in use, and the token count behind it.

**A turn is either something the user said or a sequence of assistant blocks.** A user turn is a
raised bubble with an accent edge. An assistant turn is an avatar and a column of blocks, and a
block is either markdown or a tool call.

**Markdown is parsed, not approximated.** Bold, inline code, lists and code fences render as
themselves.

**A tool block states what it did before it shows it.** The header carries the verb, the target and
a one-glance summary — `READ panels/AgentTerminal.tsx  142 ln`, `EDIT panels/AgentTerminal.tsx
+4 −1`. Its left edge is coloured by kind: reads and greps are informational, an edit is a change, a
command is something that ran. Clicking the header expands it; a block with nothing behind it does
not expand. An expanded edit shows its diff with added and removed lines on their own status
colours.

**The composer hosts a real conversation, once one is selected.** There is no separate fixture
composer any more — the panel's input is the shared `crates/ubiq/src/ui/conversation` renderer's
composer, the same one every screen that hosts a live agent uses. Enter sends; Shift-Enter inserts
a newline. Send is disabled while the draft is empty. Sending appends the turn, clears and
refocuses the field, and scrolls the transcript to the bottom.

**Before a harness has launched, the composer offers up to three pickers instead of a running
conversation's read-only pills** — model, thinking level, and mode, in that order, one per config
option the host has advertised so far. All three are launch-time only: a pick is recorded locally
and sent as `SetAgentConfig`, but nothing takes effect until the first prompt actually launches the
harness. See [`../tech/transport-contract.md`](../tech/transport-contract.md)'s conversation family
for the wire shape.

**A three-dots menu, top left of the shared renderer's header, offers the conversation's four
lifecycle verbs.** Stop interrupts the turn in flight, leaving the harness up; Unload kills the
harness but keeps the conversation, its transcript and its run directory; Resume starts the harness
again under the same agent, with no prompt; Delete ends the conversation outright, taking the run
directory and its transcript with it. Each item disables — rather than disappears — when it does not
apply, so the menu's shape never changes under the cursor: Stop only while a turn is running, Unload
only while the harness is up, Resume only while it is not, Delete always. Delete alone is confirmed
before it fires, being the one irreversible verb of the four. See
[`sessions-and-workspaces.md`](./sessions-and-workspaces.md) for what unload keeps that delete does
not.

**What it pioneered is drawn elsewhere for a live agent.** The turn, the markdown block, the tool
block with its verb, its target and its diff were designed here and are what
`crates/ubiq/src/ui/conversation/mod.rs` draws from a real harness's stream, for whichever screen
hosts it. That renderer takes a `ConversationView` saying what differs between hosts — an id prefix,
a composer slot, whether a footer and a composer come with it — so giving this panel a live agent is
a matter of passing one and holding a `Conversation`, rather than of writing a second transcript.
The panel draws its own instead, which is why the two exist side by side and why only one of them
answers.

## Contract

The panel's own list and selection are local to the UI and seeded from
`crates/ubiq/src/state/sample.rs` — the transport contract carries no chat-specific family for
them. Its composer is not local in the same way: once a conversation is selected, it is the shared
renderer's composer, and speaks whatever [`../tech/transport-contract.md`](../tech/transport-contract.md)'s
conversation family carries, the same as every other screen that hosts one. What the panel still
lacks is a conversation of its own to select outside the sample fixtures, not a message set to
invent. It is a row in [`../backlog.md`](../backlog.md) meanwhile.

The harness list is labels for a picker. Ubiq never names a harness config path and never hard-codes
how to launch one — see [`../tech/agent-manager.md`](../tech/agent-manager.md).

## Implementation

`crates/ubiq/src/state/chat.rs` holds the model: `Chat`, `ChatMessage`, `Block`, `ToolCall`,
`DiffLine`, `RunState`, and the `ChatState` that owns the list, the selection and the draft.
`ChatState::send` appends the user turn and the canned reply; `AppState::send_chat` in
`crates/ubiq/src/app/chat.rs` clears the textarea, refocuses it and scrolls.

Rendering is three modules under `crates/ubiq/src/ui/chat/`, and they are the panel's own rather
than the shared renderer's: `mod.rs` assembles the panel, `sidebar.rs` draws the header, the list
and the status strip, and `transcript.rs` draws the turns and their blocks. The fixture composer
with its four pickers is gone; the input is the shared renderer's composer once a conversation is
selected. The run pill is the shared `ui::kit::state_chip`; the scrollbar is a sibling of the
scroll area, so it stays put while the content moves under it.

## Failure

| What happens | Result |
|---|---|
| The active chat has no messages | The transcript says so rather than rendering empty space |
| The draft is empty or whitespace | Send does nothing and the button reads as disabled |
| A tool block carries no body or diff | Its header renders and does not expand |
| A menu is open and the user clicks elsewhere | The menu dismisses; no other menu opens on the same click |

## Related docs

- [`workbench.md`](./workbench.md) — the panel this one lives in
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the conversation family a live chat would speak
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens the blocks and pills are coloured from
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — who owns harness knowledge

## Next steps

- Host the shared conversation view, and a live agent with it.
- Attachments that carry a real file rather than a chip.
- Let a tool block open the file it names in the editor.
