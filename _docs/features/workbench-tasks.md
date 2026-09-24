---
id: feat-workbench-tasks
title: Tasks mode — the board
kind: feature
status: draft
summary: The rail's Tasks mode — a column per status, a card per task, what a drag means, the labels and the filter that narrow it, missions and the children they spawn, the task panel that reports one task whole and edits it a field at a time, and the plan surface a mission raises over the window.
read_when: you are changing the tasks board — its columns, its cards, what a drag means, the task panel, a task's attachments or labels, a mission, or the plan surface and its annotations
updated: 2026-09-24
verified: 2026-09-24
code_anchors: [crates/ubiq/src/state/board.rs, crates/ubiq/src/app/board.rs, crates/ubiq/src/ui/board/mod.rs, crates/ubiq/src/ui/board/detail.rs, crates/ubiq/src/ui/board/form.rs, crates/ubiq/tests/board.rs, crates/ubiq/src/state/work.rs, crates/ubiq-host/src/work/mod.rs, crates/ubiq-host/src/plan/mod.rs, crates/ubiq-host/src/plan/blocks.rs, crates/ubiq-proto/src/blocks.rs, crates/ubiq-proto/src/plan.rs, crates/ubiq-host/src/store/plan.rs, crates/ubiq-host/src/mcp/plan.rs, crates/ubiq/src/app/plan.rs, crates/ubiq/src/state/plan.rs, crates/ubiq/src/state/document.rs, crates/ubiq/src/ui/plan.rs, crates/ubiq/src/ui/document.rs, crates/ubiq/tests/plan.rs, crates/ubiq/src/state/new_mission.rs, crates/ubiq/src/app/new_mission.rs, crates/ubiq/src/ui/new_mission.rs, crates/ubiq/tests/new_mission.rs]
depends_on: [feat-workbench, tech-ui]
review_cycle: monthly
---

# Tasks mode — the board

## Purpose

Tasks is the rail mode that answers what there is to do in a project and where each piece of it
has got to: one column per status, one card per task, and a panel beside them reporting the
selected task whole. It writes the same set of tasks the graph modes draw under their canvas, and
the same set an agent reaches through the Manage Ubiq tasks server.

## Behaviour

**The board and the graph are two views of one set of tasks.** An agent started with Manage Ubiq
tasks or Use ubiq tasks ticked writes that same set: a card it creates, moves or deletes arrives as the
ordinary `TaskCreated` / `TaskChanged` / `TaskDeleted` the board applies. A comment typed in the
task panel is authored `user`; one posted through those servers is authored `agent`. The graph answers "who is doing
what"; the board answers "what is there, and where has it got to" — the same tasks, at the scale of
the project rather than of one session. Nothing is copied between them, and the one set is the
host's, held per project: a task ticked on the board is ticked in the drawer under the graph, because
the two screens are two questions about one set of facts.

**A project's sessions and agents are the host's mocks; its tasks are written down.** The mock is
minted per project and made again at every boot, so nothing an agent says outlives the process. A
task belongs to the project instead: it survives the window that made it and the restart after it.

**A new project's board is empty**, and its file is written anyway. An absent store and an empty one
are different things — the second is a board a user emptied, and it stays empty at the next boot.
Where that is kept and how is `D39`'s, not this document's.

**A column is a stage, and a card only ever changes column or its place in one.** Backlog, ready,
blocked, in progress, in review, done, abandoned: moving a card changes where the work has got to
and nothing else about it. Each column carries its own count and a dot in the token that means what
the stage means — nothing yet, queued, stuck, moving, waiting on a person, over, given up. **A
wheel over a lane moves that lane**, not the board sideways.

**A card is filed, and filed in a place.** Unlike the graph's canvas, the column *is* the drop
target: a label follows the pointer while the card stays where it is, the column under the pointer
lights up, and a bar is drawn in the gap the card would land in — between two cards, or under the
last one. A drag that ends anywhere else changes nothing, and the card is left where it came from.

**The drop names the card it lands in front of, not a position.** The board filters, so counting
cards would count only the ones on screen and land the drop above however many are hidden. Dropping
below the last card, or into an empty column, is the end of that column. Reordering inside one column
is the same act as moving between two, and neither disturbs the order of any other column.

**A column shuts to a strip and a card shuts to its title.** A board is read by ignoring most of it,
so both fold: a shut column keeps its dot, its count and its name written downwards, and still takes
a drop; a folded card keeps its title and the marks a board is scanned by — its key, its kind, its
labels and its priority — and drops the progress and the agent line. Neither is a filter — what is
shut is still counted.

**One field finds work and names it.** The filter searches a task whole — its title, its
description, its key, its kind, its labels and its session — because a card that cannot be found by
something written on it is a card that has been lost. Two prefixes narrow it to one field: `key:`
matches only the key, so a task is found by the id a human says out loud, and `#` matches only the
labels. `New task` seeds the next one from whatever is in that field — so typing to look for a card
that turns out not to exist is already most of making it — and the field clears rather than leaving
the board filtered down to the one card just asked for.

**`New task` writes a card; it does not make one.** The button opens a form in the task panel's own
slot — a title, a description, and Create — and **sends nothing**. A card is created once the form
has a title or a description, and never by the click that opened it: pressing `New task`, looking
away and pressing it again returns to the one form with what was typed still in it, rather than
leaving two cards called `New task` behind. Create reads as a ghost and does nothing until there is
something to save; Enter in the title and `⌘⏎` in the description are the same act. A draft holds
the panel while it is open, so the form and a task's report are never both on screen, and nothing
refills its fields from a record — there is no record behind it. Cancel throws it away, and there
is nothing to unwind, because nothing was sent.

**`New agent`, beside `New task` in the toolbar, does not touch the board at all.** It raises the
New agent form with `NewAgentSurface::Chat` (`T-109`) — the same aim `open_new_agent_direct` makes
in IDE mode, since Tasks is the other rail mode it now treats that way — so the agent it starts
opens as a chat tab in the right dock, beside the board, rather than jumping the window to the
Agents screen. It goes **straight to the form**, the way Teams' own `+ Add agent` does and without
the `+` menu's first stage: that menu's other row attaches an existing conversation to the surface
that raised it, and the board draws no agent to attach one to. No project step either — the board
is one project's work, so there is nothing to choose between. Starting an agent from here answers
"who should work on this" without leaving the board to do it; the task the agent ends up working
still needs its own card.

**`New mission`, beside them both, writes a card and launches its assistant in the same act.** The
dialog asks a title, a description, a `require plan` flag and an assistant, picked from whichever
profiles are marked fit to run one — the empty case reads as a sentence pointing at the profile
editor rather than as an empty picker. Start is refused until a title is typed and an assistant is
chosen. On Start the task is created, promoted to a mission and given its description, and the
chosen profile is launched with `manage-ubiq-tasks` and `ubiq-plan` ticked and an opening turn
carrying the mission's title, id and description — the `require plan` flag adds one more sentence to
that turn, a reminder rather than anything the host enforces. The button is labelled with the
project's own word for a mission, the same reading the board's level chip already gives it.

**A card written as a description names itself.** A draft with a description and no title is
created under a stand-in — the first line of that description, taken as plain text, cut to sixty
characters on a word boundary — and then asks the host's own assistance for a written one through
`SuggestSubject::TaskTitle`, the same facility that names a conversation. The answer arrives as an
ordinary `Suggestion` and is put on the task as an ordinary `UpdateTask`, sanitised to plain text
on the way. The stand-in is the point of the order: assistance that is switched off, unreachable,
slow or answers nothing usable leaves the card named after what the user actually wrote, and no
card is ever created called nothing.

The description and the generated title both arrive after the card does, because a `CreateTask`
carries a title and a session and nothing else, and because the id is the host's to mint. That is
also why `New task` cannot select what it asked for: the task that arrives is the one selected, and
the rest of the draft is sent the moment there is an id to send it to.

