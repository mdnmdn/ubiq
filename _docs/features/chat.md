---
id: feat-chat
title: The chat panel
kind: feature
status: draft
summary: Editor-like chat tabs — many, movable to any dockable region, each a view onto a host-owned conversation or onto none, drawn by the composer, transcript and tool blocks the whole window shares.
read_when: you are changing a chat tab, the control that starts or attaches a conversation, or which conversation a tab shows
updated: 2026-09-08
verified: 2026-09-08
code_anchors: [crates/ubiq/src/ui/chat/mod.rs, crates/ubiq/src/ui/chat/sidebar.rs, crates/ubiq/src/state/chat.rs, crates/ubiq/src/state/dock.rs, crates/ubiq/src/app/chat.rs, crates/ubiq/src/app/panels.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/state/conversation.rs, crates/ubiq/src/app/agents.rs, crates/ubiq/src/ui/agents/mod.rs, crates/ubiq/src/ui/dock/skin.rs, crates/ubiq/src/state/prefs.rs]
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

**A running turn is drawn at the tail of the transcript.** While the run is `Working` and nothing is
waiting on a permission answer, the last thing in the transcript is `ui::conversation::writing_mark`
— three dots pulsing a third of a cycle out of phase, in the colour `Activity` already gives that
turn, with the activity's own word beside them. A turn can be a minute of silence between two
sentences, and silence reads as nothing happening; movement is the only honest thing to draw there,
since a spinner would claim progress nothing measures. It is not drawn while an ask is up: the
question on screen is what is happening, and two marks would compete to say so. The run is folded
into `tail_signature`, so the mark appearing scrolls the tail into view the way a new block does.

**Where a reader was left is remembered per transcript.** A composer slot's transcript keeps its
position in `state::conversation::TranscriptScroll`, keyed by the conversation *and* which of its
delegates is being viewed: switching to a delegate is arriving at a different transcript rather
than moving within one, so a slot moved to a delegate and back restores both positions. A
transcript the slot has never shown opens on its tail.

**The tail is followed only for a reader who is on it.** A transcript scrolled away from the bottom
stays where it was put while the conversation goes on writing, and `tail_signature` is read over
the blocks *on screen* rather than over all of them — while a delegate's transcript is up, the main
agent writing below it is not the tail of anything the reader can see, and following it would
scroll a transcript nothing was added to.

**A transcript scrolled away from the tail carries one overlay, `Go to last message`.** It sits
over the transcript's lower right rather than in the column, so nothing moves when it appears and
the last line stays readable under it, and it is drawn only while there is something below the
viewport — a button that is always there is a button that says nothing. It scrolls and does nothing
else: it marks nothing read, and the next thing said resumes the follow, which is what coming back
down asked for.

**A long transcript stops building what is off screen.** Above forty blocks, a child the last frame
painted well clear of the viewport — plus a margin, so a wheel notch lands on drawn content rather
than on a placeholder waiting for the next frame — is replaced by a stand-in of the exact height it
was measured at. Measured, never guessed: the content above and below stays where it was, so
nothing about the scroll position changes, and the markdown, the diffs and the highlighting of a
hundred blocks nobody is looking at go unbuilt. Below forty every block is built every frame,
because the bookkeeping costs more than the drawing and the first frame has no measurements to work
from. The same pass records which block each child is, which is how the strip above resolves a
block to a child to scroll to.

**A run of the same kind of tool call is folded to its last card.** Three or more consecutive tool
blocks of one kind — `GROUP_MIN` — are drawn as the last of them plus one `tool_group` row standing
for the ones before it, wearing that kind's own colour and reading `N earlier calls`, which opens on
click to show them all in place. Twelve `READ`s in a row are twelve rows of furniture between two
sentences, and what a reader is following is the last of them; two cards would become a row plus a
card, which is no shorter and one more thing to learn, which is where the floor of three comes from.
A `Delegate` call is never folded, because a spawned agent is a second transcript rather than a
step, and neither is a call with a permission ask attached, because a prompt behind a counter is a
turn that deadlocks. Which runs are open is `Conversation::open_groups`, keyed by the run's first
call id — UI arrangement the host never hears about, like a tool block's own open flag — toggled
through `AppState::toggle_conversation_tool_group`.

