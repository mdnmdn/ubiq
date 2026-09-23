---
id: feat-chat
title: The chat panel
kind: feature
status: draft
summary: Editor-like chat tabs — many, movable to any dockable region, each a view onto a host-owned conversation or onto none, drawn by the composer, transcript and tool blocks the whole window shares.
read_when: you are changing a chat tab, the control that starts or attaches a conversation, or which conversation a tab shows
updated: 2026-09-23
verified: 2026-09-23
code_anchors: [crates/ubiq/src/ui/chat/mod.rs, crates/ubiq/src/ui/chat/sidebar.rs, crates/ubiq/src/state/chat.rs, crates/ubiq/src/state/dock.rs, crates/ubiq/src/app/chat.rs, crates/ubiq/src/app/clipboard.rs, crates/ubiq/src/app/picker.rs, crates/ubiq/src/app/panels.rs, crates/ubiq/src/app/wire.rs, crates/ubiq/src/app/shell.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/ui/conversation/info.rs, crates/ubiq/src/ui/acp_capabilities.rs, crates/ubiq/src/state/conversation.rs, crates/ubiq/src/state/work.rs, crates/ubiq/src/app/agents.rs, crates/ubiq/src/ui/agents/mod.rs, crates/ubiq/src/ui/dock/skin.rs, crates/ubiq/src/state/prefs.rs, crates/ubiq/src/app/projects.rs, crates/ubiq/src/state/ask.rs, crates/ubiq/src/app/ask.rs, crates/ubiq/src/ui/ask.rs, crates/ubiq-proto/src/ask.rs]
depends_on: [feat-workbench]
review_cycle: monthly
---

# The chat panel

## Purpose

A harness in a terminal shows what an agent is doing; a chat tab shows what it was asked and what
it concluded, beside the code rather than in another window. It is furniture in four modes — IDE,
Tasks and both Teams screens, each of which is somewhere a conversation is part of the work — and is
hidden in the rest. Unlike every other panel IDE mode draws, it comes in many instances at once: a chat tab is
a perspective on a conversation the host owns, not a conversation of its own, so many may be open —
each attached to a different run, or to none — and closing one ends nothing. The relation holds in
one direction only: deleting the conversation closes every tab looking at it, because there is then
nothing to be a perspective on.

## Behaviour

**A chat tab is `PanelKind::Chat(ChatId)`, one panel per open instance.** The id is minted the way
`AgentId::generate` mints one, but locally: it is UI arrangement the host never hears about, the
same as a file's tab key names a document the host does hear about. Several tabs coexist, dragged
apart, tabbed together, or moved to any dockable region — left, right, bottom or the centre — the
same freedom a terminal panel already has. Its default home is the right edge, at `CHAT_WIDTH`.

**A tab is attached to a conversation, or to nothing.** The attachment is `state::chat::ChatTab`, one
entry per tab in the project's own `OpenProject::chats`, holding the tab's id, its composer slot,
whether its control is down, and the `AgentId` it is looking at. Closing a tab drops the
`ChatTab` and frees its slot; the conversation, if it had one, is the host's and keeps running.

**One control chooses what a tab is looking at, and its first row starts something new.** A tab
either shows a conversation or it does not, and what the user wants in each case is a different
thing — begin something, or move to something already running. Two controls side by side made the
user pick the question before answering it; one control whose first row starts and whose rest
attach does not. **It is a bare chevron**, in the tab's own header, and it says *change agent* on
hover: what the tab is showing is already said twice on that row, by the dock's tab and by the
hexagon it now wears, so a label here would be a third copy of the conversation's name wearing a
control's clothes.

**Its first row is *New agent*, and it raises the window's start form.** Which harness, as whom,
on which model, at which reasoning level and under which permission mode are all one question,
asked together, and *The workbench* is where the form is described. The flattened
harness-and-identity list this control used to carry above its conversations is gone with the row
that started from it: it launched with every question but the first skipped. The titlebar carries
its own shortcut to the same start — `AppState::open_new_agent_direct` aims at the chat surface in
IDE mode and in Tasks (`T-109`: the board's own `+ New agent` reaches it too, so a start from there
lands in the right dock beside the task rather than jumping to the agents screen) the way this row
does, the `+` menu's first stage skipped either way.

**An attached tab is offered the start too.** Starting from a tab already showing a conversation
would once have left the first with no view; the conversation it leaves is on this very list, one
click from coming back.

**Everything under it is a conversation to move to.** The rows are the conversations this window
actually holds — `AgentsView::live`, the same set the agents columns may open on — under a
hairline, drawn only when there is something below it: a divider over nothing reads as a group that
failed to load. Typing filters them, and the hairline goes with the list when a query empties it.

**The host's projection is wider than that, and the difference is not offered.** It also carries the
fixtures the mock work thread seeds into every project, which have a name and an activity and
nothing behind them; a tab attached to one would be a transcript with no harness at the other end.
`state::chat::attach_choices` takes the live set and narrows to it before the query narrows
anything, so every surface that offers to attach — this chevron and the `+` menu's second stage —
answers the question the same way.

A conversation already attached to a *different* chat tab draws disabled and cannot be picked; it is
never dropped from the list, because a row that vanishes reads as a conversation that ended rather
than one taken. The tab's own current attachment stays selectable, since it is the row already
checked.

**Exclusivity is per chat tab, not per conversation, and it stops at this surface's edge.** The
agents workbench may show the same conversation in a column, and the Teams inspector may draw it at
`TEAMS_SLOT` — under the window span from a project this window is not pointed at — at the same
moment a chat tab is attached to it. Three viewers at once, and the host is never told which surfaces
are looking, because a view was never the workspace.

**Adding a view is the tab strip's gesture, not the panel's.** A `+` on the dock's own tab strip —
beside the terminal region's, offered on the strip of any group holding a chat *or sitting at the
chat home region with none yet* — opens the window's two-row `+` menu: *New agent*, or *Attach
existing agent*. The second clause is what puts the `+` on Tasks' right region: that group holds
the task, not a chat, until one is started or attached from here, and the strip has to offer the
gesture on the strength of the region alone the way the pane `+` already does for an emptied pane
region (`AppState::is_chat_region`, `ui::dock::skin::NewChat::region`). A chat dragged into the
editor region takes the control with it rather than leaving the gesture behind, since the same
`hosts_chats` half of the rule still applies. The strip's `+` still means "add a view"; the menu
only answers what the view will be looking at, and **the tab is minted when there is something to
put in it** — a pick attaches at once, and a start form opens the tab from
`Message::ConversationStarted`, so a form the user dismisses leaves no empty tab behind.

