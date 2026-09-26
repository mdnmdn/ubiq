---
id: wip-in-place-help
title: In-place help, and the identity layer under it
kind: wip
status: draft
summary: An inspector-style help mode — point at any part of the window and read what it is — and the element identity scheme underneath it, which exists to serve personalisation and extensions as much as help. Phases 1 and 2 are built — the `UiId` grammar, a static catalogue of names with a label and a sentence each, a `.ui_id()` element extension feeding a per-frame bounds registry, the deepest-hit lookup, and the targeting mode itself — ⇧F1, a full-window layer, a highlight and a balloon, closed by Escape or its own Done. Phase 3a — marking the rail and the titlebar, every name the catalogue currently holds — is built too, proven by headless render tests against the real elements. The help pages behind the sentences (phase 3b) are still designed here and not written.
read_when: you are marking up a screen area with a `UiId`, working on the in-place help overlay, or keying anything durable to a place on the screen
updated: 2026-09-20
verified: 2026-09-26
code_anchors: [crates/ubiq/src/state/ui_id.rs, crates/ubiq/src/ui/ident.rs, crates/ubiq/src/ui/help_target.rs, crates/ubiq/src/ui/rail.rs, crates/ubiq/src/ui/titlebar.rs, crates/ubiq/tests/ui_id.rs]
depends_on: [wip-help, feat-workbench, tech-ui, inbox-ui-id]
---

# In-place help, and the identity layer under it

**This is scratch.** `wip/` is the current task's working notes and is deleted when the task closes.
Nothing here is the durable home of anything: the identity grammar and its stability contract belong
in [`tech/ui-and-design.md`](../tech/ui-and-design.md), the targeting mode as a user-visible
capability belongs in [`features/workbench.md`](../features/workbench.md) beside the other overlays,
and the help binding belongs in [`help.md`](./help.md)'s §5. Those amendments are owed when phase 3
lands and the feature is something a user can switch on; until then this document is where the three
phases are kept together so they compose.

---

## 1. The proposal

Chrome's element inspector is the model. A user presses a key or a titlebar control; the interface
stops responding normally; whatever the cursor is over is drawn with a translucent bordered rectangle
and a balloon saying what it is and what it does. Moving the mouse moves the highlight. Nothing
clicks through. The mode persists until it is explicitly closed.

That is a different shape from the help Ubiq already has. [`help.md`](./help.md) describes a panel
that answers *where am I* — a five-rung ladder from the focused panel, the destination and the rail
mode to a page. In-place help answers *what is that* — the user is pointing at a specific thing and
wants a sentence about it, not a manual page about the screen it lives on. The two want the same
content library and completely different amounts of it.

**"Marked" means the element declared a name.** An element that calls `.ui_id(ui_id::RAIL)`
publishes both that it is a target and where it was drawn this frame. An unmarked element is
invisible to the mode: the highlight snaps to the nearest marked ancestor, and an area nobody has
marked yet reads as the window. Marking is opt-in and incremental, which is what makes phase 3 a
series of small changes rather than one sweep.

### Why identity is the deliverable, not the overlay

Three features want the same answer to *which place is that*, and only one of them is help.

- **Help** binds a sentence, or a page, to a place.
- **Personalisation** keys a user's preference to a place — hide this control, narrow this column,
  set this area's density. It is read at startup by a build that has never seen the file that names
  it.
- **Extensions** anchor a contribution to a place they do not own — a badge here, a row after that.
  They resolve late, against a catalogue that may not hold what they name.

The overlay is throwaway; the names are not. A name a personalisation file has written down is
public API from that moment, and the cost of getting it wrong is paid by users on upgrade rather
than by the author in review. So the identity scheme is specified first, and specified to outlive
the feature that motivated it.

**There is no existing registry to extend.** The roughly 355 kebab-case strings under
`crates/ubiq/src/ui/` exist to satisfy GPUI's `ElementId` for hover, scroll and focus bookkeeping.
They are not unique across the tree, carry no hierarchy, and mean nothing outside the frame that
built them. `crates/ubiq/src/ui/rail.rs` — the most recognisable area in the product — has almost
none. Repurposing them would make identity a rendering artifact, which is the one thing it must not
be.