**The labels are a `kit::MultiPicker` in the toolbar, and they narrow rather than widen.** One row
per label the project actually uses, its own dot in that label's own colour; ticking two asks for
the cards carrying both — `AND`, not `OR`, unchanged from the row of pills this replaced (`T-142`).
They stack with the field, and the reset that clears both appears only once something is filtered —
the graph toolbar's posture, for the graph toolbar's reason.

**A card carries the worst thing happening in its task, unless it has a colour of its own.** Its
left edge is the state the user would want to be told first: a failed sub-task beats one waiting on
a person, which beats one moving. A swatch picked on the panel overrides that and paints the edge
in that colour instead; clearing it returns the pulse. The line under the title names the agent the
task speaks through — the coordinator of a coordinated task, whoever is holding it now for any other
shape — and clicking that name opens its conversation. A task nobody has started says so, and
counts its sub-tasks instead. A comment count sits at the foot as a balloon and a number, and is
absent when there are none.

**A card draws what it has and nothing for what it has not.** Its key, its kind and its labels sit
across the top with the priority; its shape and its session sit at the foot; and every one of them
is absent when it is unset, taking its space with it. When neither the shape nor the session is set
the foot is not drawn at all, rather than a row of two absences. This is `Priority::Normal`'s rule —
the one value with no word — applied to every optional fact on the record: a board where every card
recites what it does not know says nothing.

**A mission leads the row it sits in.** A task allowed to have children and to carry a plan carries
an accent chip ahead of its key and kind, naming it in the project's own word for the level rather
than a fixed label — the project's override if it set one, else the application-wide default. A
task is a mission or it is not; nothing about the record fixes that at creation, and the panel's
Level control both promotes and demotes it the same way any other field changes.

**A mission's card counts what it has spawned.** A muted chip beside the mission chip names how
many children it has, drawn only where that count is over zero — the same absent-when-unset rule as
every other mark. Depth is capped at one: a task that already has a parent may not itself be given
one, and a task that already has children may not be made a child — together the two rules are what
removes any need to walk the tree for a cycle, and only a mission may be a parent at all. Deleting a
mission orphans its children rather than taking them with it or refusing the delete: each keeps
everything it has and simply loses the parent it named, because cascading would delete work the
user never selected and refusing would leave a board that can never be cleared.

**A mission's panel offers its plan; an ordinary task's does not.** The affordance is drawn only for
a task carrying a `level` — a task with `level: None` has no plan and the panel offers no route to
one. Opening it raises the document over the whole window, three columns: a thin minimap down the
left, the plan as an ordinary markdown preview — no source view, no layout toggle, the page drawn
section by section with a gutter down its right where each section reports its threads — and beside
it the rail of threads, plus an Export action that writes an explicit, one-shot copy into the
project's working tree at a path the user picks, never a continuous mirror. A mission nobody has
planned says so instead of reporting an error, the same way a task with no sub-tasks does.

**The minimap is a miniature of the document's own layout, not a row of identical ticks (T-110).**
The first cut drew one full-width mark per thread and nothing else, which read as noise rather than
orientation once there was more than a couple of threads; the rework follows
`_docs/inbox/markdown-improvement-proposal.md` §8.2's table instead. `kit::minimap`
(`ui/kit/minimap.rs`) stays the reusable primitive — geometry and colour as data, nothing
plan-shaped in it — over three inputs: a `&[MinimapMark]` for the document's own shapes, a
`&[MinimapTick]` for a small coloured mark on the strip's outer edge, and an optional
`MinimapViewport`, the translucent rectangle standing for the visible region. `on_select` answers a
tick's click; `on_scrub` answers a click or a drag anywhere on the strip with the fraction it
landed at, one callback for both because a scrub always recomputes its target from scratch rather
than tracking a drag anchor.

`state::document::minimap_rows` is the pure function behind the shapes: **one mark per block, of
every kind** (T-134) — a paragraph's or a table's own row-per-source-line drawing once read as a
barcode against the reference minimap's look, so each is sized to its widest line instead, `length`
measured in characters against a fixed column-width constant rather than by real glyph width,
because there is no second layout pass to measure by (§8.5) and the preview's markdown renderer
exposes no per-line fragment geometry to place one against instead. A paragraph that is a single
image reference draws as a neutral filled rectangle; there is no image block kind at the host's own
parsing layer to key this off (an image is phrasing inside a paragraph, not a block), so it is a
heuristic over the paragraph's text. **`ui/document.rs` places a row by block index over block
count, not by measured pixels** (T-150). It used to ask `ScrollHandle::bounds_for_item` for a real
span down the strip once there had been a frame to measure, and spread blocks evenly until then;
now that the section list is virtualized the only sections with a measured height are the ones
somebody has scrolled past, so there is no total document height to divide a pixel offset by, and
mixing measured pixels for the rendered sections against an even spread for the rest would put the
two halves of the strip in different coordinate systems and make a mark jump as its block came into
view. Every mark, every thread tick and the viewport rectangle are therefore placed in one space —
block index — which is stable from the frame the document opens in and is the space a scrub is read
back into. What that costs is §8.3's real, 1:1 scale for a short document: the strip always spans
the whole document now, and a mark's height is its share of the block count rather than of the
page.

`state::document::thread_marks` is still the pure function behind the outer-edge ticks, positioning
each open thread by the index of the block it anchors to, the same way `heading_sections` positions
the navigator's rows; an orphaned thread (its block gone from the document) carries no tick, on the
gutter's own rule. Colour is `theme::info` for an open thread and `theme::success` for a resolved
one, the gutter's own pairing. Clicking a tick shows its thread exactly the way the rail's own
**Show** button does; clicking or dragging anywhere else on the strip scrubs the preview's scroll
position towards wherever the pointer lands, the viewport rectangle following it — there is no
separate drag-to-resize-viewport gesture, one scrub formula serves a click and a drag alike. The
strip's own show/hide control sits in the chrome, next to the heading navigator, and is
`UiSettings::md_minimap` (`state/settings.rs`) — one flag shared with the standard viewer's own
heading minimap (above), on by default, remembered on the Ui settings layer like every other
reading habit there. `UiSettings::md_minimap_side` says which edge it docks to, `Left` by default;
set to `Right`, the strip is drawn between the document and the thread rail rather than past it, so
the rail stays the modal's outermost column either way. The modal's own width absorbs
the strip's cost split both ways: `DOC_WIDTH` dropped from `860` to `820`, and the modal grew by the
remainder to fit `MINIMAP_WIDTH` (`72`) alongside it, whichever side it draws on.

**The one thing above the document is the heading navigator** —
`kit::md_navigator` (`ui/kit/md_navigator.rs`), a reusable dropdown built for any markdown surface
with threads on it, not grown inline for the plan alone. The navigator paints above the modal when
the plan editor is modal, using the kit's existing layer convention: an `above_modal` flag selects
`MODAL_MENU_LAYER` instead of `MENU_LAYER` (`crates/ubiq/src/ui/kit/menu.rs`). Its trigger is the
same "N threads open" count the chrome always said; opening it lists every heading in the document,
indented by depth, each beside how many threads its own section carries — open against settled,
counted flat rather than rolled into its parents, because a `##`'s count already includes what is
under it once and a nested rollup would count it again for a second row on screen. Picking one, or
a thread's own **Show** button in the rail, scrolls the preview to that section —
`AppState::plan_preview_list`, indexed the same way the list itself is built.