**The header is the one first line every conversation-hosting surface draws**
(`ui::conversation::lifecycle_header`), not this panel's own row: the three-dots menu at the left,
this tab's own change-agent chevron beside it, and the current-action chip flush against the
strip's right edge. There is no state mark here any more — a chat tab's state is the hexagon the
dock's own tab wears, the same mark the agents column's tab wears (T-99, T-102). Nothing attached
draws the chevron alone: there is no menu and no chip with no conversation to read either off.

**Nothing in the header names the tab.** The dock's tab already carries the conversation's name, and
a second copy of it directly under the first was the same answer twice.

**The dock tab's name is whatever the conversation is called, including a name Ubiq wrote itself.**
Once an agent has answered the opening prompt, the host reads that exchange and names the
conversation — the title lands on the tab in place of the harness-and-counter it started with, and
the five-word summary that comes with it is the tab's hover, so the strip stays a strip and still
says what each tab is about. A tab attached to nothing reads `New chat` and hovers to nothing,
because there is no conversation to say anything about. The rule the naming follows, the surfaces it
reaches beyond this one and the checkbox that switches it off are the workbench's; the message
behind it is the conversation family's `ConversationNamed`
([`../tech/transport-contract.md`](../tech/transport-contract.md)), and `D90` is why a name Ubiq
invented may be replaced by one it read.

**Before that naming lands, the tab shows the profile it was started from rather than the bare
harness label (T-102).** `AppState::agent_title` is what every surface that used to print
`WorkAgent::name` directly reads now — the dock tab included — and it prefers the session-only
`agent_started_profile` record over the harness-and-counter default for as long as
`WorkAgent::summary` is `None`. See [`agent-vocabulary.md`](../wip/agent-vocabulary.md) for where
that record is kept.

**Closing the last chat tab is allowed.** There is no last-tab guard anywhere in this tree, and a
chat tab is no exception: closing the only open one leaves nothing behind but a tab strip with
nothing in it, which the region then puts itself away rather than sit empty. Opening the right
region again — the titlebar's switch, or the `+` past the last group's tab strip — mints a fresh
tab, attached to nothing, because that is the one place the window has to decide *which* instance an
empty region opens onto. A *pinned* tab is the one exception: pinning withholds the × and, on the
shared tab menu, Hide — Rename, Close and Pin (or Unpin) still work, the same as any other tab,
because ending the conversation behind a pinned tab is still the user's call — `AppState::tab_names`
and `AppState::pinned_tabs`, in memory only, because a
`ChatId` is reminted every run. **A persistent attachment is what survives one**: `ViewPrefs.chats`
remembers which agent each *persistently* attached tab held — a tab whose conversation the host was
not asked to keep is written down as nothing, exactly as an unattached tab is, because the boot
sweep deletes the run directory and reviving the tab would draw an empty panel over a conversation
that is gone (`D97`). So a restart brings back exactly the tabs whose conversations the host kept.

**Every chat tab draws from the same shared conversation view.** What a tab shows for its attachment
is `crates/ubiq/src/ui/conversation`, the transcript, the tool blocks, the footer and the composer
every surface that hosts a live agent shares — the run pill, the context ring, the quota ring, the
token cost, the launch-time model and thinking pickers. **A tab unattached to anything is one large play button**,
centred, on `start a new agent` — no title and no note, because a tab with no conversation in it is
the ordinary state of a fresh tab rather than a fault, and two lines of explanation said what the
icon says. It is drawn at `EMPTY_START_SIZE` rather than through `kit::icon_button`: that is the
window's chrome control, a fixed small square, and this is the page's whole subject. **A link in a rendered reply goes where it
points**: the transcript hands `ui::on_link` to its `TextView`, so a relative path resolved from the
project root or a full `ubiq://` opens that place in the window, `http`, `https` and `mailto` reach
the operating system, and anything else does nothing — see
[`workbench.md`](./workbench.md). A path merely *mentioned* in prose is text, not a link.

**A conversation surface resolves the agent's own project, never the window's active one.** Which
project a conversation belongs to is `AppState::project_of_agent`, and the two readers over it —
`teams_conversation` for the conversation and `teams_agent` for the host's record — are what every
surface drawing a live agent goes through: a chat tab, an agents column and the Teams inspector
answer the question the same way, from the project that owns the agent. The Teams screen is where it
bites. Under the window span its canvas draws every project the window holds, and the card its
inspector draws may belong to a project the rail is not pointed at (`D154`); a lookup through the
active project answers `None` for that card, and `None` is indistinguishable from a conversation with
nothing in it — so a turn is dropped, a flag reads *off* while it is on, or a pending list stays
stale, each of them silently. Every read and every write below — the send, the enqueue, the recall,
the permission answers, the folds, the attachment edits — goes through those three accessors for that
reason.

**The footer's third ring says how much of the account's plan is left, one band per rolling window
the provider stated.** It is an account fact, not a conversation one — two agents signed in as the
same identity read one window — so it is drawn from what the host cached for that account, falling
back to the reading the harness pushed into this conversation while nothing has been asked. A
provider that states two windows draws two concentric bands, the shorter window outermost — the one
that stops the next turn first — and the longer one inside it; a provider that states one draws the
single ring the surface always drew. Each band takes its colour from the usage thresholds rather
than the accent, because a ring's whole job here is to make "nearly out" visible without a hover, and
because two accent rings side by side would read as one fact drawn twice. The figure, the window's
name, its reset, the plan and the age of the reading are in the tooltip for every window drawn: the
bare `5h N%` readout belongs to the chrome and does not return there (`D111`). A conversation with no
account draws none, because there is no plan to have a window in; a provider that named no limit
draws none rather than an empty ring; and a delegate's transcript draws none, on the same rule the
context ring beside it follows.