**A permission ask is drawn on the tool call it authorises, not in a dialog.** A harness that stops
to ask stops mid-operation, and the operation is already on screen: the prompt is joined to that
block by the tool call id, the block reads as awaiting approval, and the buttons sit under it in the
transcript. There is one button per `PermissionOption` the harness offered, labelled from the
option's own `name` and differentiated by its `kind` — allow from reject, with an "always" variant
marked as lasting. `kind` decides only how a button reads: the `option_id` is opaque, echoed back
unchanged, and nothing on this side interprets it or remembers a choice.

**The request carries an id and little else, so the prompt reads the call it is attached to.** The
`tool_call` on a request is a patch whose id is the only field guaranteed present, so the title, the
content and the diff are read off the call the transcript already holds. A request naming a call
this transcript has never seen degrades to a self-contained prompt at the end of the transcript,
which is a prompt with less to say rather than a question the user cannot answer.

**Several asks may be up at once, and every one of them blocks.** They are held as an ordered list
keyed by `request_id`, arrival order preserved, because a harness may raise a second question before
the first is answered and there are no timeouts: a request left unanswered stalls the turn with
nothing on screen to say so. A "needs you" strip above the footer carries what is
outstanding, since a prompt attached to a block partway up the transcript can be scrolled out of
view while the conversation waits on it. Cancelling the turn discharges all of them at once — the
strip and the prompts go with it.

**The strip is answerable, and it is the way to the prompt.** It carries Yes, All and No for the
oldest request outstanding, because the one control on screen while a turn is blocked should be the
one that unblocks it. Each button is drawn only where that request offered an option of that
reading — `Pending::option_for` for the plain allow and the reject, `Pending::always_option` for
the allow that is remembered — so a control never answers with a reading the harness did not offer,
and a third button standing in for a lasting allow would be a control that lies about lasting.
Clicking the strip's label goes to the question instead: `AppState::reveal_permission` switches to
whoever raised the request and scrolls the call it authorises into view, both from the one routing
`Conversation::pending_route` resolves, so the two ways of answering lead to the same place rather
than competing. A request naming a call the transcript does not hold routes to the tail, which is
where its self-contained prompt is drawn.

**The strip counts, and it names whose question it is.** Above one request outstanding, a count
badge sits beside the `NEEDS YOU` mark rather than a clause in the label: the number is the part a
reader counts down, and a trailing "and 3 more waiting" read as part of what the operation was. A
request a delegate raised carries that delegate's name, since with the main agent's transcript on
screen the operation alone is not enough to go on. The rest are answered by working through them
one strip at a time, because each request's options are its own.

**⌘⌥Y allows and ⌘⌥N rejects the oldest ask outstanding.** They answer the conversation being read —
the active tab of the agents screen's focused column — with the first allow-kind or reject-kind
option that request offered, and do nothing where it offered none of that reading rather than
answering with the other. Both are bound in the `Workbench` and the `Input` key contexts, so a
composer holding focus does not swallow the answer to a question blocking the very turn it is
typing into.

**Delegates are a line, and a list only when asked for.** A conversation that spawned subagents
grows one row at the top of the bottom block — above the footer, above the composer — reading
`3 active subagents of 10`, and nothing else; the `of 10` is dropped while every delegate is still
working — `3 active subagents` — and the row falls back to the bare `10 subagents` once none is,
because a count twice over and a zero are both noise. A conversation that spawned none grows
nothing. Opening it draws one row per agent *upward*, over the transcript, through the same `anchored` + `deferred` pair
every menu in the window uses, so the composer never moves under the cursor. Each row says who and
what it is doing, and clicking one switches the transcript to that agent; the main agent is always
a row, because it is the way back. A subagent whose spawning call is not in the transcript reads
`unknown` rather than being claimed to be running. **A delegate says what it is answering with**:
the reading strip above its transcript carries its model beside its name, its row carries the same
model faint beside the name — a delegate is chiefly identified by what it answers with, and a
reading only on hover made the reader hover three rows to compare three — and its row's hover names
its kind, its model and its thinking level where the harness stated one — the model shortened by
`short_model_label`, the same shortener the composer's model chip uses, so one conversation never
spells a model two ways. Both read the one `SubagentTab` field, resolved on `Conversation` beside
`subagents`. Nothing is borrowed from the parent: a delegate the harness named no model for draws
nothing, and `thinking` is `None` on every harness today because no stream states a per-delegate
effort level. The `AGENT` block that spawned an agent is the
same door: clicking it switches the transcript, and stays inert until that agent has said
something.