**The section list is virtualized** (T-150). Every section is drawn as its own `TextView` so that
it can carry its own gutter and its own click target, and an ordinary flex column laid every one of
them out on every frame whether or not it was on screen — measured on a real window, a 400-block
document cost roughly forty times per frame what a ten-block one did, which is the whole of "the
annotation view gets slow". `preview` draws the sections through `gpui::list` against
`AppState::plan_preview_list`, a `ListState` rather than a `ScrollHandle`: only the sections
between the scroll top and the bottom of the viewport plus `PLAN_OVERDRAW` reach the row builder,
and a section off screen contributes its cached height and nothing more. The same measurement after
the change: a 400-block document is about twenty times cheaper per frame, and what is left that
still grows is the **thread rail**, which draws every thread and is not virtualized — that cost
follows the thread count, not the document's length. Two consequences the surface has to keep
honest: the row builder is kept for the life of the `ListState` rather than for one render, so it
reads the document fresh off the window's entity rather than borrowing a frame's `AppState` (the
same shape `ui/board/mod.rs`'s own `render_row` takes, and the reason a section's handlers are
`window.listener_for` rather than `cx.listener`); and a row's own margin is invisible to
`gpui::list`, so any gap between sections has to be padding — which costs nothing here, because the
sections are flush by design.

**The plan is written here, not only read — one section at a time.** Double-clicking a section, or
its gutter's edit button, opens that section alone as raw markdown in a field under the rest of the
page; Confirm splices what was typed back into the document and saves it, and the preview returns.
**When the edited text parses into several blocks, the source block is replaced with all of them.**
The anchor `block_id` stays on the **first** resulting block; the rest get fresh ids
(`parse_section_blocks` + `DocumentEditor::replace_cached_block` in
`crates/ubiq/src/state/document.rs`, called from `confirm_section_edit` in
`crates/ubiq/src/app/plan.rs`). `parse_section_blocks` is a thin wrapper over
`ubiq_proto::blocks::blocks` — the host's own walk — so the cache is split by the rules the
re-index will apply and not by a second copy of them. **Confirming an all-whitespace section removes it** —
`state::document::splice_section` closes the blank-line gap it would otherwise leave — rather than
saving an empty paragraph where it stood.
**The preview does not wait for the host to say the section changed.** Every block the preview
draws comes from the cached index `PlanAnnotations`/`PlanAnnotationsChanged` last filled in, and the
host only restates it — `PlanAnnotationsChanged` — when a save's reindex orphans a thread; an edit
that keeps every anchor, the ordinary case, never earns one. Confirm patches that cache itself with
what was just written (`DocumentEditor::patch_block_text`, or `remove_cached_block` for a section
emptied away), so the section shows its own edit immediately instead of only after the surface is
closed and reopened — the host's own word, whenever it arrives, still settles over it.
The document is still written whole: the buffer behind the surface holds the body, a section edit
replaces its slice of it, and `SavePlan` carries the result — so the host re-indexes from a single
write, an edit that has become several sections simply becomes several sections, and the threads
stay on the section they were about wherever the block matching still recognises it. ⌘S and the
Save button send `SavePlan` with the whole
buffer; the host answers with the document it stored, which is what settles the surface clean. An
edit is never thrown away to win a race: a plan that changed elsewhere — another window's save, an
agent's `write_plan` — while the buffer held an unsaved edit keeps the edit and says the other copy
moved, and Escape over unsaved work asks once before discarding. **Nor does a stale buffer replace
the newer copy without being told to.** The buffer holds the `revision` of the body it was seeded
from; when the host has moved past it, the first Save writes nothing — it names who moved the copy
and which revision, the button becomes *Overwrite*, and the second press sends. Every character
typed survives the question, and a further save landing under it asks again, because one
confirmation covers one revision. **The lock itself is the host's**: every `SavePlan` names the
revision it expects to replace, and a save made against one the plan has moved past is refused with
`PlanConflict` and writes nothing. So a save landing between the question and the confirming press
is refused rather than overwritten, and two windows both confirming an overwrite cannot both win —
the second is told where the plan now stands and asks its user again. What the confirmation decides
is only *which* revision this window is willing to replace: the one it was seeded from, or the newer
one it has been shown. There is no force flag on the wire and a determined user still gets there,
one honest expectation at a time. The `/` menu of markdown the planner types over and over — a
heading, a step, a checklist item, a table — belongs to the document buffer's completion provider,
so with no source view on screen there is nowhere to type it: `G333`.

**The surface still asks which lines a human wrote and which the agent did.** Every body the host
states is followed by a `ListPlanChanges` with no watermark — the whole history, which is what makes
a plan an agent wrote from nothing read as agent lines throughout with the human's edits standing
out against them — and the footer counts what changed beside the saved/unsaved word: lines added,
removed and modified, how many blocks were touched, with a dot for each origin that is actually
there. Counts, not a report: the revision split `PlanChangeStats` also carries stays off the status
line. **The per-line underlines that drew those runs are off screen**, not gone: they are painted
into the document buffer, and the buffer is no longer drawn (`G332`).

**A section is the unit the reader acts on.** Hovering one lifts it and reveals two buttons in its
gutter — start a thread on it, or edit it — and the gutter carries a count of the threads it
already has whether or not the pointer is there, in the open colour while any is still asking and
the resolved colour once they are all settled. Clicking a section claims it for a fresh thread, and
the composer opens in the rail on that block; the rail lists every thread, hides the resolved ones
behind a count, and its Show marks the section a thread is anchored to **and scrolls the preview
there** — the same `plan_preview_scroll` the heading navigator's own rows use. **While composing a
new thread, the other threads are hidden and the rail header shows a `Show all threads` button.**
This is view state only (`thread_focus_override` on `DocumentEditor`), reset on every fresh
composer. The granularity is a block, because a rendered document has no offsets to select a passage
inside one — the anchor is the block, whose id is the host's, so a section the host has not indexed
yet says to save first rather than posting a thread that would be orphaned on arrival.
A thread shows the passage it names, its replies, and one composer to add another —
resolving or reopening it is one button, offered to anyone looking, not gated to whoever opened it.
**A thread that has lost its block is marked, never dropped**: an orphan banner says so, and the
thread keeps its replies and its state exactly as they were, because a passage rewritten out of
existence is not the same fact as an answered question. An agent answers what the user opened
through the `ubiq-plan` MCP server's `list_annotations`, `reply_annotation` and
`resolve_annotation` — there is no tool that opens one, so a thread always starts on this side of
the glass.

**The task panel reports one task whole, and edits it in place.** Where it has got to and how much
it matters share the top line, the first written where a column is named and the second right up
against the other edge, because those are the two questions asked of a card before any other. Under
them the facts that identify it — its key, whether it is a mission, its parent, its kind, its
complexity, who it is assigned to, the issue it stands for, its labels, what it references and what is
attached to it — then
its description, then every sub-task with the agent that has it and where that has got to. Ticking
a sub-task is a change to the task rather than to the view of it; unticking lands on idle, because
nothing here can know what its owner would go back to doing. **A sub-task nobody has picked up says
nothing about its state**: idle is the absence of news, and a list that writes it out once per line
is a list that has to be read to find the one line that is not idle. Under the sub-tasks, every
comment in the order it was left. A comment typed here is authored `user`; one an agent posts
through a tool is authored `agent`.

**The shape and the session are at the foot, and both may be unset.** They describe how the work will
be done rather than what it is, which is the last thing looked at and the first thing not yet
decided — and neither drives anything live yet. A shape nobody chose is nothing, not `DIRECT`: `not
set` is a pill in the row beside the three, the way handing a task to no session at all is a row in
that picker rather than an absence the user has to find the way back to.

