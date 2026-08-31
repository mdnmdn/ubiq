---
id: feat-chat
title: The chat panel
kind: feature
status: draft
summary: The conversation beside the work — the chat list, the run and context readout, the transcript with its tool blocks and diffs, and the composer that chooses harness, model and mode.
read_when: you are changing the chat panel, a message or tool block renderer, or the composer's pickers
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [crates/ubiq/src/ui/chat/mod.rs, crates/ubiq/src/ui/chat/sidebar.rs, crates/ubiq/src/ui/chat/transcript.rs, crates/ubiq/src/ui/chat/composer.rs, crates/ubiq/src/state/chat.rs]
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
`+ New chat` puts an empty conversation at the top and selects it, and that chat takes its title
from the first thing said in it. The list collapses from the panel's header, leaving the transcript
the whole height.

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

**The composer says where the message is going.** A harness picker, a model picker, a thinking-budget
picker and a mode picker sit under the text, each opening upward, and only one menu in the whole window is open at a
time. Enter sends; Shift-Enter inserts a newline. Send is disabled while the draft is empty.
Sending appends the turn, clears and refocuses the field, and scrolls the transcript to the bottom.

**Nothing is behind it yet.** The reply reports the harness, model, thinking budget and mode the
composer is set to, and stops there.

## Contract

None. The transport contract has no chat family, so the panel's state is local to the UI and seeded
from `crates/ubiq/src/state/sample.rs`. Giving the chat a real agent means adding a message family
in [`../tech/transport-contract.md`](../tech/transport-contract.md) first; it is a row in
[`../backlog.md`](../backlog.md) until then.

The harness list is labels for a picker. Ubiq never names a harness config path and never hard-codes
how to launch one — see [`../tech/agent-manager.md`](../tech/agent-manager.md).

## Implementation

`crates/ubiq/src/state/chat.rs` holds the model: `Chat`, `ChatMessage`, `Block`, `ToolCall`,
`DiffLine`, `RunState`, and the `ChatState` that owns the list, the selection, the composer's
selections and the draft. `ChatState::send` appends the user turn and the canned reply;
`AppState::send_chat` in `crates/ubiq/src/app.rs` clears the textarea, refocuses it and scrolls.

Rendering is four modules under `crates/ubiq/src/ui/chat/`: `mod.rs` assembles the panel,
`sidebar.rs` draws the header, the list and the status strip, `transcript.rs` draws the turns and
their blocks, and `composer.rs` draws the input and its pickers. The pickers are the shared
`ui::kit::Picker`; the scrollbar is a sibling of the scroll area, so it stays put while the content
moves under it.

## Failure

| What happens | Result |
|---|---|
| The active chat has no messages | The transcript says so rather than rendering empty space |
| The draft is empty or whitespace | Send does nothing and the button reads as disabled |
| A tool block carries no body or diff | Its header renders and does not expand |
| A menu is open and the user clicks elsewhere | The menu dismisses; no other menu opens on the same click |

## Related docs

- [`workbench.md`](./workbench.md) — the panel this one lives in
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the message set a real chat would need
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens the blocks and pills are coloured from
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — who owns harness knowledge

## Next steps

- Give the chat a transport family and a real harness behind it.
- Stream an assistant turn as it arrives, rather than appending it whole.
- Attachments that carry a real file rather than a chip.
- Let a tool block open the file it names in the editor.
