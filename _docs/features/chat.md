---
id: feat-chat
title: The chat panel
kind: feature
status: draft
summary: Editor-like chat tabs — many, movable to any dockable region, each a view onto a host-owned conversation or onto none, drawn by the composer, transcript and tool blocks the whole window shares.
read_when: you are changing a chat tab, the control that starts or attaches a conversation, or which conversation a tab shows
updated: 2026-10-06
verified: 2026-10-09
code_anchors: [crates/ubiq/src/ui/chat/mod.rs, crates/ubiq/src/ui/chat/sidebar.rs, crates/ubiq/src/state/chat.rs, crates/ubiq/src/state/dock.rs, crates/ubiq/src/app/chat.rs, crates/ubiq/src/app/clipboard.rs, crates/ubiq/src/app/picker.rs, crates/ubiq/src/app/panels.rs, crates/ubiq/src/app/wire.rs, crates/ubiq/src/app/shell.rs, crates/ubiq/src/app/boot.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/ui/conversation/info.rs, crates/ubiq/src/ui/acp_capabilities.rs, crates/ubiq/src/state/conversation.rs, crates/ubiq/src/state/work.rs, crates/ubiq/src/app/agents.rs, crates/ubiq/src/ui/agents/mod.rs, crates/ubiq/src/ui/dock/skin.rs, crates/ubiq/src/state/prefs.rs, crates/ubiq/src/app/projects.rs, crates/ubiq/src/state/ask.rs, crates/ubiq/src/app/ask.rs, crates/ubiq/src/ui/ask.rs, crates/ubiq-proto/src/ask.rs]
depends_on: [feat-workbench]
review_cycle: monthly
---

# The chat panel

## Purpose

A harness in a terminal shows what an agent is doing; a chat tab shows what it was asked and what it
concluded, beside the code. It is furniture in four modes — IDE, Tasks and both Teams screens — and hidden
in the rest. Unlike every other IDE-mode panel it comes in many instances: a chat tab is a perspective on a
conversation the host owns, so many may be open, each attached to a different run or to none, and closing
one ends nothing. The relation holds in one direction only: deleting the conversation closes every tab
looking at it.

## Behaviour

### The tab and what it attaches to

**A chat tab is `PanelKind::Chat(ChatId)`, one panel per instance.** The id is minted locally — it is UI
arrangement the host never hears about. Tabs coexist, dragged apart, tabbed together or moved to any
dockable region (left, right, bottom, centre). The default home is the right edge, at `CHAT_WIDTH`.

**A tab is attached to a conversation, or to nothing.** The attachment is `state::chat::ChatTab`, one per
tab in `OpenProject::chats`: the tab's id, its composer slot, whether its control is down and the `AgentId`
it looks at. Closing a tab drops the `ChatTab` and frees the slot; the conversation keeps running.

**One control chooses what a tab looks at; its first row starts something new.** It is a bare chevron in the
tab's header, tooltip *change agent* — the conversation's name is already on the dock's tab and the hexagon,
so a label would be a third copy.

- **First row *New agent*** raises the window's start form (harness, identity, model, reasoning level and
  permission mode asked together; *The workbench* describes it). The titlebar's shortcut to the same start,
  `AppState::open_new_agent_direct`, aims at the chat surface in IDE mode and in Tasks (`T-109`: the board's
  `+ New agent` lands in the right dock beside the task), skipping the `+` menu's first stage. An attached
  tab is offered the start too; the conversation it leaves is one click away on the list.
- **Every other row is a conversation to move to**: the conversations this window holds
  (`AgentsView::live`), under a hairline drawn only when something is below it. Typing filters; the hairline
  goes when a query empties the list. The host's projection can be wider than what this window holds, so
  `state::chat::attach_choices` narrows to the live set before
  the query does — the chevron and the `+` menu's second stage answer the same way.
- A conversation attached to a *different* chat tab draws disabled and is never dropped from the list (a
  vanished row reads as ended). The tab's own attachment stays selectable.

**Exclusivity is per chat tab, not per conversation.** The agents workbench may show the same conversation
in a column, and Teams may point the project's chat tab at it (`AppState::open_teams_agent_panel`) while
another chat tab is attached to it: three viewers, and the host is never told which surfaces look (`D61`).