## 2. The identity scheme

### 2.1 Grammar

```text
ui-id   := segment ( "." segment )*      1..=8 segments, <= 96 characters
segment := [a-z] ( [a-z0-9] | "-" )*     must not end in "-", must not contain "--"
```

`crates/ubiq/src/state/ui_id.rs`, `UiId` and `validate()`. Lowercase kebab inside a level, a dot
between levels: `rail`, `rail.mode.tasks`, `titlebar.new-terminal-menu`. The depth and length limits
exist so that "how deep can this go" has an answer; nothing in the catalogue is near either.

**The dots are the hierarchy, and nothing else is.** `parent()`, `ancestors()`, `depth()` and
`is_ancestor_of()` are string operations on the name. Ancestry is compared level by level, so `rail`
contains `rail.mode` and does not contain `railing.mode`. An intermediate level need not itself be
drawn — `rail.mode` names a level in the scheme whether or not any element marks it.

A `UiId` built from a literal is `const`, so the catalogue is a table of constants and a call site
names one without allocating. A `UiId` parsed at runtime — from help content, a preference file, an
extension manifest — owns its string and is checked against the grammar. The two forms of the same
name are the same id.

### 2.2 Naming rules

- **The path says what a thing *is* in the product's structure, not which container currently draws
  it.** This is the rule that makes readable paths survivable and the one that is easiest to get
  wrong. Moving the search field from the titlebar into the ribbon is a layout change; it is not a
  rename, and `titlebar.search` keeps its name. A path that has to change every time the layout does
  is a layout description wearing an id's clothes.
- **Every name has a root that is itself a name.** `rail`, `titlebar` — the area a reader can be
  shown. `catalogue_is_well_formed()` checks it, which is what lets an overlay widen a highlight from
  a control to the area around it and always land on something named.
- **Granularity: every area, every menu, and elements on demand.** An area persists and contains —
  the rail, the titlebar, a rail mode's body, a panel — and is the unit personalisation usually wants
  and the unit a paragraph of help usually explains. An element earns a name when help,
  personalisation or an extension asks for one. Aiming at total coverage is how a lint gets
  suppressed.
- **Repetition is not part of a name.** A row per file, a column per agent, a tab per pane: the
  *kind* of place gets one name, and an instance is addressed separately. The set of names is bounded
  by the code, never by the user's data. The scheme has no instance form yet — `G313`.

### 2.3 Who owns a name, and what happens when it moves

- **The change that renames the element owns the rename.** Not a tidying pass. A rename with no
  behaviour change behind it is a breaking change for no benefit.
- **A name in the catalogue is a published contract.** Once a help page, a preference or an extension
  manifest can name it, it is API.
- **Renaming should be adding, never editing** — a new name plus an alias from the old one, resolved
  one hop, exactly as a help page's `redirects` keeps a pasted link working. The catalogue built in
  phase 1 has no alias column and cannot express this; that is `G312`, and it is the gap that will
  hurt, because a stale help key fails at build time while a stale personalisation key fails on a
  user's machine.
- **Deletion is deletion.** The name goes, the packer fails any page still claiming it, and a stored
  preference keyed on it is dropped at load with one log line — never a dialog, because a user who
  upgraded is not the right person to ask about a removed button.

### 2.4 The catalogue

`ui_id::CATALOGUE` in `crates/ubiq/src/state/ui_id.rs`: a `const` slice of every name the interface
knows, with a named constant per entry. It is the answer to *is that a real target?* — for
`_tools/helpbundle.py` checking a `ui.*` context key, for a preference naming a control, for
anything listing the places a user can be pointed at. `is_known()` is the check;
`catalogue_is_well_formed()` and `crates/ubiq/tests/ui_id.rs` are what keep the literals honest,
since a `const fn` cannot validate its own argument.

It covers the two areas phase 3a marks: the rail, its mark, its mode column, its project badges and
one name per `RailMode`; and the titlebar, its project cluster, its command field and each of the
fourteen actions in its right cluster.

