---
id: feat-workbench-agents
title: Agents mode — the columns
kind: feature
status: draft
summary: The rail's Agents mode — a row of parallel columns, each a transcript and a composer over one live conversation, tabs that group agents into a column, the bench of agents no column is showing, the sidebar that lists every conversation the window holds, the three-dots menu over a live agent, and the New agent form all three surfaces raise.
read_when: you are changing the agents screen — its columns, its tabs, what a tab drag means, the bench, the sidebar, a column's composer or footer, or the New agent form
updated: 2026-09-25
verified: 2026-09-25
code_anchors: [crates/ubiq/src/state/agents.rs, crates/ubiq/src/app/agents.rs, crates/ubiq/src/state/new_agent.rs, crates/ubiq/src/app/new_agent.rs, crates/ubiq/src/ui/new_agent.rs, crates/ubiq/src/state/conversation.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/tests/conversation.rs, crates/ubiq/src/ui/agents/mod.rs, crates/ubiq/src/ui/agents/sidebar.rs, crates/ubiq/src/ui/agents/column.rs, crates/ubiq/src/state/status.rs, crates/ubiq/src/ui/work.rs, crates/ubiq/src/ui/teams/status.rs, crates/ubiq/tests/agents.rs, crates/ubiq/src/app/mission.rs]
depends_on: [feat-workbench, tech-ui, feat-chat]
review_cycle: monthly
---

# Agents mode — the columns

## Purpose

Agents is the rail mode the user *talks to* the agents in: a row of columns side by side, each
one a transcript and a composer over a single conversation, so several runs can be followed at
once without switching anything. It is the counterpart of the graph screens, which arrange the
same agents rather than converse with them ([the Teams graphs](./workbench-teams.md)).

## Behaviour

**The hexagonal status mark is on every tab, and nowhere else (T-99/T-102).** It used to also sit
beside the column's own title, a line above the lifecycle strip's own reading of the same fact —
two marks for one state, a line apart — and that copy is gone: a column's header now carries only
the role mark, the name and the role label. `ui::teams::status::status_mark` is what a tab draws,
off `state::status::agent_status(agent, conversation)`: the outer hexagon reads the lifecycle, the
inner core the activity or the result, and the core pulses only while the lifecycle is `Working`.
The dock's own tab strip wears the same mark for a chat tab (`TabInfo::dot_status`), so a column's
tab and a chat tab agree pixel for pixel; a tab with no live conversation reads `agent_status`'s
record fallback rather than nothing. The one first line `ui::conversation::lifecycle_header` draws
— the three-dots menu and, flush against the strip's right edge, the current-action `status_chip`
— is where a reader scanning for *what* an agent is doing looks now; the hexagon on the tab says
*whether* it still can. A grouped column's tabs and its `lifecycle_header` read the same
`Status`, so the two cannot disagree about which of its agents wants something.

**A column is a place to talk to an agent, not a place an agent lives.** Which column an agent's
conversation is drawn in is the interface's own fact: no message carries it and no drop sends one,
the same rule the graph's card positions follow. What the host owns is which agents exist and what
each is doing, and the screen reads that and never writes it.

**A column holds tabs, and more than one tab is a group.** Dragging a tab onto another column puts
the two agents in one strip, which is how a hand-off is read — the plan and the build side by side,
one column wide. Dragging it past the last column gives it a column of its own again, and a tab
alone in its column is what that gesture produces anyway, so the drop changes nothing. The column a
drop would group into lights up, and the strip at the end of the row lights only for a drop that
would do something: a target that promises a change it will not make is worse than one that stays
dark. The strip over the columns counts them — how many there are, how many agents they hold and how
many of them are grouped — and names both gestures, because neither leaves a mark on the interface
to be discovered from. Its controls are `Close all` and `New agent`. `Close all` benches every agent
on screen — `bench_agent` for every tab in every column, the same thing a tab's own close already
does, not `EndConversation` — and is shown only when there is something on screen to bench; a row
with no columns gets no button rather than one that would silently do nothing.

**`New agent` is a `+` that asks two questions, one stage at a time.** *New agent* raises the New
agent form; *Attach existing agent* opens a searchable list of the conversations this project
already has. Two rows rather than one flat list because the two are different kinds of question,
and a list that can run to every conversation in the project does not belong hanging under a row
that is not it. The second row is the second *stage* of the same menu rather than a submenu — the
kit has none — and it is drawn as a `kit::Picker` so it can be filtered, while the first stage
stays a context menu, its two fixed rows having nothing to narrow. *Attach existing agent* is
disabled when there is nothing to attach: a row that opens on emptiness is worse than a row that
says so.

**One `+` menu, three surfaces.** The agents screen's control, the IDE chat strip's `+` and the
kitchen sink's bench all raise it, and `WorkbenchState::new_agent_menu` carries which of them
asked — so what a pick *does* differs while what it *offers* does not. A conversation picked on
the agents screen is revealed in a column, one picked on the chat strip opens a tab attached to
it, one picked on the bench becomes what the bench reads. What "already taken" means is the
asking surface's own business and nothing else's: `state::chat::attach_choices` is the one builder
behind every attach list in the window, and it disables the conversations the *other* panels of
that surface are showing — never dropping them, because a row that vanishes reads as a
conversation that ended rather than one already open. The two surfaces may show the same
conversation at once, and the host is never told.