**Adding a view is the tab strip's gesture.** A `+` on the dock's tab strip — on any group holding a chat
*or sitting at the chat home region with none yet* (`AppState::is_chat_region`,
`ui::dock::skin::NewChat::region`; that clause puts it on Tasks' right region) — opens the two-row menu *New
agent* / *Attach existing agent*. A chat dragged into the editor region takes the control with it
(`hosts_chats`). **The tab is minted when there is something to put in it**: a pick attaches at once, a
start form opens the tab from `Message::ConversationStarted`, so a dismissed form leaves no empty tab.

**Closing the last chat tab is allowed.** The emptied region puts itself away; opening it again (titlebar
switch, or the `+` past the last group's strip) mints a fresh unattached tab. A *pinned* tab withholds the ×
and Hide on the shared tab menu; Rename, Close and Pin/Unpin still work (`AppState::tab_names`,
`AppState::pinned_tabs`, in memory only, as a `ChatId` is reminted every run). **A persistent attachment
survives a restart**: `ViewPrefs.chats` remembers the agent of each *persistently* attached tab. A tab whose
conversation the host was not asked to keep is written as nothing, because the boot sweep deletes the run
directory (`D97`).

**Deleting the conversation closes its tabs** (`D100`, see Implementation).

### Header, name and empty page

**The header is the one first line every conversation-hosting surface draws**
(`ui::conversation::lifecycle_header`): three-dots menu at the left, this tab's change-agent chevron beside
it (`[menu, switch?] ... chip`), the current-action chip flush right. Nothing attached draws the chevron
alone. The header does not name the tab, and draws no state mark: a chat tab's state is the hexagon the
dock's tab wears (below). The three readers behind the menu — `is_persistent`, `accepts_all`, `dump_path` —
take the host's record through `teams_agent`; a row that read `None` would report *off* for a flag that is
on, and its toggle would send *enable* every time.

**The dock tab's name is the agent's title**, through the one resolver `AppState::agent_label` (T-283): the
user's rename, the generated or harness title (`WorkAgent::title`), else the agent definition
(`WorkAgent::definition`), else the handle (`claude 2`). It is cut to fifteen characters, as a terminal's
tab is. The hover is the standard agent tooltip, one fact a line: summary, identity (`definition · harness ·
model`), the assigned task's title and the handle, led by the full title when cut. After the first answered
prompt the host names the conversation onto its record and the tab follows with the next `AgentChanged`. An
unattached tab reads `New chat` and hovers to nothing. The naming rule, its other surfaces and the switch
that disables it are the workbench's; the record is the work family's `AgentChanged`
([`../tech/transport-contract.md`](../tech/transport-contract.md)); `D90` is why a name Ubiq invented may be
replaced by one it read.

**An unattached tab is one large play button**, centred, on `start a new agent`, drawn at `EMPTY_START_SIZE`
rather than through `kit::icon_button` (a fixed small chrome square). No title, no note: an empty tab is the
ordinary state of a fresh one.

**Every chat tab draws from the shared conversation view**, `crates/ubiq/src/ui/conversation`: the
transcript, tool blocks, footer and composer every live-agent surface shares (run pill, context ring, quota
ring, token cost, launch-time model and thinking pickers). **A link in a rendered reply goes where it
points**: the transcript hands `ui::on_link` to its `TextView`; a relative path (resolved from the project
root) or a `ubiq://` link opens in the window, `http`, `https` and `mailto` reach the OS, anything else does
nothing ([`workbench.md`](./workbench.md)). A path merely mentioned in prose is text.

**A surface resolves the agent's own project, never the window's active one.** `AppState::project_of_agent`,
with the readers `teams_conversation` and `teams_agent`, is what a chat tab, an agents column and the Teams
inspector all go through, for every read and write (send, enqueue, recall, permission answers, folds,
attachment edits). Under the window span the Teams canvas draws every project, so an inspector card may
belong to a project the rail is not pointed at (`D154`); a lookup through the active project answers `None`,
indistinguishable from an empty conversation, and a turn is dropped, a flag reads *off* while on, or a
pending list goes stale, silently.

**Each tab owns a composer from a fixed pool.** The window builds `COMPOSER_SLOTS` text areas
(`0..COLUMNS_MAX` for columns, the range above for chat tabs) before the first frame, because the
subscription mirroring typed text lives for the window's life. Text typed in one tab never appears in
another; closing a tab clears its slot's draft before the slot is reused.

**The panel is hosted by dock tabs and by Agents-mode columns.** `ChatHost` keys it by either a chat tab
or a column's composer slot. In a column it omits the attach chevron (the column's tabs choose the agent)
and the left hairline, and a column whose agent has no conversation shows the panel's own `No conversation`
page. The column keeps its tab strip, `+` menu and identity line outside the panel
([Agents mode](./workbench-agents.md)).

**Focus mode shows one chat near full-window.** An attached tab's or a column's header carries a Maximize
button beside the change-agent chevron (a column has no chevron), and `⌘⇧⏎` / `⌃⇧⏎` does the same for the
chat whose composer holds the keyboard, else the only candidate; the candidates are the attached chat tabs
plus, while Agents mode shows, its columns. The conversation is drawn as a modal at `CHAT_FOCUS_RATIO` (94%)
of the window each way, titled with the agent's title, on the docked panel's ground (`app_bg`, not a raised surface); the panel behind it reads `Focused`. The modal
draws the same conversation over the same composer slot, so the draft, the attachments and the
transcript's scroll carry over in both directions. One chat is focused at a time —
`WorkbenchState::chat_focus`; focusing another replaces it. Escape, the button (now Minimize), the
shortcut and a click outside the modal each put the chat back, and so does closing the tab or benching or
closing the column the focus follows. Its
`Layer::ChatFocus` rung sits between the task-import dialog and the feedback modal.

### The transcript

**The transcript is a virtual list.** A frame builds a *plan* — one `Row` per thing on screen, arithmetic
and no elements — and `gpui_component::v_virtual_list` builds only the visible rows, so a frame costs what
is in the viewport.

**Heights are remembered.** `TranscriptScroll` keys measured heights by row *identity* (a fold opening moves
every row below it) against the content, the width and the conversation family's body size. Unchanged is its
measurement; changed is its *last*; never drawn is an estimate (a line per eighty characters). The last two
are re-measured by the frame that draws them. The plan also records which block each row stands for, so the
strip above can resolve a block to a row.

**A forced re-measure runs on a timer** underneath `needs_measure`'s signature check, because a long busy
conversation's rows drift in a way no signature explains (a resize always fixes it).
`TranscriptScroll::force_relayout` asks the next frame to re-measure unconditionally; `AppState::new`
(`app/boot.rs`) runs one per composer slot every four seconds for the window's life. A row found at a
different height under an unchanged signature is logged as a warning on `ubiq::ui::conversation` (kind,
length, cached height, measured height). A mitigation, not a fix: `backlog.md`'s `G372`.

**Where a reader was left is remembered per transcript**, keyed by the conversation *and* the delegate
viewed (`state::conversation::TranscriptScroll`): a slot moved to a delegate and back restores both
positions. A transcript never shown opens on its tail.

**The tail is followed only for a reader who is on it.** `tail_signature` reads the blocks *on screen*
(`visible_blocks()`), so another subagent's blocks do not scroll a transcript nothing was added to. It folds
in the run and `conversation.pending.len()`: a permission ask is not a block, so without it an ask arriving
under an unchanged last block would land below the fold. **Being on the tail is sticky; only the reader
leaves it.** Content cannot make the offset *rise* between frames, so a rise by more than `TAIL_SLACK`,
while something is below the viewport, means the reader left; the follow resumes within the slack. Growth
alone never ends it (a taller-than- measured row raises the maximum while the pinned offset stays), and the
frame after the pin is discounted (`own_move`), since the paint clamp reads back as a large move nobody
made.

**`Go to last message`** is an overlay at the transcript's lower right (nothing moves when it appears),
drawn only while something is below the viewport. It scrolls and marks nothing read; the next message
resumes the follow.

**A running turn is drawn at the tail** by `ui::conversation::writing_mark`: while the run is `Working` and
no ask is waiting, three dots pulsing a third of a cycle apart, in `Activity`'s colour with its word beside
them — movement is the only honest thing to draw in a minute of silence, since a spinner claims progress
nothing measures. It is not drawn while an ask is up. Its appearance scrolls the tail into view like a new
block.

**Two things fold, with different rules.** Both judge runs over the blocks *on screen*, so another
subagent's blocks do not break a run; a sentence or tool call between two thoughts does.

- **Same-kind tool calls**: three or more consecutive (`GROUP_MIN`) draw as the last card plus one
  `tool_group` row reading `N earlier calls` in that kind's colour, which opens in place on click (two cards
  would be no shorter). A `Delegate` call is never folded (a second transcript, not a step), nor is a call
  with a permission ask attached (a prompt behind a counter deadlocks). The open set is
  `Conversation::open_groups`, keyed by the run's first call id, UI-only, toggled by
  `AppState::toggle_conversation_tool_group`.