**A running turn is drawn at the tail of the transcript.** While the run is `Working` and nothing is
waiting on a permission answer, the last thing in the transcript is `ui::conversation::writing_mark`
— three dots pulsing a third of a cycle out of phase, in the colour `Activity` already gives that
turn, with the activity's own word beside them. A turn can be a minute of silence between two
sentences, and silence reads as nothing happening; movement is the only honest thing to draw there,
since a spinner would claim progress nothing measures. It is not drawn while an ask is up: the
question on screen is what is happening, and two marks would compete to say so. The run is folded
into `tail_signature`, so the mark appearing scrolls the tail into view the way a new block does — and
so is how many prompts are outstanding, since a permission ask is not a block: an ask arriving while
the last block on screen is unchanged (a tool call still `InProgress`, waiting on the harness to ask)
would otherwise move nothing `tail_signature` reads, and the row the reader most needs to see would
land under the fold with no follow to bring it up.

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

**Being on the tail is sticky: only the reader takes a transcript off it.** The one movement content
cannot cause is the offset *rising* between two frames, so that — by more than `TAIL_SLACK`, and
only while there is still something below the viewport — is what says the reader left, and the
follow resumes the moment they are back within the slack. Growth alone never ends it: a row that
lays out taller than it last measured raises the list's maximum while the pinned offset stays where
it was, which looks exactly like a reader who scrolled away and would otherwise kill the follow for
the rest of the turn. The frame after the transcript pins itself to the bottom is discounted for the
same reason, since that pin is clamped at paint and reads back as a large move nobody made.

**A transcript scrolled away from the tail carries one overlay, `Go to last message`.** It sits
over the transcript's lower right rather than in the column, so nothing moves when it appears and
the last line stays readable under it, and it is drawn only while there is something below the
viewport — a button that is always there is a button that says nothing. It scrolls and does nothing
else: it marks nothing read, and the next thing said resumes the follow, which is what coming back
down asked for.

**The transcript is a virtual list, whatever its length.** A frame builds a *plan* — one `Row` per
thing on screen, arithmetic and no elements — and `gpui_component::v_virtual_list` builds only the
rows it can see. Three blocks or three thousand, a frame costs what is in the viewport.

**A variable-height list is told its heights before it lays one out, so the transcript remembers
what every row measured.** `TranscriptScroll` keys them by the row's *identity* rather than its
position — a fold opening moves every row below it — against the content, the width **and the
conversation family's body size** they were measured at: raising that base restates every height
the way a resize does. A row unchanged since it was drawn is its measurement; one whose content has
moved is its *last*, because a block that grew by a line is a line taller than it was; one never
drawn is an estimate of a line of prose per eighty characters. Either of the last two is measured by
the frame that draws it, which asks for one more, so an estimate lasts a frame. The plan also records which block each row stands for, which is how the strip above
resolves a block to a row to scroll to.

**Two things in the transcript fold, and their rules differ.** A run of same-kind tool calls needs
three before folding pays and keeps its last card out; a run of reasoning folds unconditionally into
one box. Both judge a run over the blocks *on screen* — contiguity is read across `visible_blocks()`,
so another subagent's blocks, drawn nowhere, do not break a run, while a sentence or a tool call
between two thoughts does.

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

**A run of reasoning is one bordered `THINKING` box, and the box is a disclosure.** Every
consecutive `ConvBlock::Thought` on screen is a child of the same box rather than a box of its own,
with no floor to reach first: where a harness chose to flush its reasoning is not something the
reader asked about, and a stack of identical frames says only that it flushed several times. The box
is expanded while the thinking is the one thing still being written — the run is read by its last
block's own `open` — and `Conversation::end_open_thought` collapses it to the caption alone the
instant a different block starts, a turn ends or the harness compacts, because once anything else is
being written there is nothing left to watch. Clicking the caption moves the whole run, never half a
box, and marks every block in it in `Conversation::touched_thoughts`, so the reader's choice is
never overridden by the next chunk. Toggled through
`AppState::toggle_conversation_thought_group`.

**A permission ask is drawn on the tool call it authorises, not in a dialog.** A harness that stops
to ask stops mid-operation, and the operation is already on screen: the prompt is joined to that
block by the tool call id, the block reads as awaiting approval, and the buttons sit under it in the
transcript. There is one button per `PermissionOption` the harness offered, labelled from the
option's own `name` and differentiated by its `kind` — allow from reject, with an "always" variant
marked as lasting. `kind` decides only how a button reads: the `option_id` is opaque, echoed back
unchanged, and nothing on this side interprets it or remembers a choice.

**A conversation set to accept everything is shown no ask at all.** The three-dots menu's
*Accept all* (`SetConversationAcceptAll`, described with the rest of the menu on the agents screen) is answered
in the host: the harness asks what it always asks, the host replies with the plain allow, and no
`ConvUpdate::PermissionRequest` reaches the window. Never shown rather than shown and then
withdrawn, because a prompt here is joined to a tool call in a transcript and nothing takes one
back. What that costs is that the transcript holds tool calls whose authorisation the reader was
never offered — the reason the flag is per conversation, off by default, and labelled on the menu
while it is on. A request the harness offers no allowing option for arrives as normal and is drawn
as normal.

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

**A structured question from `ubiq-ask` is not a permission ask, and it is drawn as a dialog rather
than on a block.** `AskUser` arrives from a tool call the host parked, not from the harness's own
protocol, and is filed on the conversation as an `AskRecord` (`crates/ubiq/src/state/ask.rs`) rather
than a `Pending` — a side channel joined by id, on `pending`'s own reasoning. Where the conversation
is already on screen and no other dialog is up, the ask modal opens on it directly; anywhere else, a
notification is raised instead and the transcript carries an "Ask for feedback" row (drawn by
`ui::ask::transcript_entry`) whose button reopens the same dialog with whatever was already typed
into it — closing the dialog never touches a draft, only the view over it. The modal is a tab strip,
one tab per question, each a column of option cards (single- or multi-select, by the question's own
flag) plus an always-offered "Other" and a notes field; Confirm sends `AnswerAsk` with the picks by
label, and "Chat about this" sends it with no question answered and lets the user type instead. Both
are real answers, and either one moves the record out of `AskStage::Waiting` — after that, and after
an `AskEnded` naming a timeout or a gone conversation, every later opening is the same dialog with no
controls in it: the questions and what was chosen (or that nothing was), read-only. **No marker
beside an option's label** — the whole card is the target and picked reads as the card's own fill
and edge, the same rule `kit::card`'s `selected` draws everywhere else. The tab strip stays put while
only the question below it scrolls, so a tall list of options never pushes the strip itself out of
reach.