**A blocked delegate says so in place of what it was doing.** A row whose delegate is waiting on a
permission answer reads `need you` in the warning tokens where its status would be — `need you ×N`
above one request, and the bare words for one, because `need you 1` is a number nobody needed. In
place of rather than beside: a delegate waiting on a human is not doing anything, so `running` and
the question together would be one of them wrong, and the question is the more useful of the two
readings. `Conversation::pending_count` is what counts, resolved onto `SubagentTab::waiting` beside
the rest of the row, and the main agent's own row is read the same way.

**Files are attached to the turn being written, as tags rather than as text.** The composer's `+`
raises the window's own file picker over the project's explorer tree, taking as many files as are
wanted, and what comes back is one tag per file in a wrapping row directly under the token and
context readout and above anything queued — the turn's own furniture, immediately over the field it
belongs to. Clicking a tag opens that file in the editor; its `×` takes it off. A tag says the file
name, and its tooltip says the whole path from the project root with the size in figures, because a
tag has room for a name and nothing else.

**The colour of a tag is the size of the file.** Over 300 KiB it is drawn in the warning tokens and
over 500 KiB in the danger ones, with the reason in the tooltip beside the figure: a file large
enough to cost a noticeable part of the context window says so *before* the turn is spent on it. A
file no host reported a size for is drawn plainly — an unknown size is not a small one, and it is
not guessed at.

**An attachment belongs to the conversation, not to the composer slot drawing it.** It sits beside
the draft and the queue on `Conversation`, for the draft's reason: unsent composer content follows
the conversation from one surface to another rather than being lost when a tab is switched. Nothing
new crosses the bus for it — when the prompt is sent, every attached path is composed into that one
`PromptAgent` text as an `@path` mention after what was typed, and the attachments are consumed with
the draft. Which is also why attachments with nothing typed are still something to send. Enqueue
does the same composition into the queued text and clears them: a queued prompt is one string, and a
queue row that carried its own tag list would need its own tag row, its own removes and its own
colouring — a second composer. An edit brings those paths back into the field as the text they now
are.

**`ctx` is a level, `tot` is a flow.** The footer's ring and its `ctx` count are how full the
context window is *now* — a number that falls when the conversation is compacted — and `tot` is
every token the conversation has ever billed, subagents included, which only grows. That is why one
is a ring and the other a number, and every readout in the row says which it is on hover: the
identity chip, the ring, `ctx`, `tot` with its per-way and per-subagent breakdown, and the
composer's model, thinking and mode chips.

**A second ring beside `tot` says how much of that total was read back out of the cache.** It sits
at `cached_tokens` over `total_tokens`, in the `info` tokens rather than the accent ones — a second
accent ring beside the context one would read as the same fact twice — and says
`cached X / Y Z%` on hover. It is off unless asked for, because it is a cost-of-running reading
rather than a how-is-this-turn-going one and the footer row is glanced at; the setting that turns it
on is [`workbench.md`](./workbench.md)'s.

**The footer reports whoever is being read.** With a delegate's transcript up, `tot` and the cache
ring are that delegate's spend rather than the conversation's, off
`Conversation::subagent_tokens` — a reader looking at one agent's turns wants that agent's numbers,
and the conversation's own total is a click away on the main agent's row. Two limits of the wire
show through here, and the row states both rather than smoothing them. **A delegate's spend is
banked per subagent *type*, not per instance**: `UsageRecord::subagent` is deliberately a type, so
two `general-purpose` delegates share one bucket, which the `tot` tooltip says outright wherever
several of a type have run — nothing divides a shared total between instances to make it look
exact. **A delegate has no context level at all**: a subagent's usage report repeats the *parent's*
occupancy, so there is no per-delegate window to draw, and the parent's ring beside a delegate's
transcript would be a number about somebody else. The ring is dropped rather than borrowed, on the
same rule that keeps a ring off a conversation whose harness named no window.
[`../backlog.md`](../backlog.md) carries both as `G194` and `G195`.

**Stop is there for the whole of a running turn, and it is a filled square.** The moment a message
is sent is the moment a reader most wants it back, so a control that appears only while the field is
empty is a control that vanishes as soon as the follow-up is being typed. It sends `CancelTurn`: the
turn in flight ends, the conversation and the harness stay, and the next message goes to the same
agent — which is why it is a square and not a cross, a cross reading as *close this* and this
closing nothing. When a running turn also has something typed, Enqueue sits beside Stop, because a
turn is not interrupted by typing at it and what is typed goes out when it ends; an idle
conversation keeps the one Send button. All three are what Enter answers through
`AppState::send_or_enqueue`, so the buttons and the key never disagree.