- **Reasoning**: every consecutive `ConvBlock::Thought` is a child of one bordered `THINKING` box, no floor
  (where a harness flushed is not the reader's concern). It is expanded while the thinking is the thing
  still being written (the last block's `open`); `Conversation::end_open_thought` collapses it when a
  different block starts, a turn ends or the harness compacts. Clicking the caption moves the whole run and
  marks every block in `Conversation::touched_thoughts`, so the reader's choice is never overridden. Toggled
  by `AppState::toggle_conversation_thought_group`.

### Permission asks

**A permission ask is drawn on the tool call it authorises, not in a dialog.** The prompt joins the block by
tool call id, the block reads as awaiting approval, and the buttons sit under it: one per
`PermissionOption`, labelled from the option's `name` and told apart by `kind` (allow/reject, an "always"
variant marked as lasting). `kind` decides only how a button reads; the `option_id` is opaque and echoed
back, and nothing here remembers a choice.

**The request's detail scrolls in its own capped region** (T-175): a `switch_mode` plan or a pre-approval
diff is unbounded, so `permission()`'s `detail` (drawn from the request's own `content`) is
`max_h(theme::permission_detail_max_h())` (base `PERMISSION_DETAIL_MAX_H`) plus `overflow_y_scroll()` on an
`id` keyed by the request id. Title and buttons stay put.