**The form edits everything about a task except where it has got to.** Its title, its description,
its priority, its key, its level, its parent, its kind, its complexity, who it is assigned to, its
link, its labels, its references, its colour, its shape and its session, its sub-tasks — added at
the foot of the list, renamed in place, ticked and removed — and a comment left at the foot of that
list. Priority, kind, complexity and shape are rows of pills, which are the report and the control
at once because each has a handful of fixed values; level is a single switch rather than a row,
because a task either is a mission or it is not, and there is no third value to choose between; who
it is assigned to is free text, like a key or a link — there is no roster to pick from; the colour
is a row of swatches behind a `none`; the session and the parent are pickers, because both lists are
as long as the project itself and grow with it — the parent's offers only what the task could
legally be given, a mission with no parent of its own; references are a chip list with a `+` that
opens the same idiom, offering every other task not already held, because a reference is symmetric
and untyped and carries no rule beyond not naming the task itself or one it already holds.

**A task's attachments are stored, and a conversation's are not.** That one difference is the whole
design: a chat's attachment is folded into the prompt as `@path` at send and dies with the draft,
while a task's lives on the record, survives a restart and is read back by an agent that never saw
the interface. So the panel's attachment row draws what the record says and nothing local — every
add and every drop is a message, and the chips come back on the answer. A chip names the file, opens
it on a click, and is drawn in the accent when it points into the knowledge base, because *attached
from the KB* is the one thing about an attachment its file name cannot say. Two controls add one:
`+` raises the file picker, and the clipboard control takes what is on the pasteboard — a copied
file by its path, a screenshot written into `.ubiq/pasted/` first, exactly as a composer's paste
does it.

**The attachment picker is the one that offers two key spaces at once.** A task may point at a
project file or at a knowledge-base document, so the dialog carries the explorer's forest with the
knowledge base beside it as one more root, each row under it already carrying the
`kb:{source}:{path}` address the record stores. The picker itself learns nothing — it is told a path
is a path, the way it is told the explorer's are project-relative and a host browse's are absolute.
It asks for files only, unlike the composer's `+`: a folder here is the way to the documents under
it, and picking one would let the container rows, which name no document, reach a record. **The
chat's own attach picker still offers the project only** — the same dialog, one forest short — which
is a card of its own rather than something this one changed.

**A dead target is struck through, not dropped.** The host never resolves `Attachment::target` —
it stays an opaque string on the record, project-relative path or `kb:{source}:{path}` address
alike — so telling a live target from a dead one is the panel's own question, asked fresh every
time the chip is drawn, against the explorer forest and the knowledge base this window already
holds. A target neither tree currently names reads dead: faint, struck through, and a click opens
nothing rather than an editor tab that would just fail to fill. But "not in the part of the tree
this window has looked at" is a weaker claim than "does not exist" — the explorer lists a folder
only on request and the knowledge base's source list may not have answered yet — so a target below
a folder nobody has expanded, or in a source that has not loaded, draws exactly as a live one does.
False dead is the failure this rules out; a dead target this window has not yet caught is the cost,
and the trade is deliberate (`T-84`).

**A label is named once and offered ever after.** Adding one lists every label the project already
uses before offering to make a new one, because two cards spelled `infra` and `Infra` are two labels
and neither filter finds both. A new label is a name and one of the swatches a project is identified
by — the same sixteen, so the board and the picker read in one vocabulary. The suggestions are
derived from the cards themselves; there is no list of labels kept anywhere, so a label stops
existing when the last card carrying it lets it go.

**The link is a string the user pastes, and nothing more.** Ubiq reads the host out of it to pick a
glyph — Azure DevOps, Jira, GitHub, or a plain link for anything else — and never fetches it, parses
it or keeps anything in step with it. A task standing for an issue somewhere else is a fact worth
writing down long before Ubiq could do anything with it.

**A task's key is the one a human says out loud.** Every task already has an id; nobody can read it.
The key is the user's own — `UBQ-123`, `#4711`, whatever their tracker calls it — and `key:` in the
filter finds a card by it. A task created with none gets `T-<n>` instead, one past the highest
`T-<n>` already in the project, so a card is never without one to say out loud; clearing the field
in the form is still how a task goes back to having none.

**One field is open at a time.** The panel is a report first, and a panel where every field is a
text box has stopped reporting. A field opens on a click and closes on a commit.

**Enter or the ✓ commits, the ✕ discards, and losing focus does neither** — the field stays open.
A blur fires before the click that caused it, so a field that committed on blur could not be
cancelled by the button beside it. Selecting another card discards as well, because the field was
about the card it was typed on.

**An empty title is refused where it was typed and never sent.** It is a slip rather than an
intention — the posture Send takes on an empty draft. A description may be emptied, because clearing
one is a thing to mean. And a value equal to the one the host holds sends nothing at all: the
message set is for acts, and re-asserting a title is not one.

**A status has no control.** The column a card is in is drawn on the panel and not offered there: a
column is a stage, and a card only ever changes column by being moved, so a picker for it would be a
second way to do the one thing the drag is for. A failed sub-task still marks the card `BLOCKED` as
a derived pulse, separate from the blocked column a card is filed into. This is the one place the
panel deliberately stops short of what it reports.

**Delete asks first, and a sub-task's × does not.** A task is the one thing on the panel that cannot
be retyped, so it takes the question the picker's Forget takes: the first click asks, the second
sends, and any other click on the panel withdraws it. A sub-task's title is one line, so its × goes
straight through.

**A description is Markdown, rendered by default**, with one control that swaps it for the source —
because a description is read far more often than written. The preview sits *inside* edit mode
rather than instead of it, so Save is still there and the draft is not lost, and what it renders is
the draft rather than the record: seeing what has just been typed is the point of the control. A
task with no description says so rather than dropping the section, on the rule the status bar and
the explorer's git marks both follow. On a **card** it is one mark saying a description exists and
nothing more — what a card carries is fixed, and a folded card keeps only its title and the marks it
is scanned by.

**Every change is a message, and the card says it is waiting.** Nothing on either screen writes to
the host's records: a field sends, the host answers, and the panel goes on reporting the task the
host last confirmed, so a refusal leaves nothing to unwind. A drop asks the same way, and splices
the card in the projection so a reorder is visible without waiting; the waiting mark stays until
the answer comes, so a slow host does not read as a drag that failed. The mark comes off on any
answer naming that task, the old column included, so a refusal cannot leave a card stuck on its way
somewhere. Why the interface asks rather than writing first is the state ownership rule in
[`../tech/architecture.md`](../tech/architecture.md).

**A refusal ends whatever asked for it.** What the host would not do is said on the panel, in its
own sentence rather than the project picker's, because a task that would not move is not a fact
about the catalogue and has to be said where the user was looking. It also puts the open field away,
takes the waiting mark off, gives up on selecting a `New task` that never arrived — and on the rest
of the draft that was waiting for its id — and withdraws an unanswered delete question, so nothing
is left in a state that cannot resolve. The next thing the
host confirms clears the sentence: a report about a change that did not happen is stale the moment
one does.

**The one way out of a task is the conversation.** `Open …'s chat` switches to Agents and reveals
that agent in a column, because a task the user wants to intervene in is a conversation with an
agent, and the conversation is a column. There was a second button here that pointed the graph at
whoever was doing the task; it went because the graph answers "who is doing what" and a user reading
one task is not asking that — the rail reaches the graph in one click for the user who is.

**A task nobody is on yet offers `Assign to an agent` in its place** (`ui::board::detail::footer`,
`AppState::assign_task_to_agent`, `T-64`). It raises the same New agent modal every other `+` in the
window does — never a second dialog — and overlays three things onto it: the board and feedback
MCPs (`manage-ubiq-tasks`, `ubiq-ask`) preselected onto the checklist, two checkboxes drawn only on
this path (`NewAgentForm::for_task`, `ask_for_feedback`, `plan_mode`), and an opening prompt
composed from the task's key and both checkboxes (`state::new_agent::task_assignment_prompt`).
Ticking *Ask for feedback* tells the agent to ask when it needs to; unticked, to assume as much as
it reasonably can instead. *Plan mode* is a sentence in the prompt only — no `ubiq-plan` server is
ticked for it, since this form attaches no plan tool yet. The start is aimed at the chat surface
(`NewAgentSurface::Chat`), so the agent lands in a `Chat` tab in the right dock next to the task
that named it, the same region the `+` below reaches.