**The status glyph and the three-dots lifecycle menu are the one exception: the tab's own header
draws them, not the shared view.** `ConversationView::header` tells the shared view whether to draw
its own bordered strip for them — `true` on the agents column, unchanged; `false` here, because the
chat panel's header draws the identical fragment inline instead — and at *opposite ends* of its
row, so it takes the two halves separately: `ui::conversation::lifecycle_mark` for the glyph and
`lifecycle_menu` for the three-dots. The agents column's bordered strip is the menu alone, its own
reading of the state being the dot on its title. One set of functions either way: the glyph's state and the menu's enable rule are read once, in
`crates/ubiq/src/ui/conversation/mod.rs`, and both surfaces call them rather than each keeping an
answer of its own.

**The glyph says the conversation's state; the word lives in its tooltip.**
`ui::conversation::lifecycle` reads `launched`, `run`, `pending`, `blocks`, `accepts_input` and
`config` into one `Lifecycle` — Starting, Ready, Waiting, Working (carrying which `Activity`), Idle,
Unloaded, or Ended — derived rather than stored, so nothing new sits on `Conversation` for it.
`Waiting` outranks the turn it is blocking: a request outstanding is the one state that needs the
reader to do something, so it is read before `run`, and `Working` therefore never carries
`Activity::NeedsYou`. `Unloaded` and `Starting` are both `launched == false`; the transcript,
`blocks`, is what tells them apart, because a harness that is gone still leaves what it said and one
never started leaves nothing. The glyph is a `kit::status_dot`, no new primitive, coloured by
`lifecycle_colour` — **yellow needs you, blue is working, green is idle, grey has stopped**, four
readings and only four, since what a dot read at a glance has to answer is whether this conversation
wants the reader; the tooltip is one or two words, `Unloaded`, `Working · Tools`, never a
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
same as every other screen that hosts one. A permission ask arrives as
`ConvUpdate::PermissionRequest` and leaves as one `AnswerPermission` naming the `request_id` and the
`option_id` pressed; `CancelTurn` is what answers the rest. Which surface drew the buttons is not on
the wire, so an ask answered here is answered for the conversation.

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
row: the state mark and the unified control on the left, the three-dots on the right. The permission prompt is that shared renderer's
too: `crates/ubiq/src/state/conversation.rs` holds `Pending` — the request id, the tool-call patch
and the options — in `Conversation::pending`, with `oldest_pending()` for what the keyboard means,
`answered()` for one request leaving, `Pending::option_for` for the first option of a reading,
`Pending::always_option` for the allow the harness remembers, `tool_block_index()` for the id join
the prompt draws through, `pending_subagent()` and `pending_route()` for whose transcript a request
belongs to and which block of it, and `pending_count()` for what a switcher row marks itself with;
`crates/ubiq/src/ui/conversation/mod.rs`'s `permission()` draws the block-attached prompt and its
fallback, and `needs_you_strip()` the answerable strip, with `waiting_count()` for its badge.
`AppState::reveal_permission` in `crates/ubiq/src/app/agents.rs` is what the strip's label runs,
taking the surface's own slot because the scroll belongs to the surface: the same conversation may
be open in a column and a chat tab, and only the one that was clicked moves. The same module
holds the rest of the transcript's own furniture: `transcript()` walks the visible blocks, folding
each run of same-kind calls into one `tool_group()` row plus the run's last card — `one_block()` is
the arm it reuses for a card it does not fold and for the ones it unfolds — `writing_mark()` is the
tail's running mark, and `tail_signature()` is what the follow-the-tail scroll compares, read over
the visible blocks it is handed. The private `Built` helper in the same module is the children that
walk produces: it records which block each child stands for, so a block can be resolved to a child
to scroll to, and above `TranscriptScroll::windows` it leaves a child the last frame painted clear
of the viewport unbuilt behind a stand-in of that child's measured height. `to_tail_button()` is
the overlay, on `AppState::scroll_transcript_to_tail`. `state::conversation::TranscriptScroll` is
the rest: `sync()` is the once-a-frame decision — save the outgoing transcript's position, restore
this one's, follow the tail only for a reader on it — with `away()` for the overlay, `request()`
and `take_request()` for the block the strip asked to be taken to, and `to_tail()`. `AppState`
holds one per composer slot as `transcript_scrolls`, indexed exactly as `column_inputs` is; every
field of it is interior-mutable, because `render` holds `&AppState` and these are readings of the
last frame rather than state the application owns. The fold's
open set is `Conversation::open_groups` with `toggle_group()` beside it in
`crates/ubiq/src/state/conversation.rs`, reached from the row through
`AppState::toggle_conversation_tool_group` in `crates/ubiq/src/app/agents.rs`. `footer()` reads
`show_cache_ring` off the workbench's UI settings and draws the cache ring from `cached_tokens()`
over `total_tokens()` — or, on a delegate's transcript, from `Conversation::subagent_tokens()`,
with `delegate_spend_tip()` for the tooltip that says which grain the figure is banked at; `stop_button()` is the composer's square, on `AppState::cancel_turn`, beside
the `action_button()` the Send and Enqueue states share.
`AppState::answer_permission` in `crates/ubiq/src/app/agents.rs` sends one answer and forgets that
one request only, `answer_oldest_permission` is what the keyboard resolves through
`read_conversation`, and `cancel_turn` clears the whole set. The `AllowPermission` and
`RejectPermission` actions are declared in `crates/ubiq/src/app/mod.rs` and bound by
`install_key_bindings` in both contexts. Attachments are the same division of labour. `crates/ubiq/src/state/conversation.rs` holds
`Attachment` — a stable per-conversation id, a project-relative path, and the size the picker
reported — in `Conversation::attached`, with `attach()` (dedupes by path, refreshing the size),
`detach()`, `clear_attached()` and `compose_prompt()`, which is the one place the `@path` mentions
are written. `crates/ubiq/src/state/file_picker.rs` owns the size vocabulary both the picker's rows
and a tag are drawn from: `size_label` for the figure, `SIZE_LARGE` and `SIZE_HUGE` for the two
thresholds, and `size_reading` for which of the three readings a size gets — one `KB` divisor, so
the colour and the printed number can never disagree. `AppState::attach_files` in
`crates/ubiq/src/app/picker.rs` is what the picker's commit routes into, taking each path's size off
the picker's own nodes rather than reading a disk the interface may not even be on;
`detach_file` is a tag's `×`. `crates/ubiq/src/ui/conversation/mod.rs`'s `attachment_tags()` draws
the wrapping row as the composer's first `extras` entry, on `kit::removable_tag`. The control
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
| The same file is picked twice, or picked again while already attached | One tag, not two — a second tag would be a second mention in the prompt and a remove that only half worked. The size on the tag it already had is refreshed from the newer listing |
| No host ever reported a size for an attached file | The tag is drawn plainly, with no size in its tooltip; an unknown size is not a small one and is not guessed at |
| An attached file is deleted or moved before the turn is sent | The mention goes out anyway and the harness answers for it; the interface reads no disk, so it has nothing newer to know |
| A permission request names a tool call the transcript does not hold | The prompt draws self-contained at the end of the transcript, with the options it carries, and the strip's label sends the reader to the tail where it is |
| A permission request offers no lasting allow | The strip draws Yes and No and no All; a third button answering with the plain allow would be a control that lies about lasting |
| A delegate's transcript is being read | `tot` and the cache ring are that delegate's spend, banked by subagent type; no context ring is drawn, because no harness reports a delegate's own occupancy |
| The transcript is scrolled up while the conversation goes on writing | It stays where it was put, the `Go to last message` overlay appears, and the follow resumes once the reader is back on the tail |
| A permission request offers no option of the reading ⌘⌥Y or ⌘⌥N asks for | The keyboard does nothing; the buttons the harness did offer are still there to press |
| A conversation reports a total but no cached figure, or a total of zero | The cache ring is not drawn; a ring at nothing over nothing is not a reading |
| The turn is cancelled while asks are up | The outstanding set is dropped, the prompts and the strip go with it, and the host answers every one of them as cancelled before the cancel reaches the harness |
| The harness ends or is unloaded while an ask is up | The prompts go with the process; there is nothing left waiting on an answer |

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
- Attachments that carry the file's *content* over the bus as a `ResourceLink` — which
  `agent-manager` already accepts — rather than an `@path` mention the harness has to resolve
  itself. The tags, their sizes and their lifecycle are built;
  [`../backlog.md`](../backlog.md) holds the wire half.