**The transcript row reads as `warning` while it blocks the harness, and as `accent` once it
does not.** `theme::warning`/`warning_soft` are the same tokens `ui::conversation::permission`
draws its own "NEEDS YOU" row in — an unanswered ask is the same kind of thing a reader has to
notice — and the row falls back to the ordinary accent shape the moment it leaves
`AskStage::Waiting`, confirmed, chatted away, timed out or the conversation gone. **It is placed
where the ask actually arrived, not pinned to the transcript's tail**: `AskRecord::at_block` is how
many blocks the conversation held the instant the tool call parked, and `ui::conversation::plan_rows`
inserts the row just before the first block-anchored row at or past that count — the point the ask
interrupted — so it stays there as later turns land underneath it rather than trailing behind them.

**Keyboard reaches the whole of it.** Up/down walk a cursor over the question on screen and space
picks or unpicks wherever it sits, exactly as a click on that card would; plain `enter` does the
same and moves on to the next question, and `⌘⏎` moves on by itself — confirming the dialog outright
from the last one, since there is nowhere left to move to. None of the four fires while "Other" or
"Notes" holds the keyboard, so typing a space or an arrow key in either still types; `tab` there
moves between them instead, the modal's only pair of fields. This closed `G327`.

**⌘⌥Y allows and ⌘⌥N rejects the oldest ask outstanding.** They answer the conversation being read —
the active tab of the agents screen's focused column — with the first allow-kind or reject-kind
option that request offered, and do nothing where it offered none of that reading rather than
answering with the other. Both are bound in the `Workbench` and the `Input` key contexts, so a
composer holding focus does not swallow the answer to a question blocking the very turn it is
typing into.

**Delegates and the todo list are two chips on one activity bar.** The bottom block draws a single
row spanning its full width, above the footer and below anything queued, with a subagent chip on
the left, a todo chip on the right and a flexible spacer between them so a lone chip still reaches
its own edge of the block rather than hugging the other's; neither chip appears when its source is
empty, and the bar itself draws when at least one chip is present.

**The left chip is the subagent count.** A chevron points upward when the panel is closed and
downward when it is open, beside a label built by `subagent_count_label`: `3 active subagents of 10`
while seven delegates have finished, `3 active subagents` while all three still run — dropping the
`of 3` because it repeats the same number — and the bare `10 subagents` once none is active,
because a count and zero are both noise. A conversation that spawned no subagent draws no chip.
Clicking opens a panel through the same `popover` pair every menu in the window uses, anchored to
the chip and drawn upward over the transcript so the composer never moves under the cursor. The
panel carries one row per agent: the main agent first, always present — it is the way back — then
each delegate from `subagents`. Each row says who and what it is doing: its name, its model shortened
by `short_model_label` where the harness stated one, and its status or a blocked message. **A blocked
delegate says so in place of what it was doing.** A row whose delegate is waiting on a permission
answer reads `need you` in the warning tokens where its status would be — `need you ×N` above one
request, and the bare words for one, because `need you 1` is a number nobody needed. In place of
rather than beside: a delegate waiting on a human is not doing anything, so `running` and the
question together would be one of them wrong. `Conversation::pending_count` counts, resolved onto
`SubagentTab::waiting` beside the rest of the row, and the main agent's own row is read the same
way. **Every other row says the precise pair** — `state::status::delegate_status` for a delegate,
`conversation_status` for the main agent, rendered as `Status::label()`: `Working · Tools`,
`Ended · Done`. A subagent whose spawning call is not in the transcript reads `Starting` rather than
being claimed to be running, and one whose call reached `Completed` reads `Ended · Done` and drops
out of the `active` half of the count above, because a delegate that came back is not running
however long its row stays on the list. Clicking a row switches the transcript to that agent and closes the panel;
the main agent's row closes it without switching. **A delegate says what it is answering with**: the
reading strip above its transcript carries its model beside its name, its row carries the same model
faint beside the name — a delegate is chiefly identified by what it answers with, and a reading only
on hover made the reader hover three rows to compare three — and its row's hover names its kind, its
model and its thinking level where the harness stated one, the model shortened by `short_model_label`,
so one conversation never spells a model two ways. Both read the one `SubagentTab` field, resolved on
`Conversation` beside `subagents`. Nothing is borrowed from the parent: a delegate the harness named
no model for draws nothing, and `thinking` is `None` on every harness today because no stream states
a per-delegate effort level. The `AGENT` block that spawned an agent is the same door: clicking it
switches the transcript, and stays inert until that agent has said something.

**The right chip is the todo count.** It reads `{done}/{total} todos` with a chevron, and is hidden
when the plan is empty — an opencode conversation with no `todowrite` calls sees no chip at all.
Clicking opens the same kind of popover panel: up to eight entries, each a status mark beside its
text — `✓` for completed, `▶` for in-progress, `○` for pending — with `… N more` when the list
exceeds eight. Clicking a row closes the panel; the transcript is not switched, because a todo row
has no agent to jump to. Exactly one panel is open at a time: clicking the subagent chip while the
todo panel is open closes it and opens the subagent panel, and vice versa, so two overlapping lists
for the same question never appear at once.

**Files are attached to the turn being written, as tags rather than as text.** The composer's `+`
raises the window's own file picker over the project's explorer tree, taking as many files as are
wanted, and what comes back is one tag per file in a wrapping row directly under the token and
context readout, immediately over the field it belongs to — the turn's own furniture. The queue sits
elsewhere: it is not this row's neighbour any more, but the topmost thing in the whole bottom block,
above the activity bar and the footer as well as the composer. Clicking a tag opens that file in the
editor; its `×` takes it off. A tag says the file
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

**The typed text follows the same rule, through `Conversation::draft`.** Every keystroke in a
column or chat composer mirrors into the addressed agent's own `draft` field
(`AppState::remember_conversation_draft`), beside its attachments — not only into the composer
slot's own copy, which is a fact about the window's furniture and is dropped whenever that slot is
freed (a tab closed, a column an arrangement change moved the agent out of). Reattaching to that
conversation — the chat header's *Attach running*, a bench pick into a column — puts it back
(`AppState::restore_composer_draft`), only into an empty field, the same guard
`recall_last_message` uses. A slot that keeps showing the same agent throughout — a mode switch, a
region hidden and reopened — never touches either copy: the pooled `Entity<TextareaState>` itself
is untouched by dock placement, so nothing is lost there in the first place.