## Contract

**The work crosses the bus as well, and every message names a project.** Going out: `ListWork`,
`CreateTask`, `UpdateTask`, `SetTaskField`, `MoveTask`, `AssignTask`, `DeleteTask`, `AddStep`, `RenameStep`,
`RemoveStep`, `MoveStep`, `ToggleStep`, `AddComment`, `AssignAgent` and `SendToAgent`. Coming back: `WorkList`,
`TaskCreated`, `TaskChanged`, `TaskDeleted`, `AgentChanged` and `WorkError`. A project is open in one
window at a time, so each answer reaches only the window that asked, except a `WorkList` the host
pushes when a loaded `tasks.toml` has changed on disk, which every window hears. The three screens
over the work draw from the same projection of it. What no message carries is the arrangement over
the records — which column an agent's conversation is drawn in, and where a card sits. The full
family, with its payloads and its rules, is
[`../tech/transport-contract.md`](../tech/transport-contract.md).

**A task's plan is a family of its own.** `LoadPlan`, `SavePlan`, `DeletePlan` and `ExportPlan` go
out; `Plan`, `PlanDeleted`, `PlanExported`, `PlanChanged`, `PlanConflict` and `PlanError` come back.
The host refuses every one of them for a task with no `level`. `SavePlan` is the editor's: the
window sends it with the whole buffer and the revision it expects to replace, and the host's `Plan`
in reply is what the surface settles against — or a `PlanConflict`, which names where the plan
actually stands and leaves the buffer untouched.
`PlanChanged` carries no body: a window with that plan open re-asks with `LoadPlan` rather than
being sent content it may not have open.
Annotations are their own sub-family: `ListPlanAnnotations`, `AnnotatePlan`, `ReplyToAnnotation` and
`ResolveAnnotation` go out; `PlanAnnotations` answers the asker with the block index and every
thread, and `PlanAnnotationsChanged` tells every other window to re-ask if it still cares. The full
family is in `tech/transport-contract.md`, with the rest.

## Implementation

The mission term is the same shape again: `HostSettings::mission_term` for the application-wide
default (`String`, defaulting to `"Mission"`) and `ProjectRecord::mission_term: Option<String>` for
a project's override, sent as a `MissionTermChange` (`Inherit` | `Set`) on `Message::UpdateProject`
— the same three-state discipline `IndexChange` uses, and for the same reason: the outer `Option`
says whether anything was said, the inner enum says what. `crate::state::work::mission_term`
resolves the two into the word a window draws; `AppState::mission_term` is the one call site every
other reader goes through. `ui/settings.rs`'s `mission_term_input` and
`app/settings.rs::set_mission_term` carry the application-wide field; `ui/sink/project.rs`'s
`mission_term_row()` draws the project's Default/Custom pill pair, and
`app/projects.rs::set_project_mission_term` sends the change and updates the window's own snapshot
at once, on `set_project_index`'s rule, so a board's mission chips relabel before the host echoes
back. `TaskField::Level(Option<Level>)` carries the task's own axis — `ubiq_proto::work::Level` has
one arm, `Mission` — set from the panel's `form::level_pill()` and drawn first in
`ui/board/mod.rs::task_card()`'s marks row.

`TaskRecord::parent: Option<TaskId>` lives on the child only — the parent carries no list of its
own, and both sides derive it the same way: `WorkProjection::children_of()` and `::child_count()` on
the UI side, a plain scan over `self.loaded` on the host's in `Work::orphan_children`. Both fields
are `#[serde(default)]` and absent when empty (`references` renamed `reference` in the file, one per
line), so a `tasks.toml` written before this slice loads unchanged with no envelope bump.
`TaskField::Parent` and `TaskField::References` carry the edits — the References arm replaces the
whole set, dropping a self-reference and a duplicate, the same posture `Labels` takes. The host is
the one place the one-level rule is enforced: `Work::parent_refusal()`
(`crates/ubiq-host/src/work/mod.rs`) refuses a `Parent` set with a `Message::WorkError` when the
named parent carries no task, has no `Level`, is itself somebody's child, or when the task being
given a parent already has children — the two depth checks between them are what removes any need
for a cycle walk, since a chain three deep would need one of the two ends to be both a parent and a
child, and both are refused. `Work::sanitize_relations()` runs on every load and poll and only drops
a parent or a reference naming no task in the project — no depth or level re-check — so data
hand-edited into something the live rule would refuse still loads rather than being rejected wholesale.

`TaskRecord::attachments: Vec<Attachment>` is a reference and never content — a project-relative
path, or a `kb:{source}:{path}` address, plus an optional label — and is `#[serde(default)]` and
absent when empty (renamed `attachment` in the file), so a `tasks.toml` written before this slice
loads unchanged with no envelope bump, the same as `references`.
`TaskField::Attachments(Vec<Attachment>)` replaces the whole set, trimmed and deduplicated by
target, `Labels`' posture again. The interface side is `app/board.rs::add_task_attachments()` /
`::remove_task_attachment()` / `::open_task_attachment()` — the last is the only place the two forms
part company, a `kb:` address opening through `click_kb_row()` and everything else through
`select_file()` — with `ui/board/form.rs::attachments()` drawing the row.
`app/board.rs::attachment_presence()` is the interface-only liveness check the card asks for: a
`state::explorer::Presence` (`Live` / `Dead` / `Unknown`) from `state::explorer::locate()`, walked
against `ExplorerState::presence()` for a project path or `KbState::presence()`
(`state/kb.rs`) for a `kb:` address — both lazily-listed forests, so `locate()` only answers `Dead`
once the folder that would hold the name has actually been listed, and reads `Unknown` the same as
`Live` otherwise. `open_task_attachment()` refuses a target `attachment_presence()` calls `Dead`
rather than opening a tab or a KB panel for it; `ui/board/form.rs::attachments()` passes the same
answer to `kit::removable_tag()`'s new `struck` flag, which is `kit::tag()`'s too. `T-84`.
`state/file_picker.rs::forest_from_kb()` builds the knowledge-base root and
`PickerOwner::TaskAttachment { task }` carries the card back to the commit, which is the one picker
owner whose answer leaves the window. A pasted picture takes the opposite bet from a composer's:
`app/picker.rs::paste_image_into_task()` sends the `WriteProjectFile` and attaches nothing, and the
`SetTaskField` goes out from `pasted_write_settled()` once the write is answered — a chip in a draft
can be taken back off, a path written into `tasks.toml` outlives the mistake. On the MCP side
`manage-ubiq-tasks`' `create_task` and `update_task` both take `attachments`, so an agent adds one
with the same verb a human does.
`Work::orphan_children()` runs on delete, clearing `parent` on every child and sending each its own
`TaskChanged` so a window's board redraws it without the breadcrumb, rather than the store losing the
link silently underneath it. On the UI side, `WorkProjection::eligible_parents()` and
`::eligible_references()` (`crates/ubiq/src/state/work.rs`) mirror the host's rule exactly so neither
picker ever offers a choice the host would refuse: `form::parent()` draws the parent picker on
`session()`'s idiom, and `form::references()` draws the chip list, `form::reference_picker()` its
`+` menu, wired through `AppState::set_task_parent()`, `::add_task_reference()` and
`::remove_task_reference()` (`crates/ubiq/src/app/board.rs`). The card's child-count chip is
`ui/board/mod.rs::task_card()` reading `WorkProjection::child_count()`.

