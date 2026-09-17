---
id: tech-components
title: Reusable components
kind: tech
status: current
summary: The reusable components Ubiq builds out of its own primitives — the floating popover, the activity bar a conversation heads with, and the file picker any screen raises to choose a path — the state that drives them, and the discipline that keeps a compound a component rather than a one-off screen's decoration.
read_when: you are building a control that floats above another, adding a second activity reading to a conversation's bar, wiring a screen to choose a path on the interface's or a host's filesystem, or reshaping something the kit's primitives are insufficient for and a one-off would have duplicated
updated: 2026-09-17
verified: 2026-09-17
code_anchors: [crates/ubiq/src/ui/kit/popover.rs, crates/ubiq/src/ui/kit/mod.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/state/conversation.rs, crates/ubiq/src/ui/file_picker.rs, crates/ubiq/src/state/file_picker.rs, crates/ubiq/src/app/picker.rs, crates/ubiq/src/app/host_browse.rs, crates/ubiq/src/ui/file_dialog.rs, crates/ubiq/src/ui/teams/inspector.rs, crates/ubiq/src/state/agents.rs]
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

**What it is.** The single line in a conversation's bottom block that answers "what is the agent
doing next": the spawned subagents on the left, the agent's own todo list on the right, spanning the
block's full width with a flexible spacer between the two chips so a lone one still reaches its own
edge rather than hugging the other's. Each side is a chip — a chevron and a count — and asking for
either opens its panel, the list drawing *upward* over the transcript, flush against the line's own
top edge, so the composer never moves under the cursor.

- Both chips are drawn from one `activity_chip` helper, taking `chevron_first` to put the chevron
  before the label on the left and after it on the right, so both land on the row's outer edges and
  neither can disagree with the other about which side its own chevron faces for a given `open`.
- The left chip carries the switch's chevron before the count, `3 active subagents of 10`; the
  right chip carries the count before its chevron, `2/5 todos`, mirrored because it ends the line
  rather than opening it.
- Either chip exists only when its list is non-empty, and the whole bar only when either is — a
  conversation that never spawned and never planned looks exactly as it did before that line. The
  bar itself sits above the footer and below anything queued, which is drawn higher still in the
  block when there is one.
- The todo panel shows up to eight entries, the same mark per status the old inline strip drew,
  then `… n more`; it is read-only, a replacement list the harness re-sends whole.

`activity_bar` in `crates/ubiq/src/ui/conversation/mod.rs` draws it, so every surface that embeds
`conversation::render` — the agents column, the chat tab and the Teams canvas inspector alike —
gets both chips with no per-surface wiring. The state behind it is `panel_open: Option<ActivityPanel>` on the conversation
(`ActivityPanel::{Subagents, Todos}` in `crates/ubiq/src/state/conversation.rs`): **collapsed by
default, and one panel at a time**, because both lists answer the same question and the bar they
hang from is one line. Toggling a side while the other is open is a switch, never a second panel;
picking a subagent row closes the list again.

**Three surfaces host `conversation::render` today**, and what differs between them is the
`ConversationView` they hand it and nothing else: the agents column
(`crates/ubiq/src/ui/agents/column.rs`), the chat tab (`crates/ubiq/src/ui/chat/mod.rs`) and the
Teams canvas inspector (`crates/ubiq/src/ui/teams/inspector.rs`). The inspector is the third and
newest: it passes `header: true`, because it has no toolbar of its own to hang the lifecycle menu
in, an element-id prefix of `teams-{agent}`, and `TEAMS_SLOT` — the composer the pool reserves for
it, above the chat tabs' range and the sink bench's (`crates/ubiq/src/state/agents.rs`). A surface
that wants a conversation widens that pool by one; it does not borrow another surface's slot,
because a slot is what addresses a turn.

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

## The file picker

**What it is.** One dialog for choosing a path, raised over whatever forest of files and folders a
caller hands it and answered back to whoever raised it. `crates/ubiq/src/ui/file_picker.rs` draws it
in the window's own shape — square, filled, a coloured left edge, painted over everything through
`deferred` and `anchored` — and `crates/ubiq/src/state/file_picker.rs` holds what it needs to:
`FilePickerState` for the dialog itself, `PickerRequest` for what a caller asked for, `PickKind`
(`Files`/`Folders`/`Either`), `PickerCount` (`Single`/`Multiple`) and `Commit` (`OnClick`/`OnButton`)
for the six ways one ask differs from another, and `PickerOwner` for who is owed the answer. It is
raised through `AppState::open_file_picker(request, forest, view, window, cx)` in
`crates/ubiq/src/app/picker.rs`.

**It takes the forest it draws rather than fetching one.** The dialog reads no disk itself; a caller
builds the tree and hands it in, and grows it further as more arrives. Four owners exist today: the
kitchen sink's own fixture tree, the composer's `@`-mention (built from the open project's explorer,
fully listed by that point), and the two host-backed ones — `PickerOwner::HostProject`, for opening a
project on a detached host, and `PickerOwner::KbFolder`, for the folder a knowledge base source reads
from. Both run on `crates/ubiq/src/app/host_browse.rs`, which asks `Message::BrowseHostDir` as the
user walks into a folder and folds each answer into the forest on screen, since nothing about a
host's filesystem can be known up front. `HostBrowseState::host` is a `HostRef` rather than a
`HostId` for exactly that pair: one browses a named remote, the other browses whichever host serves
the project being edited — usually the local one.

**Why it matters.** The platform's own folder dialog — `cx.prompt_for_paths` — browses the
*interface's* filesystem, which `tech/decisions.md` and `backlog.md` both name as the one place the
two halves are assumed to share a machine (`G32`). A screen that has to choose a path on the *host's*
machine, rather than the interface's, raises this picker instead — adding a project from a detached
host is what it was built for, and the knowledge base's "Add source" form is the second screen to
reach for it rather than for the platform dialog.

`crates/ubiq/src/ui/file_dialog.rs` is a different thing entirely, despite the name: the name-and-
confirm modals around a file operation — new file, rename, delete, the untitled buffer's save-as —
answered from the window's one `file_name` field. Neither draws the other.

**The standing gap.** The picker is raised by one screen only — the kitchen sink's picker page, over
its fixture tree — so no screen has yet wired a real project's `ProjectTree` listings into
`PickerNode`s to choose a path through it for real work (`G68`).

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