**On the chat surface, the same `+` gains a third part: *New mission* and a searchable *Missions*
list (§6.3).** A separator, then *New mission* (`AppState::open_new_mission`, the toolbar's own
dialog) and *Missions*, disabled when the project has none open — every mission not `Completed` or
`Abandoned`, most recently active first, searched by key and title (`AppState::open_missions`).
Neither row is offered on the agents screen's own control or the kitchen sink's bench, since neither
starts or opens a mission. Picking a row reveals that mission's side panel
(`AppState::open_mission_panel`, [the tasks board's own mission panel](./workbench-tasks.md)), the
same `PanelEdit::Reveal` device *Attach existing agent* uses to bring a panel the dock already holds
forward rather than opening it twice.

**The New agent form asks every question a start answers** — `D92` is why it is one form and why a
agent definition is the same form saved. It is a modal, `state::new_agent::NewAgentForm`,
painted by the shell over whatever is underneath — the same treatment the clone and remote-connect
modals get, and for the same reason: it is raised from the workbench rather than from any one page.
Its rows, top to bottom, are the target, then the harness and its identity, the model, the
reasoning level, the permission mode, a subagent ceiling, a persistence checkbox and an opening
prompt. **Every row is one line**, the label and a hint mark on the left and the control on the
right, with the explanation on the mark's hover rather than under the row — eight notes is what
made this form taller than the window. **Everything below the first row is drawn inert until a
target is chosen**, faint and taking no click rather than hidden, because a form whose shape jumps
as it is filled in has to be re-read after every answer.

**The target is a harness signed into an identity, or a saved setup, and the form asks which of
those two through a tab** (`NewAgentTab`, drawn as the row of choice pills every other set-of-one
in the window is drawn as). They were one list with a hairline in it, which made the second group
read as a footnote to the first; they are two questions, so they are two tabs. Moving between them
starts the answer again — a definition is not an answer to "which tool, as whom" — while the
project the start is aimed at, the task it is assigned to and the harness catalogue already
fetched stay where they are.

**Agents** lists the agent definitions and nothing else, one row each, read as
`reviewer — Codex · gpt-5 · high`: the name, then what it runs on, because that is the question
this tab does not ask again. It offers the global definitions plus the ones saved inside the
project this start is aimed at, those labelled `· this project`, and a project definition of the
same name as a global one stands in its place here and in no other project (`D158`). **A disabled
definition is not a row** (`AgentDefinition::disabled`, read through
`SettingsState::startable_definitions_in`): it is listed on the settings screens and offered
nowhere a run begins. Under the dropdown sits the line of what the chosen definition runs on and a
**Customize** button, which turns the harness, model, thinking-effort and mode rows back on so
this one start can override them; until it is pressed those rows are not drawn, because the
definition has already answered them.

**Harness** is the other question and draws what the form always drew, minus any way to pick a
definition. `WorkbenchState::harness_choices` offers it: the same `ListAgentTypes` answer the
new-pane menu reads plus the accounts signed in, keeping only the harnesses whose
`AgentTypeInfo::chat` is true, because a harness with no structured bridge (Grok) can draw a
pane's screen but has nothing to turn into a `ConvUpdate` — offering it here would start a
conversation that never speaks. **No harness is a new-pane row at all** — that menu offers the
terminals that are not agents, and starting one is this form's job. What survives is grouped under
a `Configured` heading, one row per `(harness, account)` pair. **A bare harness is not a row.**
Starting one with nothing else answered is what the form is for, and it asks the identity, the
model, the level and the mode in the same breath — a row that launched on whatever the library
happened to resolve was that same launch with every question skipped. A harness whose binary is
not on this machine is still what a row draws disabled over, so a list says a tool is missing
rather than omitting it.

**An agent definition is a saved answer to the same questions**, which is why the settings page's agent definition
form is this same form with a different `Purpose`: no tabs and no agent definition row in the
target picker — a definition is always written against a harness — no `Start`
button, and a name and a `Save` instead. Two forms asking one set of questions differently is how
the two drift apart. Picking an agent definition as the target fills every row below from what it saved;
`Save agent definition` on a start form writes the answers back out under a name the window's prompt asks
for, offered only when the target is a bare harness — saving a start that already points at a
agent definition would be writing that agent definition over itself.

**The form's footer offers the MCP servers Ubiq itself injects.** `MCPs` opens a checklist of the
`McpInfo` rows the host answered `ListMcps` with — one tick box per server, its title, what it is
for and the tools it answers — and what is ticked rides out on `StartConversation::mcps` or is
written into `AgentDefinition::mcps`. This build lists Test, Project info, Manage Ubiq tasks, and Use
ubiq tasks. It is a checklist and not a picker because several servers may be
asked for at once, so the panel is the same `deferred`/`anchored` shape the pickers are built on
with check-box rows in it, opening upward from the footer and staying down across ticks. The
catalogue lives on `WorkbenchState::mcps`, one list for the window: what this build can inject is a
property of the build, not of the harness, the identity or the setup being filled in. Until the
host answers, the button is drawn faint and takes no click, the way the rest of the form draws a
row with nothing to offer. `Custom policies` beside it is still the predisposition for something
the host does not answer yet.

**The model and the level are known before anything is started.** Opening the form sends
`ListHarnessCatalogue` for the chosen harness and identity, and `HarnessCatalogue` comes back with
the models that harness will answer for, the reasoning levels each accepts, and what this harness
was last actually launched with. Until it lands the model row says it is asking rather than
drawing an empty list, which would read as a harness with no models. A catalogue for a harness or
an identity the form has since moved off is dropped: a probe is slow exactly once, and a slow
answer arriving behind a fresh one would overwrite it. Preselection runs in order of how much it
knows — what the form was opened holding, then the last launch, then the harness's own default.
The mode picker opens on `AgentTypeInfo::unattended_mode`, the harness's own word for "ask
nothing", so the interface never guesses which id means all permissions.

**The footer offers two ways to start the same run.** `Start` sends `StartConversation` and the
agent arrives as a transcript; `Start in terminal` sends `SpawnWorkspace` and the same harness
arrives as a pane, drawing its own screen under a pseudo-terminal. Everything the form asked rides
out either way — the identity, the saved setup, the model, the level, the permission mode and the
ticked MCP servers are one `AgentPicks` record on both messages, and isolation is a host setting
applied to both faces alike, so the two buttons differ in the face the run wears and in nothing
else. The terminal button carries no preamble: a subagent ceiling and an opening prompt are folded
into a first turn by a composer, and a pane has none — the user types into the harness itself. A
non-chat harness is not on offer here at all, which is `G230`.

**Nothing is created until the conversation lands.** The `+` writes down *where* a start is aimed
and raises the form; the column or the tab is minted when `Message::ConversationStarted` arrives,
so a form the user dismisses leaves no empty tab and no empty column behind. A start that never
happened releases the aim too, so the next conversation from anywhere else is not claimed by it.

**The form opens on the last thing that worked.** `InterfacePrefs::LastStart` records the harness,
the identity, the saved setup, the permission mode and the subagent ceiling a conversation was last
begun with. The model and the level are deliberately not in it: the host already remembers those
per harness and hands them back with the catalogue, and the two the host knows nothing about are
the two written here. It is interface scope rather than a project's — which harnesses this machine
has and which account is signed into them is a fact about the machine — and it is a hint, never a
promise: a harness uninstalled or an agent definition deleted since answers nothing rather than opening the
form on a start that would fail.

**A subagent ceiling is said to the agent, not passed as a flag.** No harness has such an option,
so the only way to ask for one is to say so, and the form writes it — with the opening prompt, if
there is one — into a preamble held against the conversation rather than sent as a turn of its own.
A transcript that opened on a directive the user never wrote would read as the conversation
beginning with someone else's words. `AppState::send_prompt`, the one place `PromptAgent` is built,
folds the preamble in front of the **first** turn the user actually sends and tells the conversation
what it folded in; the harness echoes the turn it received, verbatim, and `Conversation` takes the
preamble back off that echo, so the harness reads the directive and the transcript never shows it.
The preamble is taken rather than read — it belongs to exactly one turn — and it is spent whether
or not it matched, so a harness that reformats its echo cannot leave it armed to cut the front off
a message the user really did write.

**The persistence checkbox is disabled and always false.** A conversation that survives a restart is
built (`D97`), but it is marked from the conversation's own three-dots menu rather than at the start
form: the flag lives on a row keyed by the conversation, which does not exist until the start has
been answered. Wiring the checkbox means carrying the intent through the start and setting the flag
once the row is written, which is `G223`.

**The opening prompt is the form's keyboard rest.** The form opens with the keyboard in it, which
is also what puts the modal on the focus path — `⌘⏎` confirms the form from inside a field, and
Escape reaches `AppState::cancel_dialog`, only because a focused element inside the modal is what
the key is dispatched from. Every one of the form's lists takes the keyboard while it is open, for
the `picker_search` field it shares with every other searchable list, and **hands it straight back
to the prompt when it closes** — picked from, dismissed or peeled by Escape. That is not a nicety:
the filter field is unmounted with the list, and a focus handle on an element nothing draws any
more is a keyboard nobody owns, which is a form that has quietly stopped answering both keys.

**An outside click while a list is down belongs to the list.** A dropdown inside the modal is
painted above it and so lands outside its bounds — the modal section of the UI and design document
says why — so the form ignores the outside click while one is open rather than dropping a
half-filled form because a list was down. One gesture peels one layer, the same rule Escape obeys.

**`Start` sends `StartConversation` at once**, carrying the model, the level and the mode the form
answered, each of which outranks the agent definition's own record. The conversation's name is not the UI's
to set: the host derives it from the harness's command, with a per-project counter from the second
occurrence onward. What a start eventually makes is a conversation rather than a pane — the same
question asked of the other face of a workspace, and a conversation has no size.

**The agents screen lists only what this window can talk to.** The work projection is wider than
that — it carries the host's mock work-thread fixtures too — so every reader on this screen goes
through `AgentsView::live_agents`, which keeps the agents this window holds a live `Conversation`
for. The sidebar, the empty page's note and the column refill all read it. `Teams` narrows the same
way, through `state::teams::live_work`, once per project its span names rather than for the active
project alone; `[Teams]` reads the whole projection, because a graph is a map of who spawned whom
and a fixture has a place on one.

**Closing a tab benches the agent; it does not end it.** This is the one place the screen
deliberately reads differently from a terminal pane, whose close kills the harness behind it —
[`panes-and-terminals.md`](./panes-and-terminals.md). A tab is a view onto a conversation, so taking
it off screen leaves the agent running: the sidebar still lists it, marked `bench`, and one click
brings it back. Nothing on this screen kills an agent — `Close all` benches the whole row the same
way a single close does. Ending an agent for good is the shared conversation view's own three-dots
menu, below, not a gesture on the tab.

**A three-dots menu, top left of the shared conversation view, is where a live agent's five
lifecycle verbs live.** Stop (`CancelTurn`) interrupts the turn in flight and leaves the harness up.
Abort (`AbortConversation`) kills the harness's process outright, keeping the conversation, its
transcript and its run directory, so Resume brings it back — it is what is left when a harness has
stopped answering and Stop has nothing to interrupt it with, the one verb that does not ask.
Unload (`UnloadConversation`) asks the harness to shut down and keeps the same three things — the
pickers return, exactly as a conversation that has not launched yet reads.
Resume (`ResumeConversation`) starts the harness again under the same agent, with no prompt. Close
(`EndConversation`) ends the conversation outright, taking the run directory and the transcript with
it, and is the one item confirmed before it fires rather than acted on the click — it is the only
irreversible one on the menu.
Fork (`ReviveConversation` with a fresh `agent_id`) copies the run directory and launches a second
agent in the copy from this point on, leaving the source untouched.

**The menu is in three sections, and the two hairlines between them are rows.** What the harness is
doing (Stop, Abort, Unload, Resume), what the reader can reach for (Info, Fork and the three
toggles), and what puts the conversation away (Hide, Close). A separator occupies an index in
`lifecycle_menu_rows` and a dead arm in `pick_conversation_menu`, because the rows are dispatched by
position: a hairline that did not take a place in the list would slide every row below it onto the
wrong verb.

**Hide and Close are the two ends of the last section, and only one of them ends anything.** Hide
closes the chat tab attached to this agent and nothing else — the conversation is the host's, keeps
its harness and goes on taking turns, and the sidebar still lists it. It is the one row on the menu
read off the window's own arrangement rather than off the work record, and it is drawn dead when no
chat tab is attached. Close is the delete this menu used to call Delete, renamed for the pair: the
confirm it raises says *Close conversation* and warns that the transcript and the run directory,
seeded credentials included, go with it and that it cannot be undone.

**Info opens the panel the tools section leads with** — `ui/conversation/info.rs`, a modal over the
window. It draws only what the window already holds: the harness label, the model and the permission
mode; the account, where an account that resolved to nothing reads as *the user's own home* rather
than as a blank; the token spend, broken down and split by subagent where the harness reported one,
falling back to `WorkAgent::tokens`; the context percentage and the last usage report; and the
session id with a copy button, because that is the handle every log line elsewhere is keyed by.
Three buttons at the foot reveal a folder in the desktop's own file manager — the run directory
(`WorkAgent::run_dir`), the configuration directory the library resolved (`WorkAgent::config_dir`),
and the folder holding the capture, which is there only while there is a capture. A folder the host
has reported no path for is drawn dead rather than dropped.

**Three toggles sit between Fork and the closing pair, and they are Ubiq's own rather than the
harness's.**
The persistence row (`SetConversationPersistent`) marks the conversation as one that outlives the
window, reading *Make persistent* or *Stop persisting* by which way it goes. The accept-all row
(`SetConversationAcceptAll`) reads *Accept all* or *Stop accepting all*, and while it is on the host
answers every permission the harness asks for and the window is never shown the ask — the chat
document holds what that does to the transcript. The dump row
(`SetConversationDebugDump`) reads *Dump messages* or *Stop dumping* and writes that one
conversation's traffic to a file; the host answers where, and the row carries the path as its
tooltip, because a capture the user cannot find is a capture that did not happen. **Both edges of
the dump row put that path on the clipboard.** Stopping has it in hand — it is on the record now and
will not be in a moment. Starting has nothing yet, so `AppState::dump_copy_pending` records who
asked and `watch_for_dump_path` copies the host's answer when it lands, giving up on its own if none
does. No toast follows it: the notification list is the host's, every row in it a broadcast, and the
row's own tooltip already says the path in the place the click happened. All three toggles sit
before the closing pair, because none is destructive and the irreversible verb stays last — which is
where a toggle added later goes too. **Only the persistence row reads the harness**: it is drawn dead for a
harness that keeps its sessions outside the run directory (`keeps_sessions`), since keeping that
directory would preserve nothing, while the other two are always enabled because Ubiq answers and
Ubiq writes, and no harness has to support either.

**Each item disables rather than disappears when it does not apply** — Stop only while a turn runs, Abort and Unload only
while launched, Resume only while it is not, Fork only while nothing is in flight (copying a session
store mid-append tears the last record), Hide only while a chat tab is attached, Info and Close
always — so the menu's shape never changes under
the cursor. The labels and their enablement are one list of pairs,
`ui::conversation::lifecycle_menu_rows`, because `AppState::pick_conversation_menu` dispatches by
position and a row in one copy of the list and not another is a menu whose rows do the wrong thing.
`ui::conversation::LIFECYCLE_DUMP_ROW` names the one row the menu reaches back into to hang the
capture's path on, for the same reason — and it moves whenever a row or a separator is added above
it, which is the other half of why it is not a literal. See [`sessions-and-workspaces.md`](./sessions-and-workspaces.md) for what unload keeps that
closing does not.

**The bench is computed, not stored.** It is every agent the host reports that no column is showing,
so an agent the host stops reporting stops being listed with nothing to clean up.

**A column's own `+` groups a second agent in, and the list it opens is `AgentsView::bench_rows`** —
one `BenchRow` per row the menu draws, `Agent`, `Label` or `Separator`, matched by position the same
way `HarnessChoice` is above: a heading and its separator are rows like any other, disabled and
unpickable, and `AppState::pick_agent_bench_menu` re-reads the same list before resolving a click so
an index can never name a different agent than the one drawn there — and, once the pick lands,
follows it with `restore_composer_draft` on the column's slot, the way the chat panel's own attach
picker does (`chat.md`), so a draft typed before the agent was benched comes back into the
composer now addressing it. Two groups only, because that is
the one honest split the record supports today: agents free on the bench, and agents already on
screen in some other column — shown, disabled rather than dropped from the list, because a row that
vanished would read as an agent that had ended, and `AgentsView::open_in` already refuses to draw one
twice. Neither group is split further by role or task: `WorkAgent` carries both, but neither is filled
from a real run yet, so a grouping built from them would be drawing real groups over invented values.
An agent definition is a real thing a conversation can start from, and it still does not help here — it
pins a harness, an identity and how the run is set up, and carries no role and no task — so the backlog row
on grouping by role, task or team waits on those fields existing rather than on definitions. The list is
searchable exactly the way every other filter in the window is: a lowercase substring typed into the
shared `picker_search` field, narrowing both groups at once and dropping a heading whole once nothing
under it still matches.

**The screen lays itself out once, and every listing after that only prunes.** The first `WorkList`
gives one column per session that has an agent in it, holding every agent in that session in the
order the host listed them: each column is a piece of work, and a session running several agents
arrives grouped rather than spread across the row. The bench therefore starts empty and fills only
from the user's own closes. Every later `WorkList` or `AgentChanged` drops the tabs naming agents the
host has forgotten, and the columns that empties, and does nothing else — an arrangement the user has
changed is not something an arriving record may undo. An arriving agent is listed on the bench rather
than put in a column.

**A column, or a chat tab, owns a composer for its life.** The window holds a fixed pool of
`COMPOSER_SLOTS` text areas, split into two ranges — `0..COLUMNS_MAX` for columns and
`COLUMNS_MAX..COLUMNS_MAX + CHATS_MAX` for chat tabs, see [`chat.md`](./chat.md) — because one is
built before the first frame and its subscription is held for the window's life. A column or a chat
tab is given a **slot** from its own range when it opens and keeps it, so what was typed at one
agent never moves into a field addressed at another, and a freed slot's draft is cleared because the
slot is handed to the next column or chat tab that opens. The placeholder names the agent the column
is showing. Enter sends, Shift-Enter inserts a newline, and cmd/ctrl+Enter sends too — the same
`secondary-enter` binding a multi-line `submit_on_enter` field already answers the same way as a
bare Enter, so there is nothing extra to wire, only a hint to show for it. `AppState::agent_for_slot`
is what "sends" resolves the agent through on every surface — a chat tab's own attachment for a slot
in the chat range, a column's active tab for one in the column range — so the Enter key and the
composer's own button never disagree about who a slot is addressed at. Up in an *empty* field
brings the last turn back, the way a shell brings back the last command — `recall_last_message`
reads it off the transcript, which is what was actually sent, and keeps nothing beside it. Claude
Code's own cancelled-turn echo (`[Request interrupted by user]`) is not pushed as a transcript block
in the first place — see the chat document — so it is never what Up hands back. A field
with a draft in it is left alone: the key moves the cursor, because a key that overwrites what is
typed is a key that loses work.

**One control does Send, Stop or Enqueue, depending on the turn.** Idle sends, exactly as
`prompt_agent` always has. A turn already running with the draft empty offers Stop, which cancels
it. A turn already running with something typed offers Enqueue instead of writing into a harness
mid-turn: the draft is held on the conversation's own `queued` list and the composer clears, the
same way a send clears it. `AppState::send_or_enqueue` is the one function behind all three — the
button's click and the Enter key both call it — and it is what a queued row's turn ending drains:
`Message::ConversationUpdate`'s handler pops the front of the queue and sends it as a plain
`PromptAgent` the instant `apply` leaves the conversation `Idle`. A queued prompt is drawn as its own
row at the top of the bottom block — above the activity bar, the footer and the composer, because
what is queued is not part of the turn being written — oldest first, each with a send-now (Send
ASAP), an edit and a delete. Send ASAP is the one way out of waiting for that flush: `AppState::
send_queued_message_now` takes the entry back out of the queue and calls `send_prompt` directly, the
same wire call a live turn's own send uses, instead of holding it for `Run::Idle`; nothing about the
turn already in flight is touched, and the harness picks the prompt up at its next step. Edit loads
the row back into the composer and delete drops it outright; the block draws nothing when the queue
is empty. Files attached to the turn are a second such block, drawn directly above the field instead
— it belongs to the turn being written, not to what is waiting — and the chat panel's own document
owns that, including why an attachment lives on the conversation and how it reaches the wire.

**The ceiling is on columns, not on tabs.** Eight columns fit the row. Grouping into a column that
is open always works, however many tabs it holds; a split that would need a ninth is refused and
leaves the tab where it was, because the room is checked before the tab comes off its column. A
click in the sidebar is never refused: with the row full, a benched agent is grouped into the
focused column rather than given one of its own, because "show me this agent" is a request the
screen can honour whatever the row looks like.

**A column draws one thing below its chrome, and it is the chat panel.** Not a panel of the same
shape — the same code, `crates/ubiq/src/ui/conversation/mod.rs`, which the chat tab the Teams and
IDE screens dock and the kitchen sink draw too. A column owns no transcript, no footer and no
composer of its own; it passes a `ConversationView` and stops there. It used to carry a second set
of all three, for an agent that was a record and nothing more, and no column could reach it:
`AgentsView::live` is exactly the conversations this window holds and `prune` drops a tab that is
not in it, so the mock branch was drawing for a state the screen had already ruled out. A column
whose agent has no conversation now says so in one line rather than opening a second transcript.
The conversation view knows
nothing about the screen hosting it: the chat panel and the kitchen sink adopt it by passing a
different `ConversationView` — an id prefix, a composer slot, whether a footer and a composer come
with it — rather than by growing a renderer each, which would drift the frame a tool block gained a
field. A block is markdown, a thinking block, or a tool call whose header carries a verb from the
tool's kind, its target and its status, and which expands onto its output or its diff. Each message
block carries its own copy control in its lower right, hidden until the pointer is over that
message: the clipboard gets the block's own text, and a control drawn on every line at rest would
read as a toolbar rather than a conversation.

**Nothing writes into a transcript.** The composer sends `PromptAgent` and appends nothing itself;
the user's own line appears when the harness
echoes it back, which is what the harness received rather than what was typed at it. A screen that
drew its own half of a conversation would be inventing the other half too.

**The run pill, the activity badge and the context ring are read off the stream** the window holds
rather than asked for, because asking would be a round trip per token.

**A column's footer reports what the turn has spent, and a ring only where there is one.** What the
turn has cost, the context used out of the size the harness reported, and — where Claude Code's
`rate_limit_event` has arrived — how full the rolling five-hour window is; the harness itself is the
composer's identity chip's business, not the footer's. **No ring is drawn when no context window was
reported** — a ratio over an invented denominator reads as a fact and is not one, and `G96` names who
reports none; the rate-limit pill is guarded the same way. It is the chat panel's footer, because
it is the chat panel: nothing about it is the column's own. The mode chip the design shows is not
coming: the mode is one of the composer's own pickers, live for the conversation's whole life, and a
read-only pill beside it would draw the same fact twice.

**Wherever a harness is named in passing, it is one glyph, not its label.** `kit::HARNESS_GLYPH` —
a single placeholder standing in for every harness alike, since none has a real icon yet — replaces
the harness text in the composer's identity chip, the sidebar's secondary line and the chat panel's row.
Only the *choosing* surfaces still spell the label out in full: the new-agent menu's rows and the
settings page's harness list, where the full name is what a reader needs to make the pick. The
conversation's own name — derived host-side from the harness's command, not set by the UI — is
unaffected either way; the glyph only ever stands in for the harness identifier next to it.

**A conversation names itself once its agent has answered the opening prompt.** The host reads that
one exchange, asks the configured provider for a title and a five-word summary, and the title
becomes the conversation's name wherever a name is printed: the column header, each of a grouped
column's tabs, the sidebar row, and the chat panel's own dock tab. The summary is the **hover** on
each of those, which is what lets a row that is one line still say what it is about — a sidebar row
with no summary hovers to its own name in full, since that is what an elided row owes a reader
anyway. It happens **once per conversation**, it needs a provider configured, and a naming that
fails says nothing: the mechanical name is still there, so there is nothing to report and nothing
to undo. The checkbox that switches it off is in application settings' Assistance section below;
`D90` is the decision behind replacing a name nobody typed. A user's own rename — the tab's
right-click Rename, or the same field on an attached chat tab — sends `Message::RenameConversation`
(`tech/transport-contract.md`) and counts as named, so this naming pass never overwrites it.

**The sidebar lists every conversation this window holds, not what is on screen.** That is the point of it: a
column is one conversation and there are only ever a few of them, so the list is the one place a
whole project is visible at once, and a benched agent is in it, marked, rather than gone. A session
is a group with a bar down its left edge, and the bar carries the worst thing happening under it —
error over waiting over running over ended — so a folded session still says it has a failing agent,
the same rule `WorkProjection::pulse` follows for a task's card. Its note line is the title of a task
in that session, read off the work rather than carried on the session, because a session has no
description on the wire. A session with no agents in it is not drawn at all. One click reveals: an
agent in a column comes to the front of it, and a benched one opens a column of its own — or joins
the focused column when the row is already full. The row folds its session, and the header's one
control folds every session or opens every one.

**A Missions section sits above the sessions (M14).** One row per mission not `Completed` or
`Abandoned`, most recently active first — closed ones sort last and draw only once the header's
own *show closed* toggle is on, which itself is offered only where there is a closed mission to
show. A row carries the same hexagon every other mission surface draws, a phase chip, the key and
title, a *needs you* dot while anything is pending, and the roster's size; the chevron alone
expands it to the roster, and the rest of the row opens the mission panel — a session row's own
split between folding and opening, applied a second time. The header carries `New agent` beside
`New mission`, so the first mission in an empty project is made from here without a trip to the
board. **An agent inside a mission keeps its session group and carries a chip there instead of
being moved out** — the same conversation is never listed as two different things — the chip
naming the mission (its key, or its title) in the phase's own colour, read once from every
mission's active roster rather than searched per row. The roster a mission's own row expands to is
narrowed through `AgentsView::live_agents` on this screen's usual rule, so a mission member with no
live conversation here is silent rather than drawn wrong.

## Contract

**A live conversation is a family of its own, and every message in it names an agent.** Going out:
`StartConversation`, `PromptAgent`, `CancelTurn`, `AnswerPermission`, `SetAgentConfig` and
`EndConversation`, with `ListAgentTypes` behind the `New agent` menu. Coming back:
`ConversationStarted`, `ConversationUpdate`, `ConversationEnded` and `ConversationError`. An update
is a delta rather than a record, so the transcript is a fold the window keeps and the host never
re-sends; the family's payloads and its ordering rule belong to the transport contract.

## Implementation

`state/status.rs` is the one place a status *is*, for an agent and a delegate alike: the `Lifecycle`
and `Doing` dictionaries, the `Status` pair over them, and the three derivations —
`conversation_status()` off a live `Conversation`, `delegate_status()` off a `SubagentTab`, and
`agent_status()` which prefers the first and falls back to the host's record. Nothing here draws and
nothing here names a colour. A delegate whose spawning call reached `Completed` is
`Ended · Done` — terminal, so a late permission request does not revive it and no active count
includes it — and one whose call the transcript never held is `Starting · Unknown`, because silence
is not success.

`ui/conversation/mod.rs` is the one place a *conversation's* state becomes a plain **element**:
`lifecycle_pulses()` says which readings move, and `lifecycle_dot()` is the dot itself; the colour
comes through from `ui/work.rs`. It is what the dock's tab strip still draws for a terminal or a
file tab, through `TabInfo::dot_colour`/`dot_pulse` in `ui/dock/skin.rs` — every kind but a chat
one, since T-99/T-102 gave a chat tab, a column's own tab, and the Teams cards the hexagon instead:
`ui::teams::status::status_mark`, over `kit::hex_mark` in `ui/kit/controls.rs`, off
`state::status::agent_status()` (`ui/dock/mod.rs`'s `TabInfo::dot_status`, `ui/agents/column.rs`'s
tab). `ui/agents/column.rs` falls back to `agent_status`'s own record reading for an agent with no
live conversation behind it, rather than to nothing.

`ui/work.rs` is the one place a work state becomes a colour, for every screen that draws one.
`activity_colour()` and `bucket_colour()` put the four buckets on the four status tokens — the three
ways of working share the one that means "moving" — `lifecycle_colour()`, `doing_colour()` and
`status_colour()` do the same for the two status dictionaries, `lifecycle_icon()`, `doing_icon()`
and `status_icon()` are their glyphs, and `role_icon()` and `role_mark()` are the glyph
a role wears. `ubiq_proto::work` keeps the words and `theme.rs` keeps the values, so the columns, the
graph, the board and the status bar cannot disagree about what running looks like.

`state/agents.rs` is the other view over that projection, and holds the arrangement rather than any
record: the columns, which one the sidebar's "here" means, which sessions are folded, what is typed
in each composer by slot, and the tab a drag is carrying. A `Column` is a composer slot, an ordered
set of agent ids and which of them is in front, and `grouped()` is the more-than-one-tab rule the
header counts. `arrange()` is the one-column-per-session layout and runs on the first listing;
`prune()` drops the tabs naming agents the host has forgotten and answers whether anything went, so a
re-sent `WorkList` costs no redraw. `reveal()` is the sidebar's one gesture, `open()` a column of its
own, `open_in()` a group, `bench()` a close, and `split_off()` the drop past the last column, which
asks `free_slot()` for room **before** it takes the tab off the column it was in. `reveal()` falls
back to `open_in()` on the focused column when the row is full, which is why the sidebar's click
never fails. `benched()` is the
difference between what the host reports and what the columns hold, and `on_the_field()`,
`grouped()`, `count()` and `has_room()` are what the header, the status bar and the drop targets
read. `COLUMNS_MAX` and `COLUMN_MIN_WIDTH` live here rather than in `theme.rs`, because how many
conversations fit and how narrow one may get are facts about a conversation. `live` is the set of
agents this window holds a `Conversation` for, written where one comes into being, and
`live_agents()` narrows the projection to it — what every reader on the agents screen goes through.
Nothing in it draws,
nothing in it names a colour, and it is tested without a frame in `crates/ubiq/tests/agents.rs`.

**The New agent form is three modules with the window's usual division of labour.**
`state/new_agent.rs` is `NewAgentForm` and small pure readings of it — `Purpose` (start, or write a
agent definition), `Target` (a harness with its identity, or an agent definition), `OpenList` (which of its pickers — or its MCP
checklist — is down, one at a time, and `has_filter()` for which of them carry the shared filter
field), `toggle_mcp()`, `model_levels()`, `default_mode()`, `preamble()` and `fold_preamble()`, all
tested without a frame. `app/new_agent.rs` is the mutators, and reaches for whichever of the two
forms is up rather than taking a discriminator — two ways to answer one question is how the two
would drift apart again — plus `start_new_agent()`, `send_prompt()` and `take_agent_preamble()`.
`ui/new_agent.rs` draws the modal, and its `body()` is what the settings page's agent definition form draws
too. `WorkbenchState` holds the live form as `new_agent`, the `+` menu as `new_agent_menu`
(`NewAgentMenu`: where it opened, which `NewAgentSurface` asked, and whether the attach stage is
drawn), and the held preambles as `agent_preambles`, one entry per conversation, taken on first use.

`state/conversation.rs` is one live agent's transcript as the window holds it. `Conversation::apply`
folds a delta in — a chunk extends the block its message id names, a change of id starts a new one, a
patch reaches its call through an index rather than a scan — and a second index, keyed on the block
count and which delegate is being viewed, is what `visible_blocks()` hands out as a shared `Arc` and
what answers `has_subagent()` from a set rather than a walk. `markdown()` holds one slot, so the
streaming tail is rendered once rather than copied per frame. `activity()`, `context_pct()`,
`tokens()`, `cost_usd()` and `rate_limit_five_hour_pct()` are what the badge, the ring and the
footer's pills are drawn from; `is_next()` is the gap check. `AppState` holds them per project as `conversations`, kept after the harness ends, and
`refresh_agent_record()` writes the badge, the ring, the token count and the model onto the
`WorkAgent` record, so the sidebar, the graph and a column's header keep one source. It folds a
naming on the same way: a `title` replaces `WorkAgent.name` only when there is one, while
`summary` is written whatever it is — a second naming that answered a title and nothing after it
has to clear the reading the first one left, or the hover would describe the conversation as it
was. `Conversation::name` is what a `ConversationNamed` lands in, beside the `title` a
`ConvUpdate::Title` writes, because the two are the same fact from two sources.
`ui/conversation/mod.rs` draws one — `render()` over a `ConversationView`, then `tool_block()`,
`diff()`, `permission()`, `footer()`, `composer()`, `attachment_tags()` and `queue_list()` —
`prompt_agent()` sends what was typed with every attached path composed into it as an `@path`
mention and appends nothing, `send_or_enqueue()` is what the composer's one button and the Enter key both call
(send when idle, queue on `Conversation` when a turn is already running and there is anything to
send — the composed text, so attachments with nothing typed still count — nothing when there is
not), `steer_column()` resolves the slot's agent through `AppState::agent_for_slot`
and chooses between `send_or_enqueue()` and the `SendToAgent` path a record with no conversation
behind it takes — which no column reaches any more, since a column only holds a live one, and is
kept only because a slot is a window-wide pool, and `send_prompt()` is the one place
`PromptAgent` is built — folding a start's held preamble in front of the first turn and telling the
`Conversation` what to take back off the echo. `crates/ubiq/tests/conversation.rs` covers both.

**A conversation is drawn before its harness exists.** `AppState::start_new_agent()` mints the `AgentId`
itself — the `SessionId` precedent — and the host adopts it, so `ConversationStarted` and the
`Conversation` it creates arrive with no process behind them yet; `Conversation.launched` stays
`false` until the harness's own `Started` update sets it. While it is false, `composer()` draws one
`Picker` chip per config option the host has advertised in `conversation.config` — `model`,
`thinking`, `mode`, in that fixed order, each drawn only when the host actually offered that id (a
"Discovering models…" note when none have arrived yet) — beside Send rather than the footer's
read-only pills. `ui::conversation::config_choices(conversation, config_id, search)` builds each
picker's rows; only the model picker's takes a search string, since thinking and mode are at most a
handful of rows. Which picker (if any) is open lives on `Conversation.open_config` — one `config_id`
at a time, per conversation — rather than the window's single `open_menu`, since several pending
conversations can each have their own picker open at once. A pick sends
`AppState::pick_agent_config(agent_id, config_id, value, ..)`, which also records it on
`Conversation.chosen` keyed by `config_id`, since the host does not echo a `SetAgentConfig` sent
before launch and the picker has nowhere else to read its own highlight from, and closes the
picker. Picking a model makes the host re-send `ConfigOptions` with `thinking` recomputed for it —
`Conversation::apply`'s `ConfigOptions` arm drops any `chosen` entry the fresh options no longer
back, so a thinking level the old model accepted cannot survive a model switch into launch. A pick
whose levels come out identical is answered with nothing at all, so the picker the window is
already showing is the one that stands.
`activity()` reads that pending, never-run state (`Run::Idle` with no `stop_reason` yet) as
`Activity::Thinking` rather than `Activity::Ended`, matching what the host already reports at
registration.

`AppState` carries it as `agents`, the composers as `column_inputs` — a fixed pool of
`COMPOSER_SLOTS` `TextareaState` entities built in the constructor (one per column, plus
`CHAT_SLOT`), each with a subscription that mirrors what is typed onto that slot's draft and steers
the column on a bare Enter — and the sidebar's scroll as `agents_scroll`, its own rather than the
explorer's. `reveal_agent()`, `group_agent_into()`, `bench_agent()`, `select_column_tab()` and
`focus_agent_column()` are the clicks; `start_tab_drag()`, `drop_tab_on()`, `drop_tab_at_end()` and
`settle_tab_drag()` are the drag, the last putting down a tab whose drag ended where no drop handler
sees it. `steer_column()` is the one thing this screen sends through the Enter key, and appends
nothing itself; `close_all_conversations()` is `bench_agent()` for every tab in every column, not
`end_conversation()`. The lifecycle menu's rows resolve by position through
`pick_conversation_menu()` onto `cancel_turn()`, `abort_agent()`, `unload_agent()`,
`resume_agent()`, a dead index for the hairline, `open_conversation_info()`, `fork_conversation()`,
`toggle_conversation_persistent()`, `toggle_conversation_accept_all()`,
`toggle_conversation_debug_dump()`, a second dead index, `hide_conversation_view()` — which finds
the chat tab attached to this agent and closes it, ending nothing — and the confirm
`end_conversation()` fires from. `fill_columns()` gives each composer its placeholder and its draft, drained in
`render` for the reason `fill_task_form()` is: `set_placeholder` and `set_value` both need a window,
and an arriving message, a project switch and a jump from another screen have none. `MenuId::AgentBench`
carries the column its `+` was clicked in, because a row of columns has one each and only one menu
may be open. `open_task_chat()` reveals an agent and switches to Agents mode, and it is the one way
out of a task now that the board's panel no longer offers the graph.
`ui/agents/mod.rs` is the frame — the header strip, the row of columns and the drop strip at the end
— and `column.rs` is one column, from its tab strip to its composer. `sidebar.rs` is the list, and it
is no longer inside that frame: it is `PanelKind::AgentsExplorer`, the window's own left-region
panel, drawn in Agents mode with a project and arranged, resized and put away like every other one.

`sidebar.rs`'s `missions_section` (M14) reads `OpenProject::missions` straight, no view of its own
beyond `AgentsView::mission_expanded`/`is_mission_expanded` (which rows are unfolded) and
`show_closed_missions`, both toggled from the header; `mission_row` and `mission_roster` draw one
row and its expansion, and `roster_of` narrows a `MissionRecord::roster` to the agents this window
holds live, `left_at` excluded, the same shape `AgentsView::live_agents` gives every other reader.
`mission_chips` is the one function behind a session member's chip: a `HashMap<AgentId, (String,
Rgba)>` built once per render from every open mission's roster, read by `agent_row` rather than
searched per agent.

## Failure

| What happens | Result |
|---|---|
| No provider is configured, or a naming fails | Every conversation keeps the mechanical name it was given, and no surface hovers to a summary. Nothing is reported: the Assistance section is where a provider that cannot answer says so |
| Naming is switched off while a conversation is mid-run | It is not named. The host re-reads the setting on every pass rather than caching it, so switching it off switches it off |
| The row of columns is full | A split is refused and the tab stays in the column it was in. A click in the sidebar still brings a benched agent on, grouped into the focused column, and the `+` still groups into the column it belongs to — the ceiling is on columns, not on tabs |
| A benched agent stops being reported | It stops being listed, and there is nothing to clean up: the bench is computed from the work rather than written down |
| An agent arrives after the screen has laid itself out | It is listed on the bench rather than put in a column. Every listing after the first only prunes, because the arrangement is the user's |
| Every agent is on the bench | The field says which control brings one back, rather than an empty row that would read as a project with nothing running |
| A tab is dropped anywhere but a column or the end strip | The next frame puts it down in the column it came from, so it cannot stay stuck to the pointer |
| A conversation's update does not follow the last one | The window reports the gap and applies the update anyway, because half a transcript is worth more than none |
| A harness reports no context window | No ring is drawn, and the footer reports what the turn has spent without one |
| A conversation's harness exits | The transcript stays and the agent takes no further turn. Closing the tab is what takes it off screen |
| The composer is used while a turn is already running | An empty draft offers Stop; a non-empty one is held on `Conversation.queued` instead of being written into the harness mid-turn, and sent automatically the moment the turn ends |

## Related docs

- [`workbench.md`](./workbench.md) — the rail, the dock and the panels this mode is drawn inside
- [`chat.md`](./chat.md) — the chat tabs that draw the same conversation outside this mode
- [`workbench-teams.md`](./workbench-teams.md) — the other kind of screen over the same records
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — what the harness library owns behind a start
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the conversation family on the wire
- [`../backlog.md`](../backlog.md) — what this mode still lacks
