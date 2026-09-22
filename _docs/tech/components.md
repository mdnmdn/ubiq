---
id: tech-components
title: Reusable components
kind: tech
status: current
summary: The reusable components Ubiq builds out of its own primitives — the floating popover, the multi-select dropdown a filter or a form narrows with, the activity bar a conversation heads with, the file picker any screen raises to choose a path, the viewer that draws one open file whole, the diff renderer two screens reach a change through, and the capabilities and tools panels each asked for by two surfaces — the state that drives them, and the discipline that keeps a compound a component rather than a one-off screen's decoration.
read_when: you are building a control that floats above another, letting a screen choose several values at once, adding a second activity reading to a conversation's bar, wiring a screen to choose a path on the interface's or a host's filesystem, adding a file kind the viewer draws, reaching a diff or a harness's capabilities from a second screen, or reshaping something the kit's primitives are insufficient for and a one-off would have duplicated
updated: 2026-09-22
verified: 2026-09-22
code_anchors: [crates/ubiq/src/ui/kit/popover.rs, crates/ubiq/src/ui/kit/menu.rs, crates/ubiq/src/ui/kit/mod.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/state/conversation.rs, crates/ubiq/src/ui/file_picker.rs, crates/ubiq/src/state/file_picker.rs, crates/ubiq/src/app/picker.rs, crates/ubiq/src/app/host_browse.rs, crates/ubiq/src/ui/file_dialog.rs, crates/ubiq/src/ui/teams/graph.rs, crates/ubiq/src/app/teams.rs, crates/ubiq/src/state/agents.rs, crates/ubiq/src/ui/viewer/mod.rs, crates/ubiq/src/ui/viewer/diff.rs, crates/ubiq/src/ui/editor.rs, crates/ubiq/src/ui/kb/mod.rs, crates/ubiq/src/ui/git/diff.rs, crates/ubiq/src/ui/acp_capabilities.rs, crates/ubiq/src/ui/tools.rs, crates/ubiq/src/ui/settings.rs, crates/ubiq/src/state/editor.rs]
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
`conversation::render` — the agents column and the chat tab alike — gets both chips with no
per-surface wiring. The state behind it is `panel_open: Option<ActivityPanel>` on the conversation
(`ActivityPanel::{Subagents, Todos}` in `crates/ubiq/src/state/conversation.rs`): **collapsed by
default, and one panel at a time**, because both lists answer the same question and the bar they
hang from is one line. Toggling a side while the other is open is a switch, never a second panel;
picking a subagent row closes the list again.

**Two surfaces host `conversation::render` today**, and what differs between them is the
`ConversationView` they hand it and nothing else: the agents column
(`crates/ubiq/src/ui/agents/column.rs`) and the chat tab (`crates/ubiq/src/ui/chat/mod.rs`). The
Teams canvas is not a third: it has no conversation surface of its own — selecting a card points a
`Chat` panel in the right dock at the agent (`AppState::open_teams_agent_panel`) rather than
embedding the component itself, so what draws the transcript is still the chat tab, at that tab's
own composer slot. A surface that wants a conversation of its own widens the composer pool by one
slot (`crates/ubiq/src/state/agents.rs`); it does not borrow another surface's, because a slot is
what addresses a turn.

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

## The multi-select

**What it is.** `kit::MultiPicker` — the dropdown for a question whose answer is a *set*. It is
`kit::Picker`'s sibling and shares its parts: the same trigger, the same panel, the same optional
search field, the same `MenuId` discipline that keeps one menu in the window open at a time. Two
things differ, and they are the whole component:

- **A row toggles instead of answering.** `on_pick(index)` reports which row was clicked and
  nothing closes the list, so narrowing to three values is three clicks rather than three reopens.
  The caller decides what a tick means; the control keeps no selection of its own.
- **Closed, it says what is ticked** — the values comma-separated in the list's own order, elided
  to `kit::menu::MULTI_WIDTH` with the whole list on the hover, through `kit::elided`, so the
  truncated label and the tooltip are one string and cannot disagree. Nothing ticked reads as the
  placeholder, which for a filter is what an empty selection *shows* (`all states`) rather than
  the word "none".

**The selection goes in as well as out.** `.selected(indices)` is where the current set is handed
back on every frame — indices into the `items` the caller passed. That is what makes one control
serve both a filter, preselected with what is narrowing the screen, and an edit form, preselected
with the record's own values.

**Order is `multi_order(len, selected, query)`.** With the field empty the ticked rows are drawn
first, because a selection scattered down a long list takes reading to recover. Once something is
typed nothing is pinned: the list is the search result in the search's own order, since a ticked
row lifted above better matches reads as a match that it is not. A list with **no** search field —
the states filter below — has no query to be empty and is never reordered, so four fixed rows do
not jump under the pointer as they are ticked.

A row may carry a `dot` — one status colour from `.dots(...)`, the same 7px dot `kit::toggle_pill`
wears — for a list whose values are read by colour before they are read by name.