**Pasting into the composer attaches, when the board carries a file.** `⌘V`/`Ctrl+V` with the field
focused reads the pasteboard before the field does: a copied *file* becomes a tag under its own path
— project-relative when it is inside a project this window holds, which is the reading a drop onto
the window makes, and absolute when it is outside every one of them — and a copied *picture*, a
screenshot or an image lifted out of a browser, becomes a tag too. Only the first path of a
multi-file copy is taken. A board carrying only text is not this gesture at all, and the keystroke
goes back to the field and types the text in as it always has. The chord is bound twice for that:
once for the workbench and once for `Workbench > Input`, because the library's own field binds it at
the deepest node and would otherwise swallow it.

A pasted *file* is made relative only to **the agent's own project**. A window can hold several,
and a file copied out of one of the others is attached absolutely: `src/lib.rs` is a path two
projects can both have, and the relative form would name a different, existing file that the
harness would then read without a word.

**A pasted picture is written into the project first**, under `.ubiq/pasted/`, through
`WriteProjectFile`. It is on no disk and an attachment is an `@path` mention, so there is nothing to
attach until it has a path. **That is a file written into the user's project**, deliberately:
`.ubiq/` is already Ubiq's own folder inside a project, beside `.ubiq/kb`. The name is the
millisecond it was pasted, so a later session never repoints an older turn's tag at a newer
picture. What comes out is an ordinary attachment — the same tag, dedupe, size warning and `@path`
on send.

**The folder ignores itself.** The first picture written into a project writes
`.ubiq/pasted/.gitignore` holding `*` beside it, through the same `WriteProjectFile`. A stored
knowledge base under `.ubiq/kb` is something the user asked for by adding a source; a pasted
screenshot is the side effect of a keystroke, and a folder of untracked binaries nobody chose has
no business in `git status`. The user's own `.gitignore` is never touched — that is a tracked file
nobody asked to change, and a rule appended to it would then have to be merged, deduplicated and
unwound.

**The bytes are re-encoded as PNG where this build can.** The only reason to write the picture at
all is for a harness to open it, and harness image support is png/jpeg/gif/webp — while a macOS
screenshot arrives on the pasteboard as TIFF and a Windows DIB as BMP. Those two are decoded and
written as PNG, which is lossless both ways, and the name follows the bytes. A format with no
decoder compiled in, or bytes that will not decode, is written exactly as it arrived under its own
extension: `G249` is what that still leaves unreadable.

**An optimistic chip is taken back off when its write fails.** The tag goes up on the send rather
than on the host's answer, so the interface tracks the writes it has outstanding; a
`ProjectFileError` for one of them detaches the tag and raises a notification, because a pasted
picture is in no editor tab and the ordinary save-failed path would never see it.

**A sent turn keeps its chips, and clicking one previews the file.** The tags do not vanish on Send:
they are drawn inside the turn's own accent surface, under the prose, because they were part of the
message rather than a footnote to it, and they carry no `×` — the harness already has the file. The
echo is one string of `@path` mentions and says nothing about which of them were tags, so the
conversation carries the list across the send itself, much as it carries a start's preamble.

**The carried list is a queue, one entry per send.** Two Sends can be in flight before either
echoes, and each keeps its own files in order; a plain send takes its place in the queue too, empty,
because the queue is only in step with the echoes if every send is on it. An entry is spent by the
**real text** echo alone — a chunk with no text in it, and Claude Code's synthetic
`[Request interrupted by user]`, are the harness talking rather than the turn, and either eating an
entry would draw the real message bare. A turn that can no longer echo **discharges** the whole
queue instead: a turn that failed or stopped with an error, a conversation unloaded, a conversation
ended. Nothing outlives the turn that armed it, so no later turn ever draws a chip for a file the
user did not attach to it.

Clicking one opens an anchored panel holding the picture — the editor's own image viewer, over bytes
read through `ReadProjectFile` inside the project and read directly for an absolute path — with the
name, size and path under it and an `Open` that hands the file to the editor. The answer is matched
on the project as well as the path, since one path can name a file in two of the projects a window
holds. A read in flight says so; a read that failed says *why* rather than waiting forever; a file
larger than the read ceiling says it is too large to preview, because a prefix of an image is not an
image and would draw an empty box; and a file the viewer declines says that instead of drawing
nothing. A panel rather than a modal because it asks nothing and blocks nothing: the window's single
open-menu slot holds it, so Escape and an outside click already peel it — and opening *any* menu
clears it, which is what keeps "`Some` exactly while that menu is open" true in both directions.

**`ctx` is a level, `tot` is a flow.** The footer's ring and its `ctx` count are how full the
context window is *now* — a number that falls when the conversation is compacted — and `tot` is
every token the conversation has ever billed, subagents included, which only grows. That is why one
is a ring and the other a number, and every readout in the row says which it is on hover: the
ring, `ctx`, `tot` with its per-way and per-subagent breakdown, and the composer's own identity,
model, thinking and mode chips. On the conversation's own transcript, `tot` also names the uncached
part of it right beside the total — `X tot · Y in`, `in` being the fresh input tokens
(`TokenSpend::input`) that were neither a cache read nor a cache write — the one figure the
breakdown otherwise held back for the hover. A delegate's spend is banked by type with no such
split behind it, so its `tot` stays a bare total. Every raw count in the row and its tooltips —
`tot`, `ctx`, the cache ring's `cached X / Y` and the context ring's `X of Y tokens` — goes through
`state::work::format_tokens`, the one place "how big is this number" is spelled: plain under a
thousand, then `k`/`M`/`G` at one decimal place.

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

**A cancelled turn's own echo is not a message.** Claude Code answers a cancelled turn with a
synthetic user-role chunk of its own — `[Request interrupted by user]`, or the tool-use variant —
and nobody typed it, so `Conversation::apply` drops it rather than pushing a `ConvBlock::User` for
it: it is not drawn, and it is not what `recall_last_message` hands back to a reader pressing Up in
an empty field, which reads the transcript for what was actually sent.

**The three-dots menu and the current-action chip are the one row the shared view itself draws.**
`ConversationView::header` tells `conversation::render` to call
`ui::conversation::lifecycle_header` — `true` on the agents column, which passes no `switch`; `true`
here too, with this panel's own change-agent chevron handed in as `switch`, so the row's shape and
the chip are one function either way rather than a fragment each surface assembles for itself. The
chevron sits **between** the menu and the chip, not beside either alone — `lifecycle_header` draws
`[menu, switch?] ... chip` — and it is `start_control` below, the same control whether or not a
conversation is attached. The three readers behind the menu — `is_persistent`, `accepts_all` and
`dump_path` — take the host's record through `teams_agent`, which is what makes one set safe to
share: a row that read `None` would report *off* for a flag that is on, and its toggle would send
*enable* every time, leaving a flag that could never be turned back off.