**A conversation set to accept everything is shown no ask.** The three-dots menu's *Accept all*
(`SetConversationAcceptAll`, on the agents screen's menu) is answered in the host with the plain allow, so
no `ConvUpdate::PermissionRequest` reaches the window — never shown rather than shown and withdrawn, since
nothing takes a prompt back. The transcript then holds tool calls the reader was never offered, which is why
the flag is per conversation, off by default and labelled while on. A request with no allowing option
arrives and is drawn as normal.

**The request carries an id and little else.** The `tool_call` is a patch whose id is the only guaranteed
field, so title, content and diff are read off the call the transcript holds. A request naming an unseen
call degrades to a self-contained prompt at the transcript's end.

**Several asks may be up at once and each blocks.** They are an ordered list keyed by `request_id` (there
are no timeouts; an unanswered request stalls the turn silently). Cancelling the turn discharges all of
them.

**A "needs you" strip above the footer carries what is outstanding** (a prompt partway up can scroll out of
view). It is answerable: Yes, All and No for the oldest request, each drawn only where that request offered
such an option (`Pending::option_for`, `Pending::always_option`) — a third button standing in for a lasting
allow would lie about lasting. Clicking the label goes to the question: `AppState::reveal_permission`
switches to whoever raised it and scrolls the call into view, both from `Conversation::pending_route`; a
request naming an unheld call routes to the tail. Above one request, a count badge sits beside the `NEEDS
YOU` mark, and a delegate's request carries the delegate's name. The rest are answered one strip at a time.

**⌘⌥Y allows and ⌘⌥N rejects the oldest ask** of the conversation being read (the agents screen's focused
column's active tab), with the first allow- or reject-kind option offered, and do nothing where none is.
Bound in both the `Workbench` and `Input` contexts, so a focused composer does not swallow them.

### Structured questions (`ubiq-ask`)

**An `ubiq-ask` question is not a permission ask, and is drawn as a dialog.** `AskUser` arrives from a tool
call the host parked and is filed on the conversation as an `AskRecord` (`crates/ubiq/src/state/ask.rs`), a
side channel joined by id. With the conversation on screen and no other dialog up, the modal opens directly;
otherwise a notification is raised and the transcript carries an "Ask for feedback" row
(`ui::ask::transcript_entry`) whose button reopens the dialog with its draft intact — closing never touches
a draft. The modal is a tab strip, one tab per question, each a column of option cards (single or multi by
the question's flag) plus "Other" and a notes field. Confirm sends `AnswerAsk` with picks by label; "Chat
about this" sends it with nothing answered and lets the user type. Either leaves `AskStage::Waiting`; after
that, and after an `AskEnded` for a timeout or gone conversation, the dialog reopens read-only (questions
and what was chosen). No marker beside an option's label: the card's fill and edge are the pick
(`kit::card`'s `selected`). The strip stays put while only the question scrolls.

**A question arrives one way** (`D210`): the agent's one ask tool, `ask_user_question`, waits for nothing —
the dialog is raised when the agent's **turn ends**, and answering opens the next turn with the answer's prose as the prompt (the
transcript shows the dialog and answer, not the prose). Typing into the composer instead closes the dialog —
one or the other, once. A turn that fails after asking raises nothing. The transport contract states
the mechanism; `D175` and `D210` are the choice.

**Dialogs registered in one turn are answered as a set** (`D192`). All go up together at turn end, each
answered on its own in any order, and **nothing reaches the agent until all are answered**; one prompt then
carries the answers in question order. A note above the tab strip reads "Question 2 of 3 the agent
registered this turn. Nothing is sent until all 3 are answered", the footer says Confirm is holding, and a
confirmed dialog says it waits on the rest. Giving one up ("Chat about this", or a dialog the window cannot
draw) costs that question only.

**A parked question stays answerable after its tool call gives up.** The call waits 45 seconds and returns
without an answer (the harness's own tool timeout is shorter); the dialog stays and a later answer is
submitted as the next turn, as for a registered dialog (`D191`).

**One ask is drawn in one transcript: the asking agent's.** A conversation and its subagents share one
`AgentId` and `ubiq-ask` endpoint, so `AskRecord::subagent` (from `Conversation::asking_subagent`) says who
was speaking, and the row is drawn in that transcript alone.

**The row reads `warning` while it blocks the harness, `accent` once it does not.** It uses
`theme::warning`/`warning_soft`, like `ui::conversation::permission`'s "NEEDS YOU", and falls back to accent
on leaving `AskStage::Waiting` (confirmed, chatted away, timed out, gone). It is placed where the ask
arrived: `AskRecord::at_block` is the block count when the call parked, and `ui::conversation::plan_rows`
inserts the row before the first block-anchored row at or past it, so later turns land beneath it.

**Keyboard reaches the whole dialog.** Up/down walk a cursor over the question and space picks or unpicks;
plain `enter` does the same and moves to the next question; `⌘⏎` moves on by itself, confirming from the
last. None fires while "Other" or "Notes" holds the keyboard, where `tab` moves between them (`G327`,
closed).

### The activity bar

**Delegates and todos are two chips on one activity bar**: a row above the footer and below anything queued,
subagent chip left, todo chip right, a flexible spacer so a lone chip reaches its own edge. A chip appears
only when its source is non-empty; the bar only when a chip does. Exactly one panel is open at a time;
opening one closes the other.

**The subagent chip** is a chevron (up closed, down open) and `subagent_count_label`: `3 active subagents of
10` while seven finished, `3 active subagents` while all run, `10 subagents` once none is active. A
conversation with no subagent draws no chip. The panel opens through the shared `popover` pair, anchored to
the chip, drawn upward. One row per agent: the main agent first (always present, the way back), then each
delegate from `subagents`, saying its name, its model (`short_model_label`, where stated) and its status.

- **A blocked delegate** reads `need you` in the warning tokens in place of its status (`need you ×N` above
  one request) — `running` plus a question would be one of them wrong. `Conversation::pending_count`
  resolves onto `SubagentTab::waiting`; the main agent's row is read the same way.
- **Every other row** shows `state::status::delegate_status` (main agent: `conversation_status`) as
  `Status::label()`: `Working · Tools`, `Ended · Done`. A delegate whose spawning call is not in the
  transcript reads `Starting`; one whose call reached `Completed` reads `Ended · Done` and leaves the
  `active` count.
- Clicking a row switches the transcript to that agent and closes the panel; the main agent's row only
  closes it. The `AGENT` block that spawned an agent is the same door, inert until that agent has said
  something.
- **A delegate says what it answers with**: the reading strip above its transcript and its row both carry
  the model; its hover names kind, model and thinking level where stated, via `short_model_label`. All read
  one `SubagentTab` field resolved on `Conversation`. Nothing is borrowed from the parent; `thinking` is
  `None` on every harness, as no stream states per-delegate effort.

**The todo chip** reads `{done}/{total} todos` with a chevron and is hidden for an empty plan. Its panel
lists up to eight entries (`✓` completed, `▶` in progress, `○` pending), then `… N more`. Clicking a row
closes the panel and does not switch the transcript.

### Attachments

**Files are attached to the turn being written, as tags.** The composer's `+` raises the window's file
picker over the project's explorer tree (multi-select); each file is a tag in a wrapping row under the token
and context readout, over the field. The queue is the topmost thing in the bottom block, above the activity
bar, footer and composer. Clicking a tag opens the file in the editor, its `×` removes it. A tag shows the
name; its tooltip shows the project-relative path and the size.

**Typing `@` at the start of the field or after whitespace raises the same picker** (`app/picker.rs`
`at_trigger`, `open_picker_from_at`, called from the composer's `Change` subscription in `app/boot.rs`). The
`@` is consumed, since the picker has no query and a pick comes back as a tag that becomes `@path` at send
time; `foo@bar` does not trigger. With no project tree the `@` stays. The field grows with its text up to
`COMPOSER_ROWS_MAX_DEFAULT` (16) rows, then scrolls; the bottom block is bottom-anchored, so it grows upward.

**A tag's colour is the file's size.** Over 300 KiB the warning tokens, over 500 KiB the danger ones, with
the reason in the tooltip: a large file costs context, and the tag says so *before* the turn is spent. A
file no host sized is drawn plainly (unknown is not small, and is not guessed).

**An attachment belongs to the conversation, not the slot.** It sits beside the draft and queue on
`Conversation`, so unsent content follows the conversation between surfaces. Nothing new crosses the bus: on
send, every path is composed into the one `PromptAgent` text as an `@path` mention after what was typed, and
the attachments are consumed with the draft (so attachments with nothing typed are still sendable). Enqueue
does the same composition into the queued text and clears them — a queue row with its own tag list would be
a second composer. An edit brings the paths back as text.

**The typed text follows the same rule through `Conversation::draft`.** Every keystroke mirrors into the
addressed agent's `draft` (`AppState::remember_conversation_draft`), because the slot's own copy is dropped
when the slot is freed. Reattaching (the chat header's *Attach running*, a bench pick into a column)
restores it (`AppState::restore_composer_draft`), only into an empty field. A slot that keeps showing one
agent through a mode switch or a hidden region touches neither copy; the pooled `Entity<TextareaState>` is
untouched by dock placement.

**Pasting into the composer attaches when the board carries a file.** `⌘V`/`Ctrl+V` with the field focused
reads the pasteboard first: a copied *file* becomes a tag under its path — project-relative inside a project
this window holds, absolute outside every one — and a copied *picture* becomes a tag too. Only the first
path of a multi-file copy is taken. A text-only board returns the keystroke to the field. The chord is bound
for the workbench and for `Workbench > Input`, as the library's own field binds it deepest. A pasted file is
made relative only to **the agent's own project**: another project may hold the same `src/lib.rs`, and the
relative form would name a different existing file the harness reads silently.

**A pasted picture is written into the project first**, under `.ubiq/pasted/` through `WriteProjectFile` —
an attachment is an `@path` mention, so it needs a path. This is a file in the user's project, deliberately
(`.ubiq/` is Ubiq's own folder there, beside `.ubiq/local/kb`). The name is the paste's millisecond, so a
later session never repoints an older tag. What results is an ordinary attachment: same tag, dedupe, size
warning and `@path` on send.

- **The folder ignores itself**: the first picture writes `.ubiq/pasted/.gitignore` holding `*`. A paste is
  the side effect of a keystroke, and untracked binaries nobody chose do not belong in `git status`. The
  user's own `.gitignore` is never touched.
- **Bytes are re-encoded as PNG where this build can.** Harness image support is png/jpeg/gif/webp, while a
  macOS screenshot arrives as TIFF and a Windows DIB as BMP; those two are decoded and written as PNG
  (lossless) and the name follows the bytes. A format with no decoder, or undecodable bytes, is written as
  it arrived under its own extension (`G249` is what that leaves unreadable).
- **An optimistic chip is taken back when its write fails.** The tag goes up on the send, so the interface
  tracks outstanding writes; a `ProjectFileError` detaches the tag and raises a notification (a pasted
  picture is in no editor tab, so the save-failed path never sees it).

**A sent turn keeps its chips; clicking one previews the file.** They are drawn inside the turn's accent
surface under the prose, without `×` (the harness has the file). The echo is one string of `@path` mentions,
so the conversation carries the list across the send, as it carries a start's preamble.

**The carried list is a queue, one entry per send.** Two Sends can be in flight before either echoes, each
keeping its files in order; a plain send takes an empty entry, or the queue falls out of step. An entry is
spent by the **real text** echo alone — a textless chunk and Claude Code's synthetic `[Request interrupted
by user]` do not spend one. A turn that can no longer echo (failed, stopped with an error, conversation
unloaded or ended) **discharges** the whole queue, so no later turn draws a chip for a file nobody attached
to it.

**Preview** opens an anchored panel with the picture — the editor's image viewer, over bytes read through
`ReadProjectFile` inside the project and directly for an absolute path — with name, size and path and an
`Open` into the editor. The answer is matched on project as well as path. In flight says so; a failed read
says *why*; a file over the read ceiling says too large (a prefix of an image is not an image); a file the
viewer declines says that. A panel, not a modal: the window's single open-menu slot holds it, so Escape and
an outside click peel it, and opening any menu clears it.

### The footer and the turn controls

**`ctx` is a level, `tot` is a flow.** The ring and `ctx` are how full the window is *now* (it falls on
compaction); `tot` is every token ever billed, subagents included, and only grows. Every readout says which
on hover: the ring, `ctx`, `tot` with its per-way and per-subagent breakdown, and the composer's identity,
model, thinking and mode chips. On the conversation's own transcript `tot` also names the uncached part, `X
tot · Y in` (`TokenSpend::input`, neither cache read nor write); a delegate's spend has no such split, so
its `tot` stays bare. Every raw count goes through `state::work::format_tokens` (plain under a thousand,
then `k`/`M`/`G` at one decimal).

**A second ring beside `tot` shows the cache-read share**: `cached_tokens` over `total_tokens`, in `info`
tokens (a second accent ring would read as the same fact twice), tooltip `cached X / Y Z%`. It is off unless
asked for (a cost reading, not a how-is-this-turn-going one); the setting is
[`workbench.md`](./workbench.md)'s. It is not drawn for a total of zero or no cached figure.

**The third ring is the account's plan, one band per rolling window the provider stated.** It is an account
fact (two agents with one identity read one window), drawn from the host's cache for the account — the default
identity (empty account, the user's own home) is a key of its own, asked for when its first conversation
arrives — and falling back to the harness's pushed reading. Two windows draw two concentric bands, the shorter outermost (it stops
the next turn first); one draws a single ring. Each band takes its colour from the usage thresholds, not the
accent, to make "nearly out" visible without a hover and because two accent rings read as one fact. The
figure, window name, reset, plan and the reading's age are in the tooltip for every window; the bare `5h N%`
readout belongs to the chrome (`D111`). No named limit, or a delegate's transcript draws no
ring.

**Each band carries a pace tick**: a thin radial line across the stroke at the share of the window that has
elapsed (`1 - time to reset / window length`) — where usage would sit if credit burned evenly to 100% at the
reset. On or under pace the tick is the neutral foreground and sits ahead of the arc's edge; over pace the arc
runs past it and the tick blends toward `danger` (`theme::pace_tick`), fully red from 25 points ahead. The
window length rides the gauge (`QuotaGauge::window_secs`, additive and optional: Claude `session` 5h, `weekly_*` 7d,
Codex `windowDurationMins`, the pushed windows 5h/7d); a window with no length or no reset draws no tick and its tooltip no pace line. The tooltip adds
per window: used, elapsed, the credit left to spend by the reset, ahead of or behind pace by N points, and —
only when over pace and the average burn so far would exhaust it before the reset — `At this rate the credits
run out in 17 min`. The maths is `state::settings::pace`; the ring and its tooltip are one component,
`kit::quota_ring`, shared with the harness settings.

**The footer reports whoever is being read.** With a delegate up, `tot` and the cache ring are that
delegate's (`Conversation::delegate_tokens`). **A delegate's spend is that delegate's**:
`UsageRecord::subagent_id` names the spawning `Task` call, so two `general-purpose` delegates are two
buckets, while `UsageRecord::subagent` stays the *type* the `tot` breakdown and usage meter aggregate by.
With no instance identified, nothing is drawn (`T-259`). **A delegate has no context level**: its usage
report repeats the *parent's* occupancy, so the ring is dropped rather than borrowed (`G195`,
[`../backlog.md`](../backlog.md)).

**Stop is there for the whole of a running turn, a filled square.** It sends `CancelTurn`: the turn ends,
the conversation and harness stay, and the next message goes to the same agent — a square, as a cross would
read *close this*. With something typed, Enqueue sits beside Stop (typing does not interrupt; the text goes
when the turn ends); an idle conversation has the one Send. All three are what Enter answers through
`AppState::send_or_enqueue`.

**A cancelled turn's own echo is not a message.** Claude Code's synthetic user chunk (`[Request interrupted
by user]`, or the tool-use variant) is dropped by `Conversation::apply` rather than pushed as a
`ConvBlock::User`: not drawn, and not in the sent history Up walks.

**The glyph says the state; the word lives in its tooltip.** `state::status::conversation_status` reads
`launched`, `run`, `stop_reason`, `pending`, `blocks`, `accepts_input` and `config` into a `Status` — **a
pair**: a `Lifecycle` (Starting, Ready, Idle, Working, Waiting, Unloaded, Ended) and a `Doing` (Queued,
Thinking, Writing, Tools, NeedsYou, Done, Failed, Unknown), both derived, nothing stored on `Conversation`.
A delegate speaks the same dictionaries, so a main agent, an agent card and a subagent row share one
vocabulary; `ui::conversation::lifecycle` is the first half alone, for surfaces that draw a dot. `Waiting`
outranks the turn it blocks, so `Working` never carries `Doing::NeedsYou`. `Unloaded` and `Starting` are
both `launched == false`; the transcript tells them apart (a gone harness leaves what it said). The colour
is `lifecycle_colour`: **yellow needs you, blue is working, green is idle, grey has stopped**; the tooltip
is one or two words (`Working · Tools`), never a sentence.

**A chat tab wears the same hexagon its conversation wears elsewhere (T-99, T-102).** The dock draws
`ui::teams::status::status_mark` for a chat tab via `TabInfo::dot_status` (other kinds keep
`dot_colour`/`dot_pulse`): outer ring `Status::lifecycle`, core `Doing`, pulsing only while `Working`. An
unattached tab has none. Shape rule: [`ui-and-design.md`](../tech/ui-and-design.md).

## Contract

A chat tab's own state — id, slot, attachment, picker flag — is local to the UI, like which column draws a
conversation. No message names a `ChatId`; the host answers only about conversations, and what a restore
remembers travels in the interface's opaque view blob. Once attached, a tab speaks the conversation family
of [`../tech/transport-contract.md`](../tech/transport-contract.md): a permission ask arrives as
`ConvUpdate::PermissionRequest` and leaves as one `AnswerPermission` (`request_id`, `option_id`),
`CancelTurn` answering the rest; which surface drew the buttons is not on the wire, so an ask is answered
for them all. A question through `ubiq-ask` speaks that family's `AskUser`/`AnswerAsk`/`AskEnded` instead
and is filed as an `AskRecord` beside the conversation.

## Implementation

**Dock** (`crates/ubiq/src/state/dock.rs`). `ChatId` is a locally minted counter, `Display` and `FromStr` so
it round-trips through the dock's payload like a pane id. `PanelKind::Chat(ChatId)`'s `class` is `Free`,
`home` the right region, `home_in` the same in every mode (right dock, Teams included, where the graph and
its inspector take the centre inline), plus `closable` and `is_drawn`. `PanelKind::chat_home(mode)` is
`home_in` without a tab in hand. `ui/dock/mod.rs`'s `chat_payload` and `chat_from_payload` are the round
trip; a saved leaf naming an id this window did not hold is dropped (a chat id is not the host's to
confirm).

**State** (`crates/ubiq/src/state/chat.rs`). `ChatTab`; `free_chat_slot` (lowest unused slot in the chat
range); `attach_choices`, the one pure function behind every attach list (chevron, `+` menu second stage,
agents screen): which conversations pass the filter, which an open panel of the asking surface already shows
(disabled), and the asking panel's own pick. `chat_picks` builds one list of `ChatPick`s (`New`, `Attach`,
`Inert`) that the frame draws and the click resolves against, by position as every menu does. `New` raises
the start form and notes this tab as the target; the tab attaches when `Message::ConversationStarted` lands.
`Attach` is immediate and `pick_chat_row` follows it with `restore_composer_draft`.

**Slots** (`crates/ubiq/src/state/agents.rs`). `COLUMNS_MAX`, `CHATS_MAX` and `COMPOSER_SLOTS = COLUMNS_MAX
+ CHATS_MAX + 1`; the extra is `SINK_SLOT`, the kitchen sink's bench. Selecting a Teams card has no slot of
its own: it opens an ordinary chat tab (`AppState::open_teams_agent_panel`). **The tab is the active
project's; the attachment need not be**: under `TeamsSpan::Window` a card may belong to any project, but the
tab must be the one on screen's or `sync_chat_panels` gives it no panel. `agent_for_slot` answers the
foreign agent and the send resolves its project through `project_of_agent` (`AppState::steer_column`).

**Lifecycle** (`crates/ubiq/src/app/chat.rs`). `open_chat_tab` mints a tab and slot; `open_chat_tab_now`
also docks it, only when there is a conversation to put in it, via `PanelEdit::Reveal` (not `Open`), since
the titlebar shortcut or a just-started conversation may fire with the right region put away. `attach_chat`
sets or clears the attachment; `toggle_chat_picker` and `dismiss_chat_picker` own the open flag;
`closed_chat_tab` drops the `ChatTab`, clears the slot's draft and leaves the conversation alone.
`settle_persistent_chat` attaches a project's seed tab to its persistent agent once the work naming it
arrives and **does not bring the right panel on screen**: the tab is queued as `PanelEdit::Open` (joins the
region's group without reopening it; `Reveal` is for user gestures). The region is where
`ModeLayout::default_for` or the saved arrangement left it (`workbench.md`).
`OpenProject::persistent_settled` guards it to once per project, so a later `WorkList` cannot reopen a tab
the user closed; it runs from `enter_project` and from the `WorkList` answer.

**Focus mode** (`crates/ubiq/src/app/chat.rs`). `toggle_chat_focus` and `toggle_chat_focus_key` (the
`ToggleChatFocus` action, bound in `app/mod.rs`) set `workbench.chat_focus` and focus the pooled
composer; `close_chat_focus` clears it, and `closed_chat_tab` and the detach path clear it for their own
tab; `drop_stale_column_focus` in `app/agents.rs` clears a column's, from `bench_agent` and `fill_columns`.
`ui/chat/mod.rs`'s `focus_modal` paints the modal at the window root from `ui/shell.rs`, and its
`body` draws the `Focused` placeholder in the panel; `ui/chat/sidebar.rs` draws the header button.

**Delete** (`crates/ubiq/src/app/wire.rs`). `Message::ConversationDeleted` (the answer to `EndConversation`)
drops the conversation and its `WorkAgent`, prunes it from columns, and **closes** every attached chat tab
through `AppState::close_chat_tab_in`, which takes the project id (the delete can arrive while the window
looks elsewhere). Going through that method rather than dropping the `ChatTab` removes the dock leaf and
clears the slot. A detached tab would be an empty panel where a conversation used to be (`D100`).

**Population** (`crates/ubiq/src/app/panels.rs`). `sync_chat_panels` squares the dock tree with
`OpenProject::chats` on entering a project (after `OpenProject::new` seeds the first tab) and at the end of
`settle_layout`, so a restore that dropped an unfamiliar id is squared at once. `settle_panels` skips
the seeded tab's `Open` edit through `AppState::is_idle_chat` — an unattached chat bound for the
mode's chat home while that region is anything but **open and empty** — so IDE mode starts with no
empty agent panel. Open and empty takes one (`toggle_region`'s gesture); admitting it into any open
region put a chat beside Git's changes panel and into Git's blob (`D156`).

**A mode switch never places a chat tab.** `settle_layout`'s leftover loop keeps a chat panel the
incoming blob does not name and drops only its *placement* (a `ChatId` is minted per process, so a
rebuild cannot recreate it); the tab returns from its own mode's blob or the user's reveal.
**Dropping the placement is not closing the tab** (`T-52`): the library reports an installed-over
panel as it reports a clicked ×, so the sweep marks it with `WorkbenchPanel::displace` (taken, not
read: one removal) to keep `closed_chat_tab` from firing on it.

**Rendering** (`crates/ubiq/src/ui/chat/`). `mod.rs` resolves the attachment once (`attached`) and hands it
to the shared renderer (`header: false`) or draws the empty play button; `sidebar.rs` calls
`ui::conversation::lifecycle_header` (or draws the chevron alone, at the same height).
`ConversationView::header` is `true` on the agents column (no `switch`) and here too, with the chevron as
`switch`, so the row is one function everywhere.

**Permission state and drawing.** `state/conversation.rs` holds `Pending` in `Conversation::pending`
(`oldest_pending()`, `answered()`, `Pending::option_for`, `Pending::always_option`, `tool_block_index()`,
`pending_subagent()`, `pending_route()`, `pending_count()`). `ui/conversation/mod.rs` draws `permission()`
and `needs_you_strip()` (badge from `waiting_count()`). `AppState::reveal_permission` (`app/agents.rs`)
takes the surface's own slot, so only the clicked surface scrolls. `answer_permission` forgets one request;
`answer_oldest_permission` is the keyboard path via `read_conversation`; `cancel_turn` clears the set.
`AllowPermission`/`RejectPermission` (`app/mod.rs`) are bound by `install_key_bindings` in both contexts.

**Transcript furniture** (`ui/conversation/mod.rs`). `plan_rows()` is the row walk, so both folds test
without an element (`RowKind`: `Group`, `Thinking`); `tool_group()`, `thought_group()` and `one_block()`
draw the rows; `writing_mark()` the running mark; `tail_signature()` the follow comparison; `build_row()`
carries the gutter and `theme::font(Family::Conversation, Role::Body)` on the row itself (the list lays out
in `prepaint`, where a parent's style is off the stack); `row_signature()` is where width and body size
enter; `to_tail_button()` is the overlay (`AppState::scroll_transcript_to_tail`).
`state::conversation::TranscriptScroll`: `sync()` (the once-a-frame save, restore and follow decision),
`away()`, `request()`/`take_request()`, `to_tail()`, with `off_bottom()`, `away_reading()`, `TAIL_SLACK` and
`own_move` beside it. `AppState::transcript_scrolls` is one per composer slot, indexed as `column_inputs`,
interior-mutable (`render` holds `&AppState`). Folds: `Conversation::open_groups`/`toggle_group()` through
`toggle_conversation_tool_group`; `toggle_thought_group` and `touched_thoughts` through
`toggle_conversation_thought_group`.

**Footer.** Each child `footer()` appends is guarded; with none fired it returns an empty element. The
account snapshot comes from `SettingsState::quota` (`state/settings.rs`), else `snapshot_from_rate_limit`
over `Conversation::rate_limit`; `kit::quota_ring` (`ui/kit/quota.rs`, shared with the harness settings) draws
the bands from `QuotaSnapshot::windows()` and the pace ticks from `window_paces`; `quota_ring_tip` (over
`quota_tip`) words the tooltip, `theme::usage_tone` colours bands and `theme::pace_tick` the ticks. The cache ring reads `show_cache_ring` and `cached_tokens()`/`total_tokens()`
(delegate: `Conversation::delegate_tokens()`, `delegate_spend_tip()`). `stop_button()` is on
`AppState::cancel_turn`, beside the shared `action_button()`.

**Attachments.** `state/conversation.rs` holds `Attachment` (id, project-relative path, size) in
`Conversation::attached`: `attach()` (dedupes by path, refreshing size), `detach()`, `clear_attached()`,
`compose_prompt()` (the one place `@path` is written). `state/file_picker.rs` owns the size vocabulary
(`size_label`, `SIZE_LARGE`, `SIZE_HUGE`, `size_reading`; one `KB` divisor). `AppState::attach_files`
(`app/picker.rs`) takes sizes off the picker's nodes, not a disk; `detach_file` is a tag's `×`. Those two
take the window's active project, where enqueue, `clear_attachments` and queue-row edits take
`project_of_agent`, so a foreign-project card cannot have a file attached or removed (`G326`, with whose
explorer its picker should offer). `attachment_tags()` (`kit::removable_tag`), `sent_attachment_tags()`
(`kit::tag`) and `attachment_preview()` (`kit::popover`) draw; `paste_into_composer` reads the board through
`app/clipboard.rs`'s `clipboard_attachment` and `AppState::project_relative` (a window drop reads it too).
The control is `ui/kit/menu.rs`'s `Picker`: `disabled`, `separators` and `search` sets.

## Failure

| What happens | Result |
|---|---|
| A chat tab has nothing attached | Its page is the play button, and its dock tab reads `New chat` |
| No provider is configured, or the naming fails | The tab keeps its mechanical name and hovers to nothing; nothing is reported |
| A naming answers Markdown, an emoji or a label it was asked not to add | Taken off before the title is written (`ubiq_proto::assist::plain_text`); a line of pure decoration is dropped, not made an empty name |
| The chat range's composer slots are all taken | The strip's `+` and a re-opened empty right region do nothing, the ceiling a ninth column meets |
| A harness is uninstalled between the draw and the click | The start form refuses rather than send a `StartConversation` that fails as a spawn; the tab keeps what it had |
| The remembered harness or agent definition is gone | The form answers nothing rather than open on a start that would fail |
| A saved arrangement names a chat id this window never minted | The leaf is dropped and the tree normalises around the gap |
| An attached file is deleted or moved before send | The mention goes out and the harness answers for it; the interface reads no disk |
| The attached conversation is deleted, in this project or another | Every tab on it closes, dock leaf and slot with it — the one host event that ends a view |
| The provider names no limit | No quota ring; a zero ring would claim an empty window |
| An `AskUser` arrives while its conversation is off screen or a dialog is up | A notification is raised; the transcript's "Ask for feedback" entry is the way back; drafts wait |
| The conversation ends, unloads or its harness dies while an ask waits | `AskEnded` closes it as `Gone`; no control that would send into nothing |
| Two dialogs were registered and the user answers the first | Held, not sent; the prompt carrying both opens when the second is answered (`D192`) |

## Related docs

- [`workbench.md`](./workbench.md) — the dock a chat tab is one panel in, and the composer slot pool it
  shares with a column
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the conversation family an attached tab
  speaks
- [`../tech/decisions.md`](../tech/decisions.md) — `D61`, why a tab is exclusive per surface and not per
  conversation, and `D100`, the one event that closes a view
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens the transcript and its blocks are
  coloured from
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — who owns harness knowledge, for the rows the
  start half offers

## Next steps

- Persist a chat tab's *arrangement* across a restart, not only its attachment.
- Let a tool block open the file it names in the editor.
- Attachments that carry the file's *content* over the bus as a `ResourceLink` — which `agent-manager`
  already accepts — rather than an `@path` mention the harness has to resolve itself. The tags, their sizes
  and their lifecycle are built; [`../backlog.md`](../backlog.md) holds the wire half.