A plan is stored as one markdown file, `<config root>/projects/<ProjectId>/plans/<TaskId>.md`,
beside `tasks.toml` and `kb.toml` — `crates/ubiq-host/src/store/plan.rs`'s `FilePlanStore`, written
and read whole, atomically, with a corrupt file preserved aside rather than clobbered. The level
check is not the store's: `crates/ubiq-host/src/plan/mod.rs`'s `Plans` holds a `work::Handle`
alongside the store and refuses `load`, `save` and `body` with `Message::PlanError` for a task with
no `level`, the same posture `Work::parent_refusal` takes; `delete` skips that check on purpose,
because `Work::delete` calls it to keep a removed task's plan from being orphaned on disk, and a
task already gone cannot be asked what its `level` was. `crate::plan::Handle` mirrors
`crate::work::Handle`'s own shape — an `Arc<Mutex<Plans>>` clone held by the coordinator and by the
MCP listener's `ubiq-plan` server. The coordinator's `plan_job()` (`crates/ubiq-host/src/coordinator.rs`)
answers `LoadPlan`, `SavePlan` and `DeletePlan` on `work_job()`'s own footing; `export_plan()` reads
the body through `Plans::body()` and writes a copy through `crate::store::plan::export_to()`
wherever `ExportPlan::rel_path` resolves to, the same containment `WriteProjectFile`'s path already
gets.