`rail_mode()` maps a `RailMode` to its name through an **exhaustive match written out by hand**,
not through `RailMode::slug()`. Deriving it would mean that renaming the Rust variant silently moves
the place a help page and a user's personalisation both point at — precisely what §2.3 forbids. The
match being exhaustive means a *new* mode is a compile error until it is named, which is the check
worth having. The one visible consequence: `RailMode::TeamsOld` binds help at `rail.teamsold`
through the existing ladder and is `rail.mode.teams-old` here, because the two are different
namespaces answering different questions.

### 2.5 Binding content — half decided

**Phase 2 took the second option, for the balloon.** The sentence lives beside the name:
`ui_id::TargetInfo` is a label and a one-sentence blurb, `ui_id::describe(&UiId)` is the only way to
one, and a `const` table beside the catalogue holds one for every name phase 1 wrote down. The
overlay calls `describe` and never touches that table, which is what makes replacing it with a
`ui-targets.toml` and a packer a change to `state/ui_id.rs` and to nothing else.

The rest of the row below is still open and is `G314`: a help *page* bound to a name — the "read
more" the second option puts on a `targets:` field — does not exist, and `_tools/helpbundle.py`
still does not know the `ui.` namespace. The first option is ruled out for the balloon on the
grounds the table gives: one sentence per control through a ladder that must resolve to exactly one
nav-reachable page is hundreds of stub pages.

What was settled from the start is the spelling: a name
is written down outside the crate as `ui.` plus the path — `ui.rail.mode.tasks` — which is what
`UiId::qualified()` and `UiId::from_qualified()` produce and read, and it is the same string under
either option. Nothing in the tree consumes it yet, and `_tools/helpbundle.py` does not know the
`ui.` namespace.

| Option | Shape | Costs |
|---|---|---|
| **The existing `context:` field**, under a `ui.*` namespace | Reuses the whole pipeline §5 of the help document describes — the packer's validation, `catalog.json`'s `context{}` map, `crates/ubiq/src/app/help.rs` — and needs no new machinery and no `crates/ubiq-proto` change | `context` is one rung of a five-rung ladder that must resolve to exactly one page, which is why the packer's exactly-once rule exists; and every bound key needs a page reachable from `nav.order`. Binding hundreds of controls that way floods the nav tree with stub pages whose only reader is a balloon |
| **A blurb in the target catalogue, plus a many-to-many `targets:` field on help pages** | The sentence lives beside the name, written by whoever named the element, present for every name; the page field is the optional "read more", with no exclusivity and no nav obligation. `context:` keeps its job unchanged | The catalogue stops being a `const` list in Rust and becomes a data file with a packer and a lint behind it — `G315`. Two bindings to understand instead of one |

The second is what [`inbox/ui-id-proposal.md`](../inbox/ui-id-proposal.md) §5 argues for, on the
grounds that what an inspector shows over a button is one sentence and a manual page is the wrong
unit for it — and it is what phase 2 built, for the balloon's half. The page's half is phase 3b's.

Whichever wins, one constraint holds: **no `crates/ubiq-proto` change.** The help catalog already
reaches the interface, and a content binding that needed a new message would be a crate-boundary
design of its own.

## 3. The phases

**Phase 1 — identity. Built.**

| Piece | Where |
|---|---|
| `UiId`, the grammar, the hierarchy, the context-key conversion | `crates/ubiq/src/state/ui_id.rs` |
| The static catalogue and `rail_mode()` | same file |
| `MarkRect`, `UiMark`, `MarkRegistry`, `deepest_hit()` | same file |
| `.ui_id(...)`, the per-window registry, `hit_at()` | `crates/ubiq/src/ui/ident.rs` |
| `begin_frame()`, called once per frame | `crates/ubiq/src/ui/shell.rs`, `render()` |
| Tests for the grammar, the hierarchy, the catalogue and the hit-test | `crates/ubiq/tests/ui_id.rs` |

**Phase 2 — the targeting mode and the overlay. Built.**

