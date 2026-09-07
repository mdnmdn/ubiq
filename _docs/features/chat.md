---
id: feat-chat
title: The chat panel
kind: feature
status: draft
summary: Editor-like chat tabs — many, movable to any dockable region, each a view onto a host-owned conversation or onto none, drawn by the composer, transcript and tool blocks the whole window shares.
read_when: you are changing a chat tab, the control that starts or attaches a conversation, or which conversation a tab shows
updated: 2026-09-07
verified: 2026-09-07
code_anchors: [crates/ubiq/src/ui/chat/mod.rs, crates/ubiq/src/ui/chat/sidebar.rs, crates/ubiq/src/state/chat.rs, crates/ubiq/src/state/dock.rs, crates/ubiq/src/app/chat.rs, crates/ubiq/src/app/panels.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/ui/agents/mod.rs, crates/ubiq/src/ui/dock/skin.rs, crates/ubiq/src/state/prefs.rs]
depends_on: [feat-workbench]
review_cycle: monthly
---

# The chat panel

## Purpose

A harness in a terminal shows what an agent is doing; a chat tab shows what it was asked and what
it concluded, beside the code rather than in another window. It is IDE furniture and leaves with
the mode. Unlike every other panel IDE mode draws, it comes in many instances at once: a chat tab is
a perspective on a conversation the host owns, not a conversation of its own, so many may be open —
each attached to a different run, or to none — and closing one ends nothing.

## Behaviour

**A chat tab is `PanelKind::Chat(ChatId)`, one panel per open instance.** The id is minted the way
`AgentId::generate` mints one, but locally: it is UI arrangement the host never hears about, the
same as a file's tab key names a document the host does hear about. Several tabs coexist, dragged
apart, tabbed together, or moved to any dockable region — left, right, bottom or the centre — the
same freedom a terminal panel already has. Its default home is the right edge, at `CHAT_WIDTH`.

**A tab is attached to a conversation, or to nothing.** The attachment is `state::chat::ChatTab`, one
entry per tab in the project's own `OpenProject::chats`, holding the tab's id, its composer slot,
whether its start-or-attach control is down, and the `AgentId` it is looking at. Closing a tab drops the
`ChatTab` and frees its slot; the conversation, if it had one, is the host's and keeps running.

**One control chooses what a tab is looking at, and it is the same control that starts one.** A tab
either shows a conversation or it does not, and what the user wants in each case is a different
thing — begin something, or move to something already running. Two controls side by side made the
user pick the question before answering it. The trigger, in the tab's own header, wears `Play` and
reads `Start or attach` on an empty tab, and the harness glyph and the conversation's name on an
attached one.

**An empty tab is offered both halves; an attached tab only the second.** Starting from a tab that
already shows a conversation would leave that one with no view and no way back to it, so the offer
to start is the empty tab's alone — which is what lets one control answer both questions without
either becoming a trap. The first half is the harness list, grouped exactly as the agents screen's
own `New agent` menu groups it, under a `Start new` heading; the second is every conversation the
project has — the same registry the agents sidebar lists — under `Attach running`, a heading drawn
only when there is something under it.

**Typing filters both halves.** A harness list runs to hundreds of rows once every account and saved
setup is on it, and a search that reached only the conversations would leave the long half
untouched. While something is typed the headings and hairlines are dropped rather than left standing
over rows that may all have gone: what a search shows is the matches.

A conversation already attached to a *different* chat tab draws disabled and cannot be picked; it is
never dropped from the list, because a row that vanishes reads as a conversation that ended rather
than one taken. The tab's own current attachment stays selectable, since it is the row already
checked.

**An empty tab opens on the last harness anything was started on.** `InterfacePrefs::last_start`
records the harness, the account and the saved setup a conversation was last begun with, so the
common case is one click and the list is still there for every other. Interface scope rather than a
project's: which harnesses this machine has, and which account is signed into them, is a fact about
the machine. It is a hint, never a promise — a harness uninstalled or an account signed out since
simply preselects nothing.

**Exclusivity is per chat tab, not per conversation, and it stops at this surface's edge.** The
agents workbench may show the same conversation in a column at the same moment a chat tab is
attached to it, and the host is never told which surfaces are looking, because a view was never the
workspace.

**Adding a view is the tab strip's gesture, not the panel's.** A second chat tab, attached to
nothing, is opened by a `+` on the dock's own tab strip — beside the terminal region's, offered on
the strip of any group holding a chat, so a chat dragged into the editor region takes the control
with it rather than leaving the gesture behind. Opening another view of the same kind is what a tab
strip does in this window, and the chat panel is not an exception to it. Starting a *harness* is the
header control's; adding a *view* is the strip's.

**The header reads left to right in the order a reader asks.** The state mark says what the
conversation *is*, the control beside it says what it is *on*, and the three-dots at the far right
says what can be *done to it*. Nothing attached draws the control alone: there is no glyph and no
menu with no conversation to read.

**Nothing in the header names the tab.** The dock's tab already carries the conversation's name, and
a second copy of it directly under the first was the same answer twice.