**The glyph says the conversation's state; the word lives in its tooltip.**
`state::status::conversation_status` reads `launched`, `run`, `stop_reason`, `pending`, `blocks`,
`accepts_input` and `config` into one `Status` — **a pair, not one enum**: a `Lifecycle` (Starting,
Ready, Idle, Working, Waiting, Unloaded, Ended) saying whether the conversation can do anything
next, and a `Doing` (Queued, Thinking, Writing, Tools, NeedsYou, Done, Failed, Unknown) saying what
it is busy with or how it stopped. Both are derived rather than stored, so nothing new sits on
`Conversation` for either. **The two dictionaries are the same ones a delegate speaks** — that is
the point of them being in `state::status` rather than in a UI module — so a main agent, an agent
card and a subagent row all report one vocabulary. `ui::conversation::lifecycle` is the first half
alone, for the surfaces that draw a dot.
`Waiting` outranks the turn it is blocking: a request outstanding is the one state that needs the
reader to do something, so it is read before `run`, and the `Working` lifecycle therefore never
carries `Doing::NeedsYou`. `Unloaded` and `Starting` are both `launched == false`; the transcript,
`blocks`, is what tells them apart, because a harness that is gone still leaves what it said and one
never started leaves nothing. The colour is `lifecycle_colour` — **yellow needs you, blue is
working, green is idle, grey has stopped**, four readings and only four, since what a mark read at a
glance has to answer is whether this conversation wants the reader; the tooltip is one or two words,
`Unloaded`, `Working · Tools`, never a sentence — replacing the muted line P7 drew above the
composer for the same fact.

**A chat tab wears the same hexagon its conversation wears anywhere else (T-99, T-102).** The dock
draws `ui::teams::status::status_mark` in place of the plain dot for a chat tab — `TabInfo::dot_status`
rather than `dot_colour`/`dot_pulse`, which every other tab kind still uses — the outer ring reading
`state::status::Status::lifecycle` and the core reading `Doing`, pulsing only while the lifecycle is
`Working`. A tab attached to nothing has no mark: there is no state to report. See
[`ui-and-design.md`](../tech/ui-and-design.md) for the shape rule the element belongs to.

**Each tab owns a composer of its own, from the same fixed pool a column draws from.** The window
builds `COMPOSER_SLOTS` text areas — `0..COLUMNS_MAX` for columns, the range above it for chat tabs
— before the first frame, because the *subscription* that mirrors what is typed has to be held for
the window's life. What was typed at one tab never turns up in another's field, and closing a tab
clears its slot's draft before handing the slot to the next tab that opens.

## Contract

A chat tab's own state — its id, its slot, its attachment, whether its picker is down — is local to
the UI, the same as which column an agent's conversation is drawn in. No message names a `ChatId`;
the host answers only about conversations, never about which surface looks at one, and what a
restore remembers travels in the interface's own opaque view blob. Once attached, a tab speaks
whatever [`../tech/transport-contract.md`](../tech/transport-contract.md)'s conversation family
carries: a permission ask arrives as `ConvUpdate::PermissionRequest` and leaves as one
`AnswerPermission` naming the `request_id` and the `option_id` pressed, `CancelTurn` answering the
rest. Which surface drew the buttons is not on the wire, so an ask here is answered for them all. A
question raised through `ubiq-ask` speaks the same family's `AskUser`/`AnswerAsk`/`AskEnded` trio
instead, and is filed as an `AskRecord` beside the conversation rather than as a block, since it is
a tool call the host parked, not a step of the harness's own turn.

## Implementation

`crates/ubiq/src/state/dock.rs` holds `ChatId` — a locally minted counter, `Display` and `FromStr`
so it round-trips through the dock's saved payload the way a pane's id does — and
`PanelKind::Chat(ChatId)`'s `class` (`Free`, so it may sit anywhere), `home` (the right region),
`home_in` — the mode is part of the placement policy, though a chat answers it the same way in
every mode: it holds the right dock whatever mode is on screen, Teams included, where the graph and
its own inspector take the centre and stay inline rather than in the dock — `closable` and
`is_drawn` rules. `PanelKind::chat_home(mode)` is `home_in` asked without a tab in hand, for the
window filling a side region it has not minted a tab for yet. `crates/ubiq/src/ui/dock/mod.rs`'s `chat_payload` and
`chat_from_payload` are that round trip; a saved leaf naming an id this window did not already hold
is dropped on restore, the way a saved terminal leaf naming a gone pane is — a chat id is not the
host's to confirm, so an unfamiliar one is trusted no further than an unfamiliar pane id is.

`crates/ubiq/src/state/chat.rs` holds `ChatTab`, `free_chat_slot` — the lowest slot in the chat
range nothing is using — and `attach_choices`, the pure function behind every attach list in the
window: which conversations survive the typed filter, which of them an open panel of the *asking
surface* already shows and so draws disabled, and which index (if any) is the asking panel's own
current pick. One builder, so "already taken" is answered once for the chat header's control, the
`+` menu's second stage and the agents screen alike.

`crates/ubiq/src/state/agents.rs` defines `COLUMNS_MAX`, `CHATS_MAX` and
`COMPOSER_SLOTS = COLUMNS_MAX + CHATS_MAX + 2` — the two above the chat range are `SINK_SLOT`, the
kitchen sink's bench, and `TEAMS_SLOT`, the Teams screen's inspector; `AgentsView::free_slot` still
allocates a column's slot from the low range, unchanged. `TEAMS_SLOT` is the one composer that does
not type into the active project: it addresses whichever card the canvas has selected, which under
the window span may be any project the window holds, and `AppState::send_or_enqueue` resolves that
project through `project_of_agent` rather than reading the window's.