| Piece | Where |
|---|---|
| `TargetInfo`, the `const` description table, `describe()` | `crates/ubiq/src/state/ui_id.rs` |
| `HelpTargeting` — the mode, and the cursor while it is up | `crates/ubiq/src/state/workbench.rs` |
| `Layer::HelpTarget`, the top rung | `crates/ubiq/src/state/overlay.rs` |
| `open_help_target`, `close_help_target`, `move_help_target` | `crates/ubiq/src/app/help.rs` |
| The rung in `top_layer`, and the first arm of `cancel_dialog` | `crates/ubiq/src/app/shell.rs` |
| `PointAtSomething`, bound to ⇧F1 in `Workbench` and in `Input` | `crates/ubiq/src/app/mod.rs` |
| `OverflowRow::PointAtSomething` — the menu row | `crates/ubiq/src/state/workbench.rs`, `crates/ubiq/src/ui/overflow_menu.rs` |
| The catcher, the highlight, the balloon and its placement, the Done bar | `crates/ubiq/src/ui/help_target.rs` |
| `hit_mark_at` — the lookup with the rectangle attached | `crates/ubiq/src/ui/ident.rs` |
| The balloon's width, its assumed room and its gap | `crates/ubiq/src/theme.rs` |
| Placement, the description table, and the first real layout | `crates/ubiq/tests/ui_id.rs` |
| The peel order and the cursor's lifetime | `crates/ubiq/tests/dismiss.rs` |

§7 is what it decided. It touches no element outside the overlay itself, as designed.

**Phase 3a — the rail and the titlebar. Built.** Every one of the thirty-four catalogue names
carries `.ui_id(...)` on a real element: `crates/ubiq/src/ui/rail.rs` (the rail itself, the mark,
the modes column, each rail mode, the project badges group) and `crates/ubiq/src/ui/titlebar.rs`
(the row, the project cluster, back/forward, the command field, the actions cluster and each
action in it). Two shapes needed a small addition on top of a bare `.ui_id(mode)` call:

- **`RAIL_MODES` and `RAIL_PROJECTS` needed a wrapper.** The rail's groups and its project badges
  were spliced into the rail as flattened `.children(...)`, with nothing holding "all the groups"
  or "all the badges" as one element to name. Each got a thin `div()` matching the rail's own
  `flex().flex_col().items_center()`, carrying the name, with the existing children moved inside
  it — additive nesting, not a restyle.
- **`TITLEBAR_PROJECT` needed one too, for a different reason.** `ui::project_menu::window_badge`
  and `ui::project_menu::render` both return an opaque `impl IntoElement`, and `project_menu.rs` is
  outside this change's scope, so `.ui_id()` cannot be chained onto their result from
  `titlebar.rs` — the trait bound `.ui_id()` needs is erased by the return type. A wrapper `div()`
  around both, built in `titlebar.rs`, is what carries `TITLEBAR_PROJECT` instead.

`nav_control` is called once per direction with a different name each time, so it took a `UiId`
parameter to carry it through; `command_field` and `bell` are each called exactly once and mark
themselves internally instead. `crates/ubiq/tests/ui_id.rs` renders the
real `AppState` behind a window with two open projects — one project makes `RAIL_PROJECTS` draw
with zero badges and a zero-width rectangle, which is real but not what "sane rectangle" means for
a mechanical check — and asserts every claimed name is recorded with a non-zero rectangle, that
`rail.modes` and `rail.projects` sit inside `rail`, and that a point inside a rail mode resolves to
that mode rather than to `rail`. `G317` is closed: the mode has real, proven targets.

**Phase 3b — the rest of the tree, and the first help pages.** Not built. Every other screen area
is still unmarked, and no name yet resolves to a help page rather than a sentence. It owns the
judgement calls phases 1 and 2 deliberately do not make: which further elements deserve a name, and
which of them are worth a page as well as a sentence.

## 4. How the overlay finds what is under the cursor

The central mechanism, and the one thing phase 1 had to get right.

**Per-element hover handlers cannot work.** The overlay covers the window, so every mouse move lands
on the overlay and nothing underneath ever receives one. The overlay has to hit-test for itself,
against a list of rectangles rather than a tree of callbacks.