**Closing the last chat tab is allowed.** There is no last-tab guard anywhere in this tree, and a
chat tab is no exception: closing the only open one leaves nothing behind but a tab strip with
nothing in it, which the region then puts itself away rather than sit empty. Opening the right
region again — the titlebar's switch, or the `+` past the last group's tab strip — mints a fresh
tab, attached to nothing, because that is the one place the window has to decide *which* instance an
empty region opens onto.

**Every chat tab draws from the same shared conversation view.** What a tab shows for its attachment
is `crates/ubiq/src/ui/conversation`, the transcript, the tool blocks, the footer and the composer
every surface that hosts a live agent shares — the run pill, the context ring, the token cost, the
launch-time model and thinking pickers. A tab unattached to anything shows a page naming what fixes
it, the same way the agents screen's empty page does. **A link in a rendered reply goes where it
points**: the transcript hands `ui::on_link` to its `TextView`, so a relative path resolved from the
project root or a full `ubiq://` opens that place in the window, `http`, `https` and `mailto` reach
the operating system, and anything else does nothing — see
[`workbench.md`](./workbench.md). A path merely *mentioned* in prose is text, not a link.

**Delegates are a line, and a list only when asked for.** A conversation that spawned subagents
grows one row at the top of the bottom block — above the footer, above the composer — reading
`3 subagents`, and nothing else; a conversation that spawned none grows nothing. Opening it draws
one row per agent *upward*, over the transcript, through the same `anchored` + `deferred` pair
every menu in the window uses, so the composer never moves under the cursor. Each row says who and
what it is doing, and clicking one switches the transcript to that agent; the main agent is always
a row, because it is the way back. A subagent whose spawning call is not in the transcript reads
`unknown` rather than being claimed to be running. **A delegate says what it is answering with**:
the reading strip above its transcript carries its model beside its name, and its row's hover names
its kind, its model and its thinking level where the harness stated one — the model shortened by
`short_model_label`, the same shortener the composer's model chip uses, so one conversation never
spells a model two ways. Both read the one `SubagentTab` field, resolved on `Conversation` beside
`subagents`. Nothing is borrowed from the parent: a delegate the harness named no model for draws
nothing, and `thinking` is `None` on every harness today because no stream states a per-delegate
effort level. The `AGENT` block that spawned an agent is the
same door: clicking it switches the transcript, and stays inert until that agent has said
something.

**`ctx` is a level, `tot` is a flow.** The footer's ring and its `ctx` count are how full the
context window is *now* — a number that falls when the conversation is compacted — and `tot` is
every token the conversation has ever billed, subagents included, which only grows. That is why one
is a ring and the other a number, and every readout in the row says which it is on hover: the
identity chip, the ring, `ctx`, `tot` with its per-way and per-subagent breakdown, and the
composer's model, thinking and mode chips.

**The status glyph and the three-dots lifecycle menu are the one exception: the tab's own header
draws them, not the shared view.** `ConversationView::header` tells the shared view whether to draw
its own bordered strip for them — `true` on the agents column, unchanged; `false` here, because the
chat panel's header draws the identical fragment inline instead — and at *opposite ends* of its
row, so it takes the two halves separately: `ui::conversation::lifecycle_mark` for the glyph and
`lifecycle_menu` for the three-dots, the same pair `lifecycle_controls` composes for the column. One
set of functions either way: the glyph's state and the menu's enable rule are read once, in
`crates/ubiq/src/ui/conversation/mod.rs`, and both surfaces call them rather than each keeping an
answer of its own.

**The glyph says the conversation's state; the word lives in its tooltip.**
`ui::conversation::lifecycle` reads `launched`, `run`, `blocks`, `accepts_input` and `config` into one
`Lifecycle` — Starting, Ready, Working (carrying which `Activity`), Idle, Unloaded, or Ended — derived
rather than stored, so nothing new sits on `Conversation` for it. `Unloaded` and `Starting` are both
`launched == false`; the transcript, `blocks`, is what tells them apart, because a harness that is
gone still leaves what it said and one never started leaves nothing. The glyph is a `kit::status_dot`,
no new primitive, coloured by `Activity`'s own reading while a turn runs and by the same tokens the
bucket colours use otherwise; the tooltip is one or two words, `Unloaded`, `Working · Tools`, never a
sentence — replacing the muted line P7 drew above the composer for the same fact.

**Each tab owns a composer of its own, from the same fixed pool a column draws from.** The window
builds `COMPOSER_SLOTS` text areas — `0..COLUMNS_MAX` for columns, the range above it for chat tabs
— before the first frame, because the *subscription* that mirrors what is typed has to be held for
the window's life. What was typed at one tab never turns up in another's field, and closing a tab
clears its slot's draft before handing the slot to the next tab that opens.

## Contract

A chat tab's own state — its id, its slot, its attachment, whether its picker is down — is local to
the UI, the same as which column an agent's conversation is drawn in. No message names a `ChatId`
and none carries a tab's arrangement; the host answers only about conversations, never about which
surface is looking at one. Once a tab is attached, it speaks whatever
[`../tech/transport-contract.md`](../tech/transport-contract.md)'s conversation family carries, the
same as every other screen that hosts one.

