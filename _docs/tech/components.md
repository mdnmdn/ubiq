---
id: tech-components
title: Reusable components
kind: tech
status: current
summary: The reusable components Ubiq builds out of its own primitives — the floating popover and the activity bar a conversation heads with — the state that drives them, and the discipline that keeps a compound a component rather than a one-off screen's decoration.
read_when: you are building a control that floats above another, adding a second activity reading to a conversation's bar, or reshaping something the kit's primitives are insufficient for and a one-off would have duplicated
updated: 2026-09-14
verified: 2026-09-14
code_anchors: [crates/ubiq/src/ui/kit/popover.rs, crates/ubiq/src/ui/kit/mod.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/state/conversation.rs]
depends_on: [tech-ui]
review_cycle: monthly
---

# Reusable components

## What this is

A screen is built from the kit's primitives, and from a few **compound components** that put several
primitives together into one unit a screen asks for whole. This is the register of those compounds:
their name, what they are responsible for, where they live, and the discipline that decides whether
a new one is worth making. The primitives themselves — the slab, the pill, the field, the
dropdown — are the kit's, and `tech/ui-and-design.md` carries their conventions; this document does
not restate a shape rule its owner states.

A compound earns its name when **the second screen wants it**. The first use is an extraction that
must not wait for a third: the panel that hangs over a conversation's bottom row started as a
subagent list, and its move into `kit::popover` happened the day the todo list needed it, not after
a third list invented its own poorer copy. An element drawn once, for one screen, stays inline.

## The activity bar

**What it is.** The single line at the top of a conversation's bottom block that answers "what is
the agent doing next": the spawned subagents on the left, the agent's own todo list on the right.
Each side is a chip — a chevron and a count — and asking for either opens its panel, the list
drawing *upward* over the transcript, flush against the line's own top edge, so the composer never
moves under the cursor.

- The left chip carries the switch's chevron before the count, `3 active subagents of 10`; the
  right chip carries the count before its chevron, `2/5 todos`, mirrored because it ends the line
  rather than opening it.
- Either chip exists only when its list is non-empty, and the whole bar only when either is — a
  conversation that never spawned and never planned looks exactly as it did before that line.
- The todo panel shows up to eight entries, the same mark per status the old inline strip drew,
  then `… n more`; it is read-only, a replacement list the harness re-sends whole.

`activity_bar` in `crates/ubiq/src/ui/conversation/mod.rs` draws it, so every surface that embeds
`conversation::render` — the agents column and the chat tab alike — gets both chips with no
per-surface wiring. The state behind it is `panel_open: Option<ActivityPanel>` on the conversation
(`ActivityPanel::{Subagents, Todos}` in `crates/ubiq/src/state/conversation.rs`): **collapsed by
default, and one panel at a time**, because both lists answer the same question and the bar they
hang from is one line. Toggling a side while the other is open is a switch, never a second panel;
picking a subagent row closes the list again.

## The popover

**What it is.** The floating list `kit::popover(panel_id, min_width, debug, rows)` draws: one
`deferred` + `anchored` pair, `Anchor::BottomLeft` putting the panel's bottom edge flush on the
trigger's top, over whatever is under it — the transcript, in a conversation. The caller owns the
rows; the panel owns the chrome: its square filled shape, the coloured left edge that says what it
is, the window-out-of-reach clamp, and the `MENU_LAYER` it paints at. The modal, the dropdown and
this panel shared the mechanism; `popover` in
`crates/ubiq/src/ui/kit/popover.rs` is the same pair with the chrome pre-built, so a fourth
floating list does not rebuild it a fourth time.

An optional debug name keeps the panel findable by its tests — the dropdowns this panel mirrors
answer to nothing but their element id, while a conversation screen's tests name their own panels.

## The boundary with the kit

A compound lives either in the kit or in the screen that uses it, never in both:

- **`kit::popover` is chrome with no knowledge of the workbench.** It does not know what a
  subagent is, so the subagent rows are `agent_row`s a screen builds — and it does not care that
  its trigger sits at the bottom of a conversation, so the panel never breaks if another screen
  hangs it off a titlebar. It is exported with the kit's other reusable parts, and `tech/ui-and-design.md`
  documents the shape conventions it obeys.
- **The activity bar is a conversation's, not the kit's.** It knows the conversation's vocabulary —
  `status_label`, `subagent_count_label`, the todo count — and that is exactly what keeps it out
  of the kit, which has no vocabulary. The chat surface gets it by embedding the conversation
  view, not by importing a chip.

The gap between the two is the rule: a component earns the kit when the second screen wants the
*shape*; it stays in the screen when the second screen wants the *meaning*.

## Rationale

The subagent switcher existed as one screen's own `deferred` + `anchored` block, and the todo list
as an inline strip below the composer. Adding the todo list called for the same floating shape —
and the same single-line discipline the switcher practised — which made both an extraction
of the panel and a folding of the two readings into one bar. Keeping one panel open at a time is
deliberate: two lists for the same question, open together, would be a second answer drawn where a
switch belongs, and the count-on-the-line resting state keeps a conversation's delegates and plans
a line rather than a permanent list.

## Related docs

- [`tech/ui-and-design.md`](./ui-and-design.md) — the kit's primitives, their conventions, and the
  shape every surface and suspended layer is drawn in
- [`features/chat.md`](../features/chat.md) — the chat tab, whose body embeds the conversation view
  and with it this bar
- [`crates/agent-manager/_docs/`](../../crates/agent-manager/_docs/) — the ACP bridge's definitions
  of a delegate and a plan, which the bar's two chips read