**A marked element reports its own bounds during layout.** `.ui_id(id)` adds one
absolutely-positioned child whose `canvas` prepaint closure records `(UiId, MarkRect)` — an element
does not learn where it ended up until layout has run, and a `canvas`'s prepaint is the hook GPUI
offers for reading it. `ui::web_view` marks its live webviews the same way. The extension makes its
receiver `relative`, because an absolute child measures against its nearest positioned ancestor; the
consequence is that `.ui_id()` and `.absolute()` conflict on one element, and the wrapper is what
gets marked.

**The registry is per window, and double-buffered.** `crates/ubiq/src/ui/ident.rs` holds one
`MarkRegistry` per `WindowId` in a thread-local — frame bookkeeping, not application state, and
routing it through `AppState` would mean an `Entity::update` inside prepaint. The two buffers are
forced by ordering: the whole element tree, overlay included, is built before layout runs, so an
overlay asking what is under the cursor can only be answered from the frame before.
`begin_frame()` retires the frame that finished and starts a fresh one; `hit()` reads the retired
one. One frame of lag on a highlight that follows the mouse is invisible; reading a half-filled list
would not be.

**Deepest wins, then smallest, then latest.** `deepest_hit()` ranks every mark containing the point
by name depth first — `rail.mode.tasks` beats `rail`, because the deeper name describes the same
pixel more precisely — then by area, because two names at the same depth over one point are
overlapping siblings and the smaller is what the user means, then by record order, because layout
order is paint order. It assumes neither that a name's ancestors were marked nor that a child's
rectangle sits inside its parent's: an anchored menu is a child of its trigger by name and elsewhere
on screen, and it hit-tests correctly.

**The coordinate space is the window's, and phase 2 proved it.** `G316` was the open question under
all of the above: no `.ui_id()` had ever been laid out, so whether `canvas` prepaint reports bounds
in window coordinates or relative to some ancestor was an assumption.
`a_mark_records_where_the_element_really_is` in `crates/ubiq/tests/ui_id.rs` lays out a titlebar and
a rail in a real GPUI window — the test platform, no graphics device — and asserts the rail is
recorded at `(0, 34)` rather than `(0, 0)`. Window coordinates, which is what the hit-test and the
highlight both assume. The same test proves the two other halves of that row: the marked divs come
out at exactly the size and position the unmarked layout gives them, so the forced `relative`
displaces nothing, and the registry answers only after a second frame is drawn, which is the
documented one-frame lag behaving as designed. What it does *not* cover is a marked element with
children of its own or one that is already `.absolute()` — `G318`.

`MarkRect` is the crate's own rectangle rather than GPUI's `Bounds<Pixels>`, which is what lets
`crates/ubiq/tests/ui_id.rs` drive the whole lookup — nesting, overlap, exact ties, shared edges,
zero-area rectangles, an empty registry, a cursor over nothing — with no window and no rendering.

## 5. What phases 1, 2 and 3a deliberately leave out

- **Every mark call outside the rail and the titlebar.** Phase 3b. The rest of the tree calls no
  `.ui_id()` yet, so pointing anywhere but those two areas still finds nothing marked.
- **Any instance form.** `explorer.tree.row` would name a kind of place and there is no way to say
  *which* row. Static areas and controls are what the catalogue holds.
- **Aliases and deprecation.** The catalogue is a list of names with a label and a sentence beside
  each and no alias column, so §2.3's rename contract is a rule with no mechanism behind it.
- **Any help *page* bound to a name.** §2.5's second half. A balloon has its sentence; nothing reads
  a `ui.*` string, `_tools/helpbundle.py` does not know the namespace, and a click on a target is
  inert because there is nothing for it to open — `G314` and `G319`.
- **Any lint.** `is_known()` is the check and nothing outside the crate calls it, so a `.ui_id()` on
  a name the catalogue does not hold compiles — the name is a `const` a call site can invent.

Each of those is a row in [`backlog.md`](../backlog.md): `G312` through `G315`, plus `G319` and
`G320` for what phase 2 left behind it. `G317` — the mode having nothing to point at — is closed by
phase 3a.