**The family is keyed by a document, not by a task** (`D161`). Every message in it names a
`ubiq_proto::plan::DocumentHandle` — `Plan { project_id, task_id }`, or
`File { project_id, rel_path }` for an ordinary markdown file in the project's own tree. `plan_job()`
resolves the handle to a `crate::plan::Target` once, against the project's root, and that is the
only place a `rel_path` is contained and checked for being markdown; everything below it reads
`Target`, which answers where the body is and therefore where the sidecar goes
(`Placement::ConfigRoot` for a plan, `Placement::InsideProject` for a file, which creates no
directory and carries the file's mode over). A file document's body is the user's own file, written
in place, and `DeletePlan` refuses one outright — the family that annotates a repository's file is
never the thing that removes it. `Plans` is otherwise unchanged: one block matcher, one orphaning
rule, one conflict arbitration, one provenance layer, for both kinds. On the interface side the surface is **generic over a document**: `crate::state::document`'s
`DocumentEditor` holds a `DocumentHandle`, a `DocumentBody` (`Loading`, `Loaded`, `Failed`), the
text the host last stated, and the flags that make a save mean something (`dirty`, `stale`,
`saving`); `DocumentHandle` is the contract's own type (`crates/ubiq-proto/src/plan.rs`), re-exported from
`state::document`, with a `Plan` and a `File` variant — native on the file viewer's own editor
rather than a web-panel tenant (`D160`) — and `app/plan.rs`'s `DocumentWire` trait is the only
place a handle becomes a message. `state::plan::plan_document()` and `::file_document()` mint the
two; nothing in the interface opens a file document yet. `app/plan.rs::open_plan()`
sends `LoadPlan` through it and `app/wire.rs` folds `Plan`, `PlanDeleted`, `PlanExported`,
`PlanChanged`, `PlanConflict` and `PlanError` back — a `PlanChanged` for the open plan re-sends
`LoadPlan` rather than trusting a body it was not given, and `plan_body_arrived()` reads the answer
against what the buffer holds so an unsaved edit is reported rather than overwritten. A
`PlanConflict` goes through `DocumentEditor::save_refused()`, which leaves the buffer exactly as it
is and puts the surface back where a third party's save would have put it: stale, naming who moved
the copy, and asking the overwrite question again about the revision the host just stated.

**The surface has two frames and one implementation** (T-124). `ui/document.rs::surface()` draws the
notices, the minimap, the document and the thread rail, and knows nothing about which frame it is
in; `DocumentEditor::presentation` is the only thing that differs. `Presentation::Modal` is the plan
editor, which **stays a dialog** — `ui/plan.rs` is now its title, its chrome strip, its footer and
nothing else — and is the only one that takes `Layer::Plan`, so Escape and ⌘S still mean the dialog
while it is up and mean the tab underneath otherwise. `Presentation::Viewer` is a markdown tab in
`ViewLayout::Annotation`, drawn inside the panel with no scrim: the tab's file is the document, as
`state::plan::file_document()`, and `AppState::settle_annotation_document()` keeps the window's one
`DocumentEditor` pointed at whatever the editor's active tab asks for — opening it on the way in,
putting it away on the way out, and standing aside while the dialog is up. A second markdown tab
left in that layout says so rather than drawing another file's threads. Two things are warned about
in the tab and nowhere else: a mode that shows the buffer for editing says that editing the source
can orphan a thread, and a tab holding unsaved edits says the surface is showing the saved file
instead. The header's annotation button carries a dot when the file is annotated and the reader is
elsewhere, on `AppState::has_annotations()`'s own sidecar-presence answer. The heading navigator is
offered in all four positions: the open document's own indexed headings, with their thread counts,
where there is one, and `markdown::heading_marks()` off the buffer otherwise.

The buffer itself is the
window's `plan_editor`, one `EditorState` built in `app/boot.rs` with `ui::editor::SlashCommands`
installed as its completion provider; `settle_plan_editor()` seeds it and repaints its decorations
in `render`, where there is a `Window`, for the reason `attach_arrived_files` does. `ui/document.rs::surface()`
draws the document over `ui::viewer::markdown::render_block()`, section by section — a preview with
no source view and no layout toggle (T-101), so the `buffer()` and `half()` routes that split
Source, Split and Preview are gone — and `open_export_plan_dialog()` raises `FileDialog::ExportPlan`
over it, `SaveAs`'s own route. The
`ubiq-plan` MCP server (`crates/ubiq-host/src/mcp/plan.rs`, catalogued as `mcp::catalogue::UBIQ_PLAN`)
carries `read_plan` and `write_plan` over the same `Plans`, reached through `PlanReach`, which holds
the plan handle and nothing else — not a `work::Handle` of its own, since `Plans` already carries
one.

Annotations hang off the same `Plans`: its block index and its threads live in a sidecar,
`<TaskId>.annotations.json` beside the `.md` for a plan and `<file>.md.annotation.json` beside the
file for a project document (`crates/ubiq-host/src/store/plan.rs`'s `PlanSidecar`, `save_sidecar()`
and `sidecar_beside()`), so the markdown itself stays untouched by anything of Ubiq's, on a stable
block id rather than a quoted-context match or a per-run id (`D159`). The two spellings differ —
singular for the file, plural for the plan — and that is kept rather than migrated (`D161`).
**What a document splits into is `crates/ubiq-proto/src/blocks.rs`, and there is exactly one copy of
it.** The walk — container nodes walked through so a list annotates per item, a table one block, a
block's text its own trimmed source at `ParseOptions::gfm()` — lives in the contract crate because
the host's index and the window's optimistic cache have to split a document identically and neither
half may depend on the other. Two copies of those rules is drift that would show up as the preview
disagreeing with the host's re-index, silently (`T-114`).
`crates/ubiq-host/src/plan/blocks.rs::match_blocks()` is the layer over it, and re-indexes the
document on every save — identical blocks keep their id, an edited block keeps it by word-overlap similarity, and a
block nothing matches is reported in `Matching::vanished` so `Plans::save()` can flag its
annotations `orphaned` rather than drop them. `Plans::annotations()`, `::annotate()`, `::reply_to()`
and `::resolve()` answer `ListPlanAnnotations`, `AnnotatePlan`, `ReplyToAnnotation` and
`ResolveAnnotation` with `Message::PlanAnnotations`, broadcasting `PlanAnnotationsChanged` to every
other window. On the interface side, `crate::state::document::AnnotationsBody` and `ComposerTarget`
track the block index, the threads and which composer (if any) is open. The block ids are the
host's and the offsets are the buffer's, so `state::document::block_ranges()` joins them with a
forward scan in document order and `annotation_range()` narrows a thread to its quote where the
quote is still there — the mapping a section's thread count in `ui/document.rs::gutter()` is counted
over. `app/plan.rs::paint_annotation_marks()` and `::paint_change_marks()` still build their two
`TextDecorationCollection`s over the buffer the same way, but the buffer is never drawn (T-101,
`G332`), so what a reader sees instead is `gutter()`'s own count, in the open colour while any
thread is still asking and the resolved colour once they are all settled (`thread_count()`).
Clicking a section calls `compose_annotation()`, which opens the rail's composer against that
block; double-clicking, or the gutter's edit button, calls `begin_section_edit()` instead, and
`section_editor()` is the raw-markdown field that opens in the section's place —
`confirm_section_edit()` splices what was typed back into the buffer and saves it, and
`cancel_section_edit()` drops the edit. `::compose_reply()` and `::set_annotation_resolved()` send
the rail's other two mutating messages. The `ubiq-plan` MCP server adds
`list_annotations`, `reply_annotation` and `resolve_annotation`
(`crates/ubiq-host/src/mcp/plan.rs`) over the same `Plans` — no tool opens an annotation; an agent
only answers or closes one a window already started.

The new-mission dialog is three modules on the New agent form's own division: `state/new_mission.rs`'s
`NewMissionForm` holds what was typed and `ready()`, plus `assistants()` — profiles filtered to
`ProfileInfo::mission_assistant == Some(true)` — and `mission_briefing()`, the one place the opening
turn's text is built, so the reminder `require_plan` adds is cheap to change if it turns out to be
more than that. `assistants()` itself is scope-blind; `ui/new_mission.rs` is what hands it
`SettingsState::profiles_in(app.project(cx))` rather than the global list alone, so a profile scoped
to the dialog's own project is offered beside the global ones, and `app/new_mission.rs`'s
`settle_new_mission()` resolves the picked id through the same `profiles_in(project_id)` at launch
time. `ui/new_mission.rs` draws it, titled with `AppState::mission_term`. `app/new_mission.rs`
carries the mutators and `start_new_mission()`, which composes the launch out of messages that already
exist rather than adding one: a `CreateTask` first, and the rest — `SetTaskField(Level::Mission)`, the
description's `UpdateTask`, `StartConversation` and the briefing's `PromptAgent` — waits on the id
`TaskCreated` answers with, parked on `BoardState::pending_mission` the way `PendingTask` already
parks an ordinary draft, and run once by `settle_new_mission()` from `TaskCreated`'s arm. No wait is
needed between starting the conversation and prompting it: `StartConversation.agent_id` is minted by
the window (`AgentId::generate()`, `start_new_agent`'s own convention), so `PromptAgent` can name it
immediately, and the coordinator's `launch_pending` already launches a pending conversation on its
first prompt (`crates/ubiq-host/src/coordinator.rs`) — the host's own supported path, not a race.
`Layer::NewMission` raises the dialog over the board.

`state/board.rs` is the board's view of the same projection, and holds nothing that is a fact about a
task: the filter text, which session's pills are on, which task is open, which columns and cards are
shut, `opened` — the columns held open against a project setting that would shut them, `shut`'s
counterpart, since a lane that shuts itself when empty has nothing in `shut` for a click to remove —
whether the open task draws in the docked side panel or a centred modal (`popup`,
`toggle_popup()`), the carry, `suppress_popup` — set the instant a carry starts and left alone
through the drop that ends it, so the popup does not pop open over whatever a drag just filed, and
cleared only by a click with no drag behind it — and what the panel is in the middle of doing. `set_column(status,
shut)` puts one column into whichever of `shut`/`opened` the caller names, clearing it from the
other, rather than one method flipping a single list blind. Both `shut` and `popup`
survive a restart, the way the explorer's expanded folders do — carried in `ViewPrefs::board_shut`
and `board_popup`, gathered by `AppState::remember()` and put back once by `restore_files()`, since
neither is a fact the host reports back; `opened` does not, since it only ever answers a setting the
project record already carries. `Field` names the one field open — the
title, the description, a step by its id rather than its place in the list, or the field that names
the next one — and `TaskForm` is what was typed into them. `moving` is a drop the host has not
answered, read back by `is_moving()`; `awaiting_new` is a `CreateTask` whose id is not known;
`preview` is the description showing as markdown while it is written; `confirm_delete` is a delete
asked once. `is_editing()`, `edit()` and `stop_editing()` are the one-field-at-a-time rule, `select()`
discards an open field because it was about the card being left, and `needs_fill()` answers whether
the fields still describe the open task — a pure predicate rather than the refill itself, because
writing into the component library's state needs a window and this has to be testable without one.
`column()` is what one column draws — the tasks a lane holds, filtered — and `matches()` is the same
filter the status bar's counts go through; `text_matches()` is `matches()`'s free-text half pulled
out on its own, over a task's title, description, key, kind, labels, its steps' titles and its
comments' text, because the reference picker's own search (`ui::board::form::reference_picker`)
needs the identical rule and not a second one that drifts from it. `lane_list()` is one
`gpui::ListState` per lane, made the first time that lane draws and kept in
`lane_lists: RefCell<Vec<(Status, ListState)>>` across renders — a `RefCell` because a lane is drawn
from `&BoardState`, and rebuilding the list state on every frame would throw its row-height cache
away. `end_carry()` answers the task and the column it landed in. It is tested without a frame
in `crates/ubiq/tests/board.rs`.

`ui/board/mod.rs::column()` flattens a lane's drop-gap marker, its cards and its end-of-column drop
zone into one `Row` enum (`Marker`, `Card(TaskId, Option<TaskId>)`, `Tail`) — the `Flat` pattern
`ui/git/changes.rs` uses — so `gpui::list`, drawn over `BoardState::lane_list()`'s state, can ask
for a row by index with no separate idea of where the gap or the tail sits. `gpui::list` over
`uniform_list`, because a task card is variable-height by design. Only the rows between the
viewport and its overdraw are ever built; before this a lane built an `AnyElement` for every one of
its cards on every render, which is what made a lane past roughly eighty cards the one thing that
made the whole board stop feeling responsive. `render_row()` is the one row `gpui::list` asked for,
kept for the lane's lifetime rather than one frame — everything it draws is looked up fresh off the
view rather than borrowed from a frame's `AppState`, which is why `task_card()`, `shape_line()`,
`link_chip()`, `now_line()` and `column_tail()` all take `window` and `view` rather than reading
`cx.listener` the way the rest of the screen does. The end-of-column drop zone's `flex_1` — which
filled whatever space was left in the old, non-virtualized column — is inert inside a fixed-stack
`gpui::list`, so `column_tail()` draws it as a fixed 40px strip instead.

`AppState` carries it as `board`, the filter as `task_filter`, and the panel's fields as
`task_title_input`, `task_description_input`, `step_title_input`, `new_step_input` and
`task_reference_query` — the reference picker's own search field, mirrored into
`BoardState::form.reference_query` by a subscription in `boot.rs` the same way the others mirror
theirs, and cleared by `toggle_reference_picker()` every time the picker opens fresh so a search
left over from the last task never narrows this one.
`select_task()` — the plain click, and `navigate()`'s `View::Tasks` arm — clears `suppress_popup`
and, in non-popup mode, queues `PanelEdit::Reveal(PanelKind::Task)` so the docked panel comes
forward the way `reveal_search()` and the other `reveal_*` calls do; `start_task_carry()` selects
the lifted card too but sets `suppress_popup` instead, since a lift is not the click that opens
anything. `new_task()` queues the same reveal in non-popup mode, whether it is opening a fresh draft
or just bringing the keyboard back to one already open, and `toggle_board_popup()` clears
`suppress_popup` on its way through, so switching the shape to popup is never swallowed by a
suppression a drag left behind. Every edit is
a handler that sends and waits: `begin_task_edit()` opens a field and gives it the keyboard,
`cancel_task_edit()` puts it away and refills it from the record, `commit_task_title()` and
`commit_step_title()` refuse an empty title and send nothing when the value has not changed,
`commit_task_description()` allows an empty one, `commit_task_assigned()` follows the key and link
fields' own rule for an empty value, `set_task_priority()`, `set_task_shape()`,
`set_task_complexity()`, `set_task_level()` and `set_task_session()` send on the click, `add_task_step()` keeps its field so several can be typed in
a row, `remove_task_step()` goes straight through, `delete_task()` asks the first time and sends the
second, `withdraw_task_delete()` takes the question back, and `toggle_description_preview()` swaps
the markdown for the source. `new_task()` is where the filter field becomes a title and the task is
asked for; `drop_task()` is the column's own drop handler, because the column is the drop target
here; `settle_board()`, beside `settle_graph()` in `render`, puts down a carry whose drag ended
outside every column. `ui/board/mod.rs` is the toolbar, the columns and the cards, and its
`status_colour()` is the one place a column becomes a colour. `ui/board/mod.rs::columns()` filters
`Status::all()` down to what `AppState::lane_drawn()` says the project draws before building a
single column, so a hidden lane is never in the tree at all. `app/board.rs::lane_pref()` is the one
place that reads a `LanePref` off the open project's `ProjectSnapshot` — `LanePref::plain` for the
sink's fixture board and a folder outside the catalogue — `lane_drawn()` and `lane_shut()` answer
the column's two questions off it, and `toggle_lane_hidden()`/`toggle_lane_collapse()` are the
settings page's two switches, both routed through `edit_lane()`, which rebuilds the whole `lanes`
list with one entry changed and sends it through `set_project_lanes()` — the list travels whole, the
way `search_excludes` does. `toggle_board_column()` reads `lane_shut()` for the column's current
state and calls `BoardState::set_column()` with its opposite, rather than inverting `shut` the way
it used to: that is what makes a lane the project shuts itself openable by a click.

`ui/board/mod.rs::render()` guards its drag-vs-popup ambiguity with `!board.suppress_popup`: a card
drag lifts the same task a click would open, and in popup mode a drop ending a drag over a column
used to pop the detail modal open under the pointer the instant the carry cleared — `suppress_popup`
outlives the carry through that drop, so the popup branch stays shut until a click with no drag
behind it puts it back. A draft in popup mode never takes the docked panel's slot either — `form::draft`
splits into `draft_body()` and `draft_footer()`, shared by the docked panel and by
`form::draft_popup()`, which wraps the same two in `kit::modal_sized` the way `detail::popup()`
wraps the report and controls, and Escape closes it through the same `cancel_new_task()` the side
panel's Cancel button calls.

**The task is a dock panel, in the window's right region.** `PanelKind::Task` is its kind, homed
right, drawn in Tasks mode with a project, and `ui/board/mod.rs::panel()` is its body: the form while
a draft is open, the report while a task is selected, and an empty page otherwise — one slot, because
a draft makes `open_task()` answer nothing. `popup` is the shape toggle over the same bodies rather
than a second copy of the task: with it on, both draw as a modal over the columns and the docked
panel says where they went, so the toggle back is always in reach. A first visit to the mode opens
the right region onto it, through `AppState::queue_mode_furniture()` and
`prefs::ModeLayout::default_for`, exactly the way Git's and KB's furniture arrives. A saved blob from
before this panel existed is not a first visit, so it gets no injection either: the right region it
left open and empty is closed instead (`D156`), and the titlebar's right-hand switch is what puts
the task there.

`ui/board/detail.rs` is the report and `ui/board/form.rs` the controls, drawn into the same panel.
The form is not an area of its own and has no row in the table above: the rule about adding an area
is about something that occupies new space, and this fills the panel that has a row and a
`TASK_PANEL_WIDTH` of its own. It is a second file for the reason `ui/chat/sidebar.rs` sits apart
from `ui/chat/mod.rs` — the report and the controls are two jobs, the same way a tab's head and its
frame are. `title()` and `description()` are
the two fields that open, `pills()` is priority and shape, `session()` is the picker behind
`MenuId::TaskSession`, `step_controls()`, `step_field()` and `new_step()` belong to the sub-task
list, `delete()` is the two-click question, and `refusal()` is where `WorkbenchState::work_error` is
said. The description's textarea answers `SubmitSearch` (⌘⏎, ⌃⏎ off macOS) by calling
`commit_task_description()` — the same "confirm this form from inside a field" device
`ui::new_agent::confirmable()` uses, so bare Enter stays a newline. `references()`'s `+` opens
`reference_picker()` as a `kit::popover` anchored to the `+` itself, with a `kit::filter_bar` over
`task_reference_query` gating a vertical, scrollable result list — empty until something is typed,
capped at fifty rows and at half the window's height, tracked by `AppState::task_reference_scroll`
— in place of the wrapped chips this control drew before `T-133`; the popover's own
`.snap_to_window_with_margin(px(8.))` is what keeps the popover itself inside the window in the
popup shape or a narrow dock, where the panel's own edges would otherwise cut it off. `detail::popup()` is `render()`'s
report and controls again, wrapped in `kit::modal_sized` instead of the panel's own chrome — what
`ui/board/mod.rs::render()` draws over the columns while `BoardState::popup` is on; both
read the same `selected`/`editing`, so the toggle only moves where the task is drawn.

## Failure

| What happens | Result |
|---|---|
| A move is never answered | The card stays in the column it came from, drawn muted and saying it is waiting. Nothing times it out, and the mark comes off on the next answer naming that task |
| An edit is never answered | The field closes and the panel goes on reporting the task the host last confirmed, so the change reads as not having happened. What was typed stays in the form until the selection changes or the project is entered again |
| The host refuses a change to the work | The panel says what it would not do, puts the open field away, takes the waiting mark off, gives up on a `New task` that never arrived and withdraws a pending delete. The next thing the host confirms clears the sentence |
| The selected task is absent from a fresh listing | The panel closes rather than reporting a task nobody holds. The selection is left as it was, so the panel returns if a later listing carries the task again |
| A project's tasks cannot be written | The change holds for the session and one refusal says once that it is not durable. The card moves, so the board and the store disagree until a write succeeds |
| A plan is asked for or saved against a task with no `level` | The host refuses with `PlanError`; the modal is never offered for such a task in the first place |
| A mission has never had a plan written | The modal reports no plan rather than an error, the same posture an untasked mission's sub-task list takes |
| Exporting a plan fails, or the destination cannot be written | The modal's banner names the reason, in the same place a successful export reports its path |
| A block a save's matching could not carry forward | Its annotations are flagged `orphaned` rather than deleted, and the thread panel shows the banner instead of silently losing the comment |
| An annotation names a block the index no longer has | The host refuses with `PlanError` rather than creating one already orphaned |

## Related docs

- [`workbench.md`](./workbench.md) — the rail, the dock and the panels this mode is drawn inside
- [`workbench-teams.md`](./workbench-teams.md) — the graph's tasks drawer over the same set
- [`workbench-agents.md`](./workbench-agents.md) — the New agent form a task assignment raises
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the work and plan families on the wire
- [`../tech/decisions.md`](../tech/decisions.md) — `D157` through `D161`, the planning flow's own choices
- [`../backlog.md`](../backlog.md) — what this mode still lacks

## Next steps

- Reorder a task's sub-tasks, which `MoveStep` names on the wire for exactly that.
- Hand a sub-task to an agent, so `Step.owner` is set by something.
- Remember which of the board's columns were shut.
- Reach a status change from the keyboard, so a card can move without a drag.
