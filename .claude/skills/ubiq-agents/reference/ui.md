# The surfaces that draw a conversation

Two surfaces host a live agent — a **chat tab** (`ui/chat/`) and an **agents column**
(`ui/agents/column.rs`) — and both draw the same shared view over the same state.

```
state/conversation.rs   Conversation: transcript, pending asks, queue, attachments, spend
ui/conversation/mod.rs  render(): transcript, tool blocks, permission prompts, footer, composer
ui/chat/mod.rs          resolves the tab's attachment once, hands it to render(header: false)
ui/agents/column.rs     one column's tabs, its active agent, render(header: true)
app/agents.rs           every mutator both surfaces call
app/chat.rs             a chat tab's own lifecycle
```

## `Conversation` — `crates/ubiq/src/state/conversation.rs`

`new(id, harness, account)`. The account is **fixed for the conversation's life**: a turn already
taken was taken as somebody.

| Concern | API |
|---|---|
| The transcript | `ConvBlock` — `User(String)`, `Agent { body, subagent }`, `Thought { body, subagent }`, `Tool { call, open }`. `subagent()` / `subagent_id()`; `visible_blocks()` filters to what is being viewed |
| Applying the wire | `is_next(seq)` then `apply(seq, update)`; `ended(stop_reason)`, `unloaded()` |
| Run state | `Run::{Idle, Working, Ended}`, `activity()` |
| Spend | `context_pct()` (the ring), `tokens()`, `total_tokens()`, `cached_tokens()`, `cost_usd()`, `subagent_spend()`, `subagent_tokens(kind)` (a delegate's total and cached part, keyed by subagent **type**), `rate_limit_five_hour_pct()` |
| Permission asks | `Pending { request_id, tool_call, options }`, `oldest_pending()`, `answered(request_id)`, `Pending::option_for(allow)`, `Pending::always_option()` (the allow the harness remembers), `tool_block_index(id)` for the join, `pending_subagent(p)` / `pending_route(p)` (whose transcript, which block), `pending_count(subagent)` |
| The queue | `enqueue(text) -> u64`, `dequeue_front()`, `remove_queued(id)` |
| Attachments | `Attachment`, `attach(path, size)` (dedupes by path, refreshing the size), `detach(id)`, `clear_attached()`, `compose_prompt(typed)` — **the one place `@path` mentions are written** |
| Subagents | `SubagentTab` (with `model` and `waiting`), `subagents()`, `subagent_name(id)`, `has_subagent(id)`, `viewing_subagent()` |
| Scroll | `TranscriptScroll` — one per composer slot on `AppState::transcript_scrolls`, keyed by `TranscriptKey = (AgentId, Option<String>)`: `sync(key, signature)`, `away()`, `request(block)` / `take_request()` / `request_held()`, `to_tail()`, `windows(children)`, `child_bounds(ix)`, `near_viewport(bounds)` |
| Folding | `toggle_tool(id)`, `toggle_group(id)`, `open_groups` |
| Labels | `short_model_label(harness, model)` — one shortener, so a conversation never spells a model two ways |

## `ui/conversation/mod.rs` — the shared view

`ConversationView { id, slot, footer, composer, header }`. `id` is the element-id prefix, so two
conversations on screen do not collide. `slot` is which pooled composer field this surface types
into — an index and nothing else.

| Function | Draws |
|---|---|
| `render(...)` | The whole thing |
| `transcript()` | Walks the visible blocks, folding runs of same-kind calls |
| `one_block()` | The arm reused for an unfolded card and for the ones a fold opens |
| `tool_group()` | The `N earlier calls` row |
| `writing_mark()` | The running mark at the tail |
| `tail_signature()` | What the follow-the-tail scroll compares, over the **visible** blocks it is handed |
| `Built` (private) | The children the walk produces: which block each stands for, and the off-screen ones left unbuilt behind a measured stand-in |
| `to_tail_button()` | The `Go to last message` overlay, drawn only while the transcript is away from its tail |
| `permission()` | The block-attached prompt and its self-contained fallback |
| `needs_you_strip()`, `waiting_count()` | The answerable strip, and its count badge above one request |
| `footer()`, `delegate_spend_tip()` | The run pill, the context ring, `ctx`, `tot`, the cache ring — and what those readouts mean on a delegate's transcript |
| `stop_button()`, `action_button()` | The square Stop, and the Send/Enqueue pair |
| `attachment_tags()` | The wrapping tag row, on `kit::removable_tag` |
| `config_choices()`, `ConfigRow` | The launch-time model / thinking / mode pickers |
| `lifecycle()`, `lifecycle_colour()`, `lifecycle_menu_enabled()`, `lifecycle_mark()`, `lifecycle_menu()`, `LIFECYCLE_ROWS` | The state dot and the three-dots menu — **read in this one module regardless of caller**, `lifecycle_colour` `pub` because the agents column draws the dot on its own title and tabs |
| `subagent_tip()` | A delegate row's hover |

### `Lifecycle`

`lifecycle()` derives one of `Starting`, `Ready`, `Waiting`, `Working(Activity)`, `Idle`,
`Unloaded`, `Ended` from `launched`, `run`, `pending`, `blocks`, `accepts_input` and `config` —
derived rather than stored, so nothing new sits on `Conversation`. **`Waiting` outranks
`Working`**: a request outstanding is the one state that needs the reader, so it is tested before
`run`, and `Working` therefore never carries `Activity::NeedsYou`. `Unloaded` and `Starting` are
both `launched == false`; the **transcript** tells them apart, because a harness that is gone still
leaves what it said and one never started leaves nothing. The dot is a `kit::status_dot`, no new
primitive; the tooltip is one or two words (`Unloaded`, `Working · Tools`), never a sentence.

**Four readings and only four:** `warning` wants you, `info` is working, `success` is idle,
`text_faint` has stopped — `lifecycle_colour`, the one place the mapping is written. Every working
turn is one `info` rather than `Activity`'s own palette, because what a dot glanced at across a
row of columns has to answer is whether that conversation wants the reader. The agents column
draws it on its header title (before the name — that is where the eye lands when it is scanning
columns) and on every tab, falling back to `activity_colour` for an agent with no live conversation
behind it: a record is not idle, it is a record.

**The menu has five rows, `LIFECYCLE_ROWS`:** Stop, Abort, Unload, Resume, Delete, and
`lifecycle_menu_enabled` returns `[bool; 5]` matched by position — Stop only while a turn runs,
Abort and Unload only while launched, Resume only while not, Delete always. **Stop and Abort are
different verbs**: Stop interrupts the *turn*, Abort kills the *process*, which is what is left
when a harness has stopped answering. Abort keeps the conversation, so Resume brings it back; only
Delete is irreversible and only Delete is confirmed. One list of labels, because two copies drifted
a row apart.

`ConversationView::header` decides whether the shared view draws its own bordered lifecycle strip —
the menu alone, the state being the dot on the column's title. The agents column keeps it `true`;
the chat panel sets `false` and draws `lifecycle_mark` and `lifecycle_menu` inline in its own
toolbar row, at opposite ends — one set of functions either way.

## The transcript's rules

- **A running turn is drawn at the tail.** While `Working` with nothing waiting, `writing_mark`
  pulses three dots a third of a cycle out of phase, in `Activity`'s own colour, with the
  activity's word beside them. Not drawn while an ask is up: the question on screen is what is
  happening. A spinner would claim progress nothing measures. The run folds into `tail_signature`,
  so the mark appearing scrolls the tail in.
- **A run of same-kind tool calls folds to its last card.** Three or more (`GROUP_MIN`) become the
  last card plus one `N earlier calls` row in that kind's colour, opening in place. Twelve `READ`s
  are twelve rows of furniture between two sentences. Two cards would become a row plus a card —
  no shorter, one more thing to learn — which is where the floor of three comes from. **Never
  folded**: a `Delegate` call (a spawned agent is a second transcript, not a step), and a call with
  a permission ask attached (a prompt behind a counter is a turn that deadlocks).
- **A permission ask is drawn on the tool call it authorises.** Joined by tool call id; the block
  reads as awaiting approval and the buttons sit under it. One button per `PermissionOption`,
  labelled from its `name`, differentiated by its `kind` — allow from reject, with the "always"
  variant marked as lasting. A request naming a call this transcript never saw degrades to a
  self-contained prompt at the end.
- **Several asks may be up at once and every one blocks.** Ordered by arrival, keyed by
  `request_id`; there are no timeouts, so an unanswered request stalls the turn. A "needs you"
  strip above the footer carries what is outstanding, because a prompt attached partway up can be
  scrolled out of view. Cancelling the turn discharges all of them.
- **The strip is answerable, and it is the way to the prompt.** Yes / All / No for the oldest
  request, each button drawn only where that request offered an option of that reading
  (`option_for` for allow and reject, `always_option` for the allow that lasts) — a button that
  answered a lasting allow with the plain one would lie about lasting. Clicking the label instead
  runs `AppState::reveal_permission`: it switches to whoever raised the request and scrolls to the
  call it authorises, both from the one `pending_route`, so the two ways of answering land in the
  same place. A request whose call the transcript never saw routes to the tail, where its
  self-contained prompt is. Above one outstanding, a count **badge** beside `NEEDS YOU` rather than
  a clause in the label; a delegate's request carries the delegate's name.
- **Scroll is per transcript, and the tail is followed only for a reader on it.**
  `TranscriptScroll` is keyed by `(AgentId, Option<subagent>)`, because switching to a delegate is
  arriving at a *different* transcript — a slot moved to a delegate and back restores both
  positions, and one never shown opens on its tail. `tail_signature` reads the **visible** blocks,
  so the main agent writing under a delegate's transcript scrolls nothing.
- **`Go to last message` is an overlay, not a column control.** Over the transcript's lower right,
  drawn only while it is away from the tail, so nothing moves when it appears. It scrolls and does
  nothing else; the next thing said resumes the follow.
- **Above 40 blocks the transcript windows.** A child the last frame painted clear of the viewport
  (plus a 2000px margin) is replaced by a box of its exact measured height, so the scroll position
  cannot move — measured, never guessed. Below the floor every block is built every frame: the
  bookkeeping outweighs the drawing, and the first frame has nothing measured to read. The same
  pass (`Built`) records which block each child is, which is how a jump resolves a block to a
  child.
- **⌘⌥Y allows and ⌘⌥N rejects the oldest ask**, with the first allow-kind or reject-kind option
  that request offered — and does nothing where it offered none of that reading, rather than
  answering with the other. Bound in **both** `Workbench` and `Input`, so a composer holding focus
  does not swallow the answer to the question blocking the turn being typed into.
- **A link in a rendered reply goes where it points** — the transcript hands `ui::on_link` to its
  `TextView`. A path merely *mentioned* in prose is text, not a link.

## Delegates

A conversation that spawned subagents grows one row at the top of the bottom block, reading
`3 active subagents of 10` — the `of 10` dropped while every delegate is still working, and the
row falling back to `10 subagents` once none is, because a count twice over and a zero are both
noise. One that spawned none grows nothing. Opening it draws one row per agent **upward** over the
transcript, through the same `anchored` + `deferred` pair every menu uses, so the composer never
moves under the cursor. Clicking a row switches the transcript; the main agent is always a row,
because it is the way back. A subagent whose spawning call is not in the transcript reads
`unknown` rather than being claimed to be running.

A delegate says what it is answering with: its own model beside its name in the reading strip,
**the same model faint beside the name on its row** (a reading only on hover made the reader hover
three rows to compare three), and its kind / model / thinking on its row's hover — all from the one
`SubagentTab`. Nothing is borrowed from the parent, and `thinking` is `None` on every harness
today. The `AGENT` block that spawned an agent is the same door, inert until that agent has said
something.

**A blocked delegate reads `need you` in place of its status**, `need you ×N` above one request, in
the warning tokens — `SubagentTab::waiting` off `Conversation::pending_count`. In place of rather
than beside: a delegate waiting on a human is not doing anything, so `running` and the question
together would be one of them wrong. The main agent's row is read the same way.

## Attachments

The composer's `+` raises the window's file picker; what comes back is one tag per file in a
wrapping row under the token readout and above anything queued. Clicking a tag opens the file;
its `×` takes it off. A tag says the file name; its tooltip says the whole path with the size.

**The colour of a tag is the size of the file** — warning over `SIZE_LARGE` (300 KiB), danger over
`SIZE_HUGE` (500 KiB), with the reason in the tooltip, so a file large enough to cost a noticeable
part of the context window says so *before* the turn is spent on it. A file no host reported a
size for is drawn plainly: an unknown size is not a small one.

**An attachment belongs to the conversation, not to the composer slot** — it sits beside the draft
and the queue, so unsent content follows the conversation from one surface to another. Nothing new
crosses the bus: on send, every path is composed into the one `PromptAgent` text as an `@path`
mention after what was typed. Enqueue does the same composition into the queued text and clears
them, because a queued prompt is one string and a queue row carrying its own tag list would be a
second composer. `state/file_picker.rs` owns the size vocabulary both the picker's rows and a tag
read — `size_label`, `SIZE_LARGE`, `SIZE_HUGE`, `size_reading` — with one `KB` divisor, so the
colour and the printed number can never disagree.

## The footer

**`ctx` is a level, `tot` is a flow.** The ring and `ctx` are how full the context window is *now*
— a number that falls when the conversation is compacted — and `tot` is every token ever billed,
subagents included, which only grows. Every readout says which it is on hover.

A **second ring beside `tot`** is `cached_tokens` over `total_tokens`, in the `info` tokens rather
than the accent ones (a second accent ring would read as the same fact twice), saying
`cached X / Y Z%` on hover. Off unless asked for — it is a cost-of-running reading rather than a
how-is-this-turn-going one, and the footer row is glanced at.

**The row reports whoever is being read.** On a delegate's transcript `tot` and the cache ring are
that delegate's spend (`subagent_tokens`, `delegate_spend_tip`). Two wire limits show through, and
neither is smoothed over: `UsageRecord::subagent` is a subagent **type**, so two `general-purpose`
delegates share one bucket and the tooltip says so rather than dividing it to look exact (`G194`);
and a subagent's usage report repeats the **parent's** occupancy, so there is no per-delegate
context level to draw and **the context ring is dropped** rather than borrowed (`G195`).

**Stop is there for the whole of a running turn, and it is a filled square.** The moment a message
is sent is the moment a reader most wants it back, so a control that appears only while the field
is empty vanishes exactly when it is needed. It sends `CancelTurn`: the turn ends, the conversation
and harness stay, the next message goes to the same agent — which is why it is a square and not a
cross, a cross reading as *close this*. With something typed during a running turn, Enqueue sits
beside Stop; an idle conversation keeps the one Send. All three are what Enter answers through
`AppState::send_or_enqueue`, so the buttons and the key never disagree.

## The composer pool

`state/agents.rs` defines `COLUMNS_MAX = 8`, `CHATS_MAX = 8`, `SINK_SLOT`, and
`COMPOSER_SLOTS = COLUMNS_MAX + CHATS_MAX + 1`. The window builds every text area **before the
first frame**, because the subscription mirroring what is typed has to be held for the window's
life. Columns allocate from the low range, chat tabs from the range above it. What was typed at
one surface never turns up in another's field, and closing one clears its slot's draft before the
slot is handed on. Also here: `COMPOSER_ROWS_MIN/MAX`, `COMPOSER_ROWS_MAX_DEFAULT`,
`COMPOSER_ROW_HEIGHT`, and `COLUMN_MIN_WIDTH = 360.0`.

## The chat panel

**A chat tab is `PanelKind::Chat(ChatId)`**, one panel per instance, `class: Free` so it may sit
in any dockable region; its home is the right edge at `CHAT_WIDTH`. The id is minted locally — it
is UI arrangement the host never hears about — and round-trips through the dock's saved payload
via `chat_payload` / `chat_from_payload`. A saved leaf naming an id this window did not mint is
dropped, the way an unfamiliar saved terminal leaf is.

**One control both starts and attaches.** In the tab's own header, wearing `Play` and reading
`Start or attach` when empty, or the harness glyph and the conversation's name when attached. An
**empty** tab is offered both halves (`Start new` from the harness list, `Attach running` from the
project's conversations); an **attached** tab only the second, because starting from a tab that
already shows a conversation would leave that one with no view and no way back. Typing filters
both halves, and drops the headings and hairlines while it does — what a search shows is the
matches. A conversation attached to a *different* tab draws disabled and is never dropped from the
list, because a vanished row reads as ended rather than taken.

**An empty tab opens on the last harness anything was started on** — `InterfacePrefs::last_start`,
interface scope rather than a project's, because which harnesses this machine has is a fact about
the machine. A hint, never a promise: a harness uninstalled since preselects nothing.

**Adding a view is the tab strip's gesture, not the panel's** — a `+` on the dock's own strip,
offered on any group holding a chat. Starting a *harness* is the header control's; adding a *view*
is the strip's. Closing the last chat tab is allowed; reopening the right region mints a fresh one.

**The header reads left to right in the order a reader asks**: the state mark says what the
conversation *is*, the control says what it is *on*, the three-dots says what can be *done to it*.
Nothing attached draws the control alone. Nothing in the header names the tab — the dock's tab
already carries the name.

Code: `state/dock.rs` (`ChatId`, `PanelKind::Chat`), `state/chat.rs` (`ChatTab`,
`free_chat_slot`, `attach_choices`, `chat_picks`), `app/chat.rs` (`open_chat_tab`,
`new_chat_tab`, `attach_chat`, `pick_chat_row`, `toggle_chat_picker`, `dismiss_chat_picker`,
`closed_chat_tab`), `app/panels.rs` (`sync_chat_panels` squares the dock's tree with
`OpenProject::chats`; `toggle_region` mints a tab when an emptied right region reopens).

**Rows and actions are matched by position.** `chat_picks` builds one list of `ChatPick`
(`Start`, `Attach`, `Inert`) that the frame draws and the click resolves against — no second
reading of the harness list to drift out of step. A `Start` resolves through
`AppState::start_harness_choice` in `app/agents.rs`, which the agents menu also calls; what a
harness row *says* is `ui::agents::harness_offers`, not a second copy of the labelling.

## The agents screen

**A column is a place to talk to an agent, not a place an agent lives.** A column holds tabs, and
more than one tab is a group. Dragging a tab onto another column groups it; dragging past the last
column gives it a column of its own. Eight columns fit the row (`COLUMNS_MAX`) — **the ceiling is
on columns, not on tabs**: with the row full, a benched agent clicked in the sidebar is grouped
into the focused column rather than refused, because "show me this agent" is a request the screen
can always honour.

**Closing a tab benches the agent; it does not end it.** The sidebar still lists it, marked
`bench`, and one click brings it back. `Close all` benches the whole row the same way — it calls
`bench_agent`, not `EndConversation` — and is shown only when there is something to bench.
**Nothing on this screen kills an agent.**

**The bench is computed, not stored**: every agent the host reports that no column is showing.
`AgentsView::bench_rows` is what a column's `+` opens, and `pick_agent_bench_menu` re-reads the
same list before resolving a click. Agents already on screen elsewhere are shown disabled rather
than dropped.

**The arrival arrangement is one column per session that has an agent in it**, holding that
session's agents in the order the host listed them — a session running several agents arrives
grouped rather than spread. After that, an arriving agent goes to the bench: an arrangement the
user has changed is not something an arriving record may undo. `prune` drops what the host has
forgotten and nothing else.

Code: `state/agents.rs` (`AgentsView`, `Column`, `BenchRow`, and the arrangement mutators
`arrange`, `prune`, `reveal`, `open`, `open_in`, `bench`, `split_off`), `ui/agents/mod.rs`
(`render`, `harness_offer`, `harness_offers`, `new_agent_menu`), `ui/agents/sidebar.rs`,
`ui/agents/column.rs`.

## `app/agents.rs` — the mutators both surfaces call

Arrangement: `reveal_agent`, `group_agent_into`, `bench_agent`, `select_column_tab`,
`focus_agent_column`, `toggle_agents_session`, `start_tab_drag`, `drop_tab_on`, `drop_tab_at_end`,
`open_agent_bench_menu` / `pick_agent_bench_menu` / `dismiss_agent_bench_menu`.

Turns: `prompt_agent`, `send_or_enqueue`, `recall_last_message`, `edit_queued_message`,
`delete_queued_message`, `cancel_turn`, `steer_column`, `agent_for_slot`.

Composer: `start_composer_resize`, `drag_composer_resize`, `end_composer_resize`,
`set_composer_rows`.

Permissions: `allow_permission`, `reject_permission`, `answer_permission` (sends one answer and
forgets that one request), `answer_oldest_permission` (what the keyboard resolves through
`read_conversation`), `reveal_permission` (the strip's label — takes the asking surface's `slot`,
because the scroll belongs to the surface and only the one clicked moves).

Transcript scroll: `scroll_transcript_to_tail`.

Lifecycle: `end_conversation`, `abort_agent`, `unload_agent`, `resume_agent`,
`close_all_conversations`,
`confirm_end_conversation`, `open_conversation_menu` / `pick_conversation_menu` /
`dismiss_conversation_menu`.

Transcript and config: `toggle_conversation_tool`, `toggle_conversation_tool_group`,
`view_conversation_agent`, `toggle_conversation_subagents`, `pick_agent_config`,
`toggle_agent_config_menu`, `dismiss_agent_config_menu`.

Starting: `open_new_agent_menu`, `pick_new_agent_menu`, `dismiss_new_agent_menu`,
`start_harness_choice`, `remembered_choice`.

## Contract, in one line

A chat tab's state — its id, slot, attachment, whether its picker is down — is local to the UI.
**No message names a `ChatId`**; the host answers about conversations, never about which surface
is looking at one. Which surface drew a permission button is not on the wire, so an ask answered
anywhere is answered for the conversation.