## Implementation

`crates/ubiq/src/state/dock.rs` holds `ChatId` — a locally minted counter, `Display` and `FromStr`
so it round-trips through the dock's saved payload the way a pane's id does — and
`PanelKind::Chat(ChatId)`'s `class` (`Free`, so it may sit anywhere), `home` (the right region),
`closable` and `is_drawn` rules. `crates/ubiq/src/ui/dock/mod.rs`'s `chat_payload` and
`chat_from_payload` are that round trip; a saved leaf naming an id this window did not already hold
is dropped on restore, the way a saved terminal leaf naming a gone pane is — a chat id is not the
host's to confirm, so an unfamiliar one is trusted no further than an unfamiliar pane id is.

`crates/ubiq/src/state/chat.rs` holds `ChatTab`, `free_chat_slot` — the lowest slot in the chat
range nothing is using — and `attach_choices`, the pure function behind the picker: which
conversations survive the typed filter, which of them are attached to a *different* tab and so
disabled, and which index (if any) is this tab's own current pick.

`crates/ubiq/src/state/agents.rs` defines `COLUMNS_MAX`, `CHATS_MAX` and
`COMPOSER_SLOTS = COLUMNS_MAX + CHATS_MAX`; `AgentsView::free_slot` still allocates a column's slot
from the low range, unchanged.

`crates/ubiq/src/app/chat.rs` is where a tab's own lifecycle lives: `open_chat_tab` mints one and
gives it a slot, `new_chat_tab` is what the strip's `+` runs, `attach_chat` sets or clears an
attachment, `chat_picks` builds what the control offers, `pick_chat_row` resolves a click against
that same list, `toggle_chat_picker` and `dismiss_chat_picker` own the control's open flag, and
`closed_chat_tab` is what a tab leaving the dock for good runs — dropping the `ChatTab`, clearing its
slot's draft, and touching nothing about the conversation it was looking at.

**Rows and the actions behind them are matched by position**, the rule every menu in this window
follows, so `state::chat::chat_picks` builds one list of `ChatPick`s — `Start`, `Attach` or `Inert`
— that the frame draws and the click resolves against. There is no second reading of the harness
list to drift out of step with the first. A `Start` resolves through
`AppState::start_harness_choice` in `crates/ubiq/src/app/agents.rs`, which the agents screen's own
menu also calls: the agent id is minted client-side there, before the host is asked to start it, so
the tab attaches with no round trip to wait on. What a harness row *says* is
`ui::agents::harness_offers`, the agents menu's own labelling rather than a second copy of it.

`crates/ubiq/src/app/panels.rs`'s `sync_chat_panels` is a chat tab's real population — squaring the
dock's tree with `OpenProject::chats` — called whenever a project is entered, right after
`OpenProject::new` has seeded that project's first tab, and again at the end of `settle_layout`, so a
restore that dropped an unfamiliar id is squared with the truth immediately. `toggle_region` mints a
fresh tab when the user reopens an emptied right region.

Rendering is two modules under `crates/ubiq/src/ui/chat/`: `mod.rs` resolves a tab's own attachment
once — `attached`, read by both children below rather than asked twice — and hands it to the shared
conversation renderer (`header: false`), or draws the empty page; `sidebar.rs` draws the one header
row: the state mark and the unified control on the left, the three-dots on the right. The control
itself is `crates/ubiq/src/ui/kit/menu.rs`'s `Picker`, unchanged — its `disabled` set draws the
headings and the rows another tab holds, its `separators` set the group lines, and its `search`
field the filter. A grouped, searchable, partly-inert list was already what that primitive did.

## Failure

| What happens | Result |
|---|---|
| A chat tab has nothing attached | Its page names what fixes it, rather than an empty transcript |
| The chat range's composer slots are all taken | The strip's `+` and a re-opened empty right region do nothing; there is no ninth slot to hand out, the same ceiling a ninth column meets |
| A harness is uninstalled between the draw and the click | `start_harness_choice` answers nothing, the tab stays attached to what it had, and the control shuts rather than sitting open over a row that did nothing |
| The remembered harness is gone | Nothing is preselected; the list opens as it would have on a fresh machine |
| A saved arrangement names a chat id this window never minted | The leaf is dropped, and the tree normalises around the gap, the same as an unfamiliar saved terminal leaf |
| A picker's filter matches nothing | The panel says so; a row already disabled is never what a filter with no matches is confused for |
| A menu is open and the user clicks elsewhere | The menu dismisses; no other menu opens on the same click |

## Related docs

- [`workbench.md`](./workbench.md) — the dock a chat tab is one panel in, and the composer slot pool it shares with a column
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the conversation family an attached tab speaks
- [`../tech/decisions.md`](../tech/decisions.md) — `D61`, why a tab is exclusive per surface and not per conversation
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens the transcript and its blocks are coloured from
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — who owns harness knowledge, for the rows the start half offers

## Next steps

- Persist a chat tab's arrangement and attachment across a restart, rather than seeding one fresh
  unattached tab per project every time a window takes it.
- Let a tool block open the file it names in the editor.
- Attachments that carry a real file rather than a chip.