## 6. A second design exists and disagrees

[`inbox/ui-id-proposal.md`](../inbox/ui-id-proposal.md) and
[`inbox/ui-id-tooling-proposal.md`](../inbox/ui-id-tooling-proposal.md) were written against this one
and argue four changes: the catalogue should be a TOML data file carrying a label, a sentence and a
`moved_from` alias rather than a Rust `const` list; help should bind through a new `targets:` page
field rather than through the `context:` ladder; repetition should be a typed instance selector; and
a `uiids.py` lint should join `just verify`. It keeps the registry and the deepest-hit lookup
unchanged.

Phases 1 and 2 implement the design in this document, as briefed. The disagreement is a real and
open one and is not settled by this having been built first — three of the four points are the
backlog rows in §5, and phase 2 conceded the fourth in substance while keeping the shape: the
sentence does live beside the name rather than on a help page, and it lives there in a `const`
table behind `describe()`, which is about as cheap a thing to replace with a generated one as it
could be.

## 7. What phase 2 decided

The four judgement calls §3 left to it, and the reasons.

**The mode outranks every other layer.** `Layer::HelpTarget` is the top rung, above `Dropdown`,
above the menus and above every modal, and `ui::shell` paints it after the bell's list — the last
overlay in the window. Everything below that rung is something the user is *doing*; this one is a
question about what they are looking at, and a dialog's own controls are exactly the things a
reader cannot otherwise ask about. Being the top rung costs nothing and buys three things from the
order for free: Escape peels it first (its arm is the first in `cancel_dialog`, ahead even of the
menu), `covered()` stops any layer below dismissing itself on a click that happened up here, and
the mode can cover a modal and point at it. The price is that Escape means "stop pointing" while
the mode is up rather than "close what I was doing" — which is the right meaning, and the reason
`escape_takes_in_place_help_before_anything_under_it` also asserts that the stack underneath is
exactly where it was.

**Two ways in, and F1 keeps its job.** ⇧F1 — beside F1 because it is the same question asked the
other way round, distinct from it because the answers are different sizes — bound in both the
`Workbench` and the `Input` contexts the way F1 is, so it works with the caret in a field. And
**Point at something…** in the titlebar's overflow menu, directly under **Help**. The `?` control
and F1 are untouched: `reveal_help` opens a page about where you are standing, `open_help_target`
waits for you to point.

**Two ways out, and a click is neither.** Escape, and a **Done** button in a small bar at the
bottom right of the overlay — bottom right because the titlebar and the rail are the two areas a
reader most wants to point at, and a bar across either would be furniture standing on its own
subject. The catcher `.occlude()`s and binds no press at all, so a click anywhere is inert: the
brief is that the mode persists until it is explicitly closed, and a mode that ends on a stray
click is a mode the reader loses by trying to point precisely. Opening the target's page on a click
is the obvious next gesture and is `G319` — it needs a page to open.

**An unmarked region reads as nothing, and a nameless one says so.** No hit means no rectangle and
no balloon; the bar's note says to keep moving and that unnamed areas have nothing to say yet,
rather than highlighting the window and calling it "the window". A name `describe()` does not know
is a different case and gets a balloon with its leaf as the label and one line saying the
description has not been written — because marking is incremental and a name arriving before its
sentence is expected, not broken.

**The window is not dimmed.** No `scrim` under the highlight, which is the one place this overlay
departs from the modal shape it otherwise follows. A modal dims because what is behind it is not
the subject; here it is exactly the subject, and a reader pointing at a button wants to read the
button. The highlight is `theme::fade(theme::accent(), 0.18)` under a one-pixel `accent` border —
faint enough to see through — and the mode announces itself with the balloon and the bar instead.

## Related docs

- [`help.md`](./help.md) — the help pipeline this binds into: pages, the packer, `catalog.json`, the context ladder
- [`../features/workbench.md`](../features/workbench.md) — the areas the scheme names, and where the mode belongs when it ships
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the render model, the overlay shapes, and where the grammar belongs durably
- [`../inbox/ui-id-proposal.md`](../inbox/ui-id-proposal.md) — the competing design, §6