**Its first use is the Teams toolbar's states filter** (`crates/ubiq/src/ui/teams/mod.rs`,
`MenuId::TeamsBuckets`), which replaced a row of four `toggle_pill`s: the buckets are the one
filter on that row where several values are on at once. The row's other two are left as they were,
because neither is a set — the session row is a choice of one, and `Hide done` is a toggle. The
style reference draws a specimen (`crates/ubiq/src/ui/sink/style.rs`), preselected, as every kit
primitive must.

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

## The viewer

**What it is.** One open file, drawn whole: the header that carries the layout toggle — or an
image's annotation toolbar — over a body that dispatches on what the file turned out to be. An
editor buffer, a Markdown render, a diagram, a scene, a picture, a diff. A caller hands it an
`OpenFile` and gets the whole arrangement back, and learns nothing about which of the six it got.

`render` in `crates/ubiq/src/ui/viewer/mod.rs` is the entry, `header` and `body` are its two halves,
and `ViewerKind` on `crates/ubiq/src/state/editor.rs` is what `body` matches on — with the status
bar's file-kind chip able to override it for the life of the tab. **Two hosts ask for it**: the IDE's
file tabs (`crates/ubiq/src/ui/editor.rs`) and a knowledge-base document tab
(`crates/ubiq/src/ui/kb/mod.rs`, `render_doc`). The second is what makes it a component rather than
the editor's own body — a KB document is reached through a different tree, loaded through a
different message and saved against a different write, and everything past that lookup is the
viewer's. A web panel's `Edit` position is a layout on this same axis, not a seventh body.

## The diff renderer

**What it is.** One `FileDiff` as a flat, virtualized list of rows — the `@@` hunk headers and the
old and new lines under them — in either the unified or the split arrangement. `render` in
`crates/ubiq/src/ui/viewer/diff.rs`, with `header` and `draw` under it.

**Two screens draw a change and neither owns the drawing.** The Git screen's diff panel
(`crates/ubiq/src/ui/git/diff.rs`) and the viewer's own `FileBody::Diff` arm both call it, so a
change reads identically whether it was opened as a tab or picked out of the changed-paths list.
That is the whole reason it is a component: the two paths to a diff are unrelated, and a reader
comparing what one shows against what the other shows must not find a difference that is only the
renderer's.

## The capabilities panel

**What it is.** The reading of what one harness said it can do at `initialize` — its identity, its
capability groups, the authentication methods it offers — as one panel. `panel` in
`crates/ubiq/src/ui/acp_capabilities.rs`.

**Two surfaces ask the same question and must not word it differently.** A conversation's info modal
(`crates/ubiq/src/ui/conversation/info.rs`) asks it about the harness behind *this* conversation;
the Harnesses settings section (`crates/ubiq/src/ui/settings.rs`) asks it about a harness in the
abstract. The facts are identical and the wording of a capability that is *absent* is the part worth
sharing — a reader who sees "not offered" in one place and a missing row in the other learns a
difference that is not there.

## The tools panel

**What it is.** A project's or the machine's runnable tools: the rows, and the add-or-edit form
under them. `panel` in `crates/ubiq/src/ui/tools.rs`, parameterised by `ToolEditScope` on
`crates/ubiq/src/state/settings.rs`.

**One panel twice, because only the list behind it differs.** Application settings passes
`ToolEditScope::System` and the project settings page passes `ToolEditScope::Project(project)`; the
rows, the form, the validation and the empty state are the same. The scope is a parameter rather
than two panels for the reason the file picker's `PickerOwner` is one: what differs between two uses
of a compound belongs in what the caller hands it, not in a second copy.

## The boundary with the kit

A compound lives either in the kit or in the screen that uses it, never in both:

- **`kit::popover` is chrome with no knowledge of the workbench.** It does not know what a
  subagent is, so the subagent rows are `agent_row`s a screen builds — and it does not care that
  its trigger sits at the bottom of a conversation, so the panel never breaks if another screen
  hangs it off a titlebar. It is exported with the kit's other reusable parts, and `tech/ui-and-design.md`
  documents the shape conventions it obeys.
- **`kit::MultiPicker` is the same rule one step further.** It shares the dropdown's parts with
  `kit::Picker` rather than forking a copy of them, and knows nothing about what a bucket, a label
  or a role is — it is handed strings, indices and colours, and hands an index back.
- **The activity bar is a conversation's, not the kit's.** It knows the conversation's vocabulary —
  `status_label`, `subagent_count_label`, the todo count — and that is exactly what keeps it out
  of the kit, which has no vocabulary. The chat surface gets it by embedding the conversation
  view, not by importing a chip.

The gap between the two is the rule: a component earns the kit when the second screen wants the
*shape*; it stays in the screen when the second screen wants the *meaning*.

**Four of the eight compounds above sit on the meaning side, and none of them is in the kit.** The
viewer knows what a Markdown file is, the diff renderer knows what a hunk is, the capabilities panel
knows what ACP advertises and the tools panel knows what a runnable tool is — so each lives in
`crates/ubiq/src/ui/` under its own name and is called by the two screens that want it, rather than
being generalised into a shape the kit could hold. The test a new compound answers is which of the
two it is: a second screen that wants the same *arrangement* is asking for a kit primitive, and a
second screen that wants the same *answer* is asking for one of these.

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