`crates/ubiq/src/app/chat.rs` is where a tab's own lifecycle lives: `open_chat_tab` mints one and
gives it a slot, `open_chat_tab_now` puts it in the dock as well — called when there is a
conversation to put in it, never before — through `PanelEdit::Reveal` rather than `Open`, since
either caller (the titlebar's own shortcut, or a conversation the `+` menu just started) may fire
with the right region put away, and creating a chat has to bring the region back regardless of what
was on screen when it was asked for. `attach_chat` sets or clears an attachment, `chat_picks` builds what the control offers, `pick_chat_row` resolves a click against
that same list, `toggle_chat_picker` and `dismiss_chat_picker` own the control's open flag, and
`closed_chat_tab` is what a tab leaving the dock for good runs — dropping the `ChatTab`, clearing its
slot's draft, and touching nothing about the conversation it was looking at.

`settle_persistent_chat`, also in `app/chat.rs`, attaches a project's seed chat tab to its persistent
agent the moment the work naming that agent is in hand — and **does not bring the right panel on
screen**. A project opens with its right region where `ModeLayout::default_for` or the user's own
saved arrangement left it (workbench.md), so the tab is queued as `PanelEdit::Open`, which joins the
group in the right region without reopening it; `PanelEdit::Reveal` is the edit that would, and it is
what the user's own gestures use. `OpenProject::persistent_settled` guards it to once per project, so a later `WorkList` cannot
reopen a tab the user has since closed on purpose; it is called from `enter_project`, which may run
before the work has arrived, and from the `WorkList` answer, which is when it has.

`Message::ConversationDeleted` — the answer to `EndConversation` — is handled in `app/wire.rs`: the
conversation and its `WorkAgent` are dropped from the project, it is pruned out of any column, and
every chat tab attached to it is **closed** — `AppState::close_chat_tab_in`, which takes the
project id rather than reading the window's, because the delete can arrive while the window is
looking at somewhere else. Closing goes through that method rather than dropping the `ChatTab` row,
so the dock leaf leaves the tree and the composer slot is cleared before it is handed on. A detached
tab would be an empty panel left where a conversation used to be, which is what a fresh `+`
produces *on request* and not what a delete should leave behind (`D100`).

**Rows and the actions behind them are matched by position**, the rule every menu in this window
follows, so `state::chat::chat_picks` builds one list of `ChatPick`s — `New`, `Attach` or `Inert` —
that the frame draws and the click resolves against. A `New` raises the start form and writes down
that this tab is where the conversation goes; the tab is attached when
`Message::ConversationStarted` lands, so the id is never claimed by a form that was dismissed. An
`Attach` is immediate: the conversation already exists — and `pick_chat_row` follows it with
`restore_composer_draft`, putting back whatever that agent's own `Conversation::draft` is still
holding.

`crates/ubiq/src/app/panels.rs`'s `sync_chat_panels` is a chat tab's real population — squaring the
dock's tree with `OpenProject::chats` — called whenever a project is entered, right after
`OpenProject::new` has seeded that project's first tab, and again at the end of `settle_layout`, so a
restore that dropped an unfamiliar id is squared with the truth immediately. `settle_panels` skips the
`Open` edit `sync_chat_panels` queues for that seeded tab through `AppState::is_idle_chat` — a chat
panel attached to nothing, bound for the side region the chat calls home in the mode on screen while
that region is anything but **open and empty** — so IDE mode does not start with an empty agent
panel open, or the region it would sit in. Open and empty is the one case that takes one, because it
is `toggle_region`'s own gesture: the user asking to see something on that side, answered with a
fresh tab where the mode has no furniture of its own for it. A region that already holds something
is not asking, and letting the seeded tab in wherever that region happened to be open is what put a
chat beside Git's changes panel and then wrote it into Git's blob (`D156`).

**A mode switch never places a chat tab either.** `settle_layout`'s leftover loop keeps a chat panel
the incoming mode's blob does not name and drops only its *placement*: the panel is the one thing a
rebuild cannot make again — a `ChatId` is minted fresh every process, so a saved leaf naming one this
window no longer holds names nothing — while the tab comes back on screen from the blob of the mode
it was opened in, or from the user's own reveal.

Rendering is two modules under `crates/ubiq/src/ui/chat/`: `mod.rs` resolves a tab's own attachment
once — `attached`, read by both children below rather than asked twice — and hands it to the shared
conversation renderer (`header: false`, since the row is drawn separately below), or draws the empty
page's play button; `sidebar.rs` calls `ui::conversation::lifecycle_header` itself for that row —
the three-dots menu and the change-agent chevron on the left, the current-action chip flush against
the right edge — or, unattached, draws the chevron alone at the same row height. The permission
prompt is that shared renderer's
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
holds the rest of the transcript's own furniture: `plan_rows()` is the row walk, lifted out of
`transcript()` so both folds can be tested without an element — one `RowKind` per row, `Group` for a
run of same-kind calls and `Thinking` for a run of reasoning — and `transcript()` draws what it
planned; `tool_group()` is the `N earlier calls` row, `thought_group()` the bordered `THINKING` box
over a whole run, `one_block()` the arm reused for a card that is not folded and for the ones a fold
opens; `writing_mark()` is the
tail's running mark, and `tail_signature()` is what the follow-the-tail scroll compares, read over
the visible blocks it is handed and folding in `conversation.pending.len()` beside the block count
and the run, since a pending prompt moves nothing else it reads. `build_row()` carries the gutter and
`theme::font(Family::Conversation, Role::Body)` on the row itself, because the list lays items out
in `prepaint` where a parent's style is off the stack; `row_signature()` is where the width and
that size enter a row's signature. `to_tail_button()` is
the overlay, on `AppState::scroll_transcript_to_tail`. `state::conversation::TranscriptScroll` is
the rest: `sync()` is the once-a-frame decision — save the outgoing transcript's position, restore
this one's, follow the tail only for a reader on it — with `away()` for the overlay, `request()`
and `take_request()` for the block the strip asked to be taken to, and `to_tail()`. The follow rule
is two free functions beside it, `off_bottom()` for whether there is anything below the viewport and
`away_reading()` for the sticky answer over that plus the frame's own rise, with `TAIL_SLACK` as the
slack both allow and `own_move` marking the frame after `to_bottom()` pinned the offset itself.
`AppState`
holds one per composer slot as `transcript_scrolls`, indexed exactly as `column_inputs` is; every
field of it is interior-mutable, because `render` holds `&AppState` and these are readings of the
last frame rather than state the application owns. The tool fold's
open set is `Conversation::open_groups` with `toggle_group()` beside it in
`crates/ubiq/src/state/conversation.rs`, reached from the row through
`AppState::toggle_conversation_tool_group` in `crates/ubiq/src/app/agents.rs`; the reasoning fold
has no open set of its own — the run is read off its last block, moved by
`Conversation::toggle_thought_group` over the run's indices and remembered in `touched_thoughts`,
reached through `AppState::toggle_conversation_thought_group`. `footer()` takes the account's snapshot from
`SettingsState::quota` in `crates/ubiq/src/state/settings.rs`, falls back to
`snapshot_from_rate_limit` over `Conversation::rate_limit` where the host has said nothing yet,
words the tooltip with `quota_tip`, and colours each band with `theme::usage_tone` — the one
accessor the settings meters share, so the two surfaces cannot disagree about where the thresholds
fall. `QuotaSnapshot::windows()` supplies up to two gauges in provider order for `progress_ring_pair`
in `crates/ubiq/src/ui/kit/controls.rs`, falling back to `progress_ring_in` where only one is stated.
It also reads
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
`detach_file` is a tag's `×`. Those two are the exception to the rule above — they take the window's
active project, where the enqueue write, `clear_attachments` and a queue row's edit, remove and
requeue take `project_of_agent` — so a card belonging to another project cannot have a file attached
to it or taken off it; `G326` is the backlog row that holds that, along with the question of whose
explorer tree a foreign card's picker should offer. `crates/ubiq/src/ui/conversation/mod.rs`'s `attachment_tags()` draws
the wrapping row as the composer's first `extras` entry, on `kit::removable_tag`;
`sent_attachment_tags()` draws the same list under a sent turn on `kit::tag`, and
`attachment_preview()` is the `kit::popover` a chip there opens. `paste_into_composer` is the
paste, reading the board through `app/clipboard.rs`'s `clipboard_attachment` and the project roots
through `AppState::project_relative`, which a drop onto the window reads too. The control
itself is `crates/ubiq/src/ui/kit/menu.rs`'s `Picker`, unchanged — its `disabled` set draws the
headings and the rows another tab holds, its `separators` set the group lines, and its `search`
field the filter. A grouped, searchable, partly-inert list was already what that primitive did.

## Failure

| What happens | Result |
|---|---|
| A chat tab has nothing attached | Its page is the play button that starts one, rather than an empty transcript, and its dock tab reads `New chat` |
| No provider is configured, or the naming fails | The tab keeps the mechanical name it started with and hovers to nothing. Nothing is reported: no name was taken away and no question was asked |
| A naming answers a title and nothing after it | The tab is renamed and draws no hover. A summary is a tooltip, and a tooltip is allowed to be absent |
| A naming answers Markdown, an emoji or a label it was asked not to add | Both are taken off before the title is written down. The wording asks for simple plain text in so many words; `ubiq_proto::assist::plain_text` is the net under it, and a line that was nothing but decoration is dropped rather than becoming an empty name |
| The chat range's composer slots are all taken | The strip's `+` and a re-opened empty right region do nothing; there is no ninth slot to hand out, the same ceiling a ninth column meets |
| A harness is uninstalled between the draw and the click | The start form refuses rather than sending a `StartConversation` that would fail as a spawn, and the tab stays attached to what it had |
| The remembered harness is gone, or the remembered profile deleted | The form answers nothing rather than opening on a start that would fail |
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
| A row lays out taller than it last measured while the transcript sits on the tail | The follow holds: content growing raises the list's maximum, and only the offset rising says the reader moved |
| The conversation a chat tab is attached to is deleted, in this project or another | Every tab on it closes, dock leaf and composer slot with it — the one host event that ends a view |
| A permission request offers no option of the reading ⌘⌥Y or ⌘⌥N asks for | The keyboard does nothing; the buttons the harness did offer are still there to press |
| A conversation reports a total but no cached figure, or a total of zero | The cache ring is not drawn; a ring at nothing over nothing is not a reading |
| The conversation runs as no account, or its provider names no limit | No quota ring; there is no window to draw, and a zero ring would claim one that is empty |
| The host has cached nothing and the harness has pushed nothing | No quota ring. The reading arrives when a turn runs or the accounts page asks, and until then nothing is stated |
| The cached snapshot is old | The ring still draws — a stale reading is a real one — and the tooltip says how old it is rather than implying it is current |
| The turn is cancelled while asks are up | The outstanding set is dropped, the prompts and the strip go with it, and the host answers every one of them as cancelled before the cancel reaches the harness |
| The harness ends or is unloaded while an ask is up | The prompts go with the process; there is nothing left waiting on an answer |
| The conversation accepts everything and the harness offers no allowing option | The host emits the request unchanged, and it is drawn and answered like any other |
| A conversation surface is asked about an agent in a project the window is not pointed at | It is answered from the project that owns the agent: `project_of_agent` finds it, and the transcript, the record and every write follow the card rather than the rail. Attaching a file is the one thing that does not, and `G326` holds it |
| Accept-all is switched on while an ask is up | The prompt on screen stays and is answered by hand; the flag governs the asks that follow, and nothing retracts a prompt the transcript holds |
| An `AskUser` arrives while its conversation is off screen, or while any dialog is already up | A notification is raised instead of the modal, and the transcript's "Ask for feedback" entry is the way back to it; the drafts wait there until it is opened |
| An ask's dialog is opened after it was answered, chatted away, timed out or its conversation ended | The same dialog, with no controls — the questions and what was chosen, read-only |
| A conversation an ask belongs to ends, is unloaded, or its harness dies while the ask is still waiting | `AskEnded` closes it as `Gone`; the entry and a reopened dialog say so instead of offering a control that would send into nothing |

## Related docs

- [`workbench.md`](./workbench.md) — the dock a chat tab is one panel in, and the composer slot pool it shares with a column
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the conversation family an attached tab speaks
- [`../tech/decisions.md`](../tech/decisions.md) — `D61`, why a tab is exclusive per surface and not per conversation, and `D100`, the one event that closes a view
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens the transcript and its blocks are coloured from
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — who owns harness knowledge, for the rows the start half offers

## Next steps

- Persist a chat tab's *arrangement* across a restart, not only its attachment.
- Let a tool block open the file it names in the editor.
- Attachments that carry the file's *content* over the bus as a `ResourceLink` — which
  `agent-manager` already accepts — rather than an `@path` mention the harness has to resolve
  itself. The tags, their sizes and their lifecycle are built;
  [`../backlog.md`](../backlog.md) holds the wire half.
