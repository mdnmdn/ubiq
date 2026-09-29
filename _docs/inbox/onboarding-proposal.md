---
id: inbox-onboarding
title: Proposal — Onboarding, a first-run tour that points at the running interface and hands over to the manual
kind: proposal
status: proposal
summary: One mechanism — a list of steps shipped as data in the help bundle — covers the welcome modal, the coach marks that highlight a real control, and the announcement of a new feature; a step is centred or anchored to a `ui.*` name, advances on a button or on a live condition, and can hand the reader to the Help panel at the page that explains what it just pointed at; separately and reusing the same page ids, a modal grows an optional help drawer along its side.
read_when: you are building the onboarding tour, writing tour content, adding help to a dialog, or deciding when a new feature announces itself
updated: 2026-09-29
depends_on: [inbox-help, wip-help, wip-in-place-help, inbox-ui-id, feat-workbench, tech-ui, tech-transport]
---

# Proposal — Onboarding

Ubiq now has a manual — a Help panel, a bundled content tree, a contextual ladder, and a mode that
points at a control and tells you its name ([`wip/help.md`](../wip/help.md),
[`wip/in-place-help.md`](../wip/in-place-help.md)). Every one of them waits to be asked. A person
opening Ubiq for the first time sees an empty catalogue, a rail of eight modes whose icons mean
nothing yet, and no reason to press `?`.

This proposes the thing [`inbox/help-proposal.md`](./help-proposal.md) §12 deliberately deferred —
help that points at the running interface rather than describing it — and states that it is now
cheap, because the two things §12 said were missing both exist: an overlay layer above every modal
(`Layer::HelpTarget`, `ui/help_target.rs`) and an anchor vocabulary that names a live element
(`state/ui_id.rs::UiId` plus the per-frame `MarkRegistry`).

The argument is one sentence: **the welcome modal, the coach mark and the "what's new" card are the
same mechanism seen three times**, and that mechanism is a list of steps shipped as data beside the
help pages. Build one thing, get all three.

---

## 1. What exists, and what onboarding borrows

| Need | Already in the tree |
|---|---|
| A layer above every modal | `state/overlay.rs::Layer::HelpTarget`, painted in `ui/shell.rs::render` |
| Name a place on screen | `state/ui_id.rs::UiId`, `CATALOGUE`, `describe` → `TargetInfo` (38 names, rail + titlebar) |
| Find where that place was drawn | `ui/ident.rs::Identified::ui_id` → `MarkRegistry`, `deepest_hit`, `hit_mark_at` |
| Draw a highlight and a balloon that stays on screen | `ui/help_target.rs::highlight`, `balloon`, `place` (private today) |
| A centred card with a title, a body and buttons | `ui/kit/overlay.rs::modal`, `modal_sized`, `modal_note` |
| Content, versioned and shipped | the `UBIQBND1` help bundle, `_tools/helpbundle.py`, `HelpCatalog` |
| A page to hand over to | `PanelKind::Help`, `AppState::open_help_page`, `ubiq://./help/<id>` |
| Somewhere to put "seen it" | `ui-settings.toml`, `Settings::get/set(SettingsLayer::Ui)` — opaque to the host |
| A non-interrupting announcement | `ui/notifications.rs` |
| Which version is running | `crates/ubiq/src/version.rs::FULL` |

Onboarding adds: one step type in `ubiq-proto::help`, one tour file format the packer validates, one
state struct, one overlay arm, one `Layer` rung, a seen-record, and an optional drawer on
`kit::modal_sized`.

## 2. A tour is a list of steps, and steps come in two shapes

A **tour** is an ordered list of steps with an id and a version. A **step** is a title, a short
markdown body, and — optionally — a target, a page and an advance condition.

- A step **with no target** is a **centred card**, drawn with `kit::modal_sized`. This is the
  welcome modal, and it is step 0 of the first-run tour rather than a separate screen.
- A step **with a target** is a **coach mark**: the named element is highlighted and the body is
  drawn in a balloon beside it, using the painters `ui/help_target.rs` already has.

That single rule is what makes "start from the modal and hand over to the highlights" fall out with
no seam. The tour does not switch mode between step 0 and step 1; only whether the step names a
target changes.

Every step draws the same chrome: `Back`, `Next` (or `Done`), a step counter, and `Skip the tour`.
Escape skips. Nothing in a tour is modal in the blocking sense — see §5.

## 3. The content format

Tours live in the help content tree, under `help/tours/<id>.toml`, and the packer reads them exactly
as it reads page frontmatter. They are content, not code, which is the whole point: retargeting or
rewording the first-run tour must not be a release.

```toml
id = "first-run"
title = "Welcome to Ubiq"
version = 3                      # bump to re-offer a tour that changed materially
since = "2026.10"                # the build this tour describes
announce = "notify"              # "never" | "notify" | "open"  — see §6

[[step]]                         # no target: a centred card
title = "Ubiq runs agents in real terminals"
body  = "Every agent gets a pane. Panes live in **sessions**."
page  = "concepts-panes"         # optional "Read more" → the Help panel

[[step]]                         # a target: a coach mark on the rail
target = "ui.rail.mode.agents"
title  = "Start here"
body   = "Open Agents mode and start your first agent."
advance = { rail_mode = "agents" }   # a do-it step: advances when it happens
```

`target` is a qualified `UiId` (`ui.` + path, `state/ui_id.rs::qualified`). `page` is a help page
id, the same handle a link, the MCP server and the context ladder use. `body` is short markdown
rendered by the existing renderer — a balloon that needs scrolling is a page, not a step.

The packer writes the tours into `catalog.json`, so the interface reads them off `HelpCatalog` with
no second file format at runtime. That is an **additive** `crates/ubiq-proto` change —
`HelpCatalog::tours: Vec<HelpTour>` behind `#[serde(default)]`, so a bundle built before this lands
still loads.

## 4. Advance conditions, and why they are a closed list

A passive step has a `Next` button. A **do-it** step also watches for something to happen, and
advances itself when it does — because "open Agents mode" teaches more when the reader does it than
when they read that they could.

The conditions are a small closed enum matched against state the window already holds:

| Condition | True when |
|---|---|
| `rail_mode = "<mode>"` | `WorkbenchState`'s rail mode is that one |
| `panel_open = "<PanelKind::name()>"` | that panel is revealed |
| `view = "<View>"` | the current destination is that view |
| `has_project = true` | the catalogue is not empty |
| `has_pane = true` | the window owns at least one pane |

Five is enough for the tours worth writing, and the list grows one row at a time with a reason.
**An unknown condition is ignored, and the step becomes passive.** Content ships on its own cadence
from the binary that reads it, so a newer bundle in front of an older build has to degrade rather
than stall — the same rule §7 of `help-proposal.md` already applies to a
missing page. A do-it step always keeps its `Next` button too: a reader who does not want to click
the rail is not trapped.

## 5. Targets that are not on screen

A tour points at a live interface, and a live interface may not be drawing what the step names — a
panel is closed, a mode is elsewhere, the build is older than the content, the name was renamed.

**A step whose target has no rectangle in the retired frame degrades to a centred card with the same
title and body.** Not a stall, not a skipped step, not an arrow pointing at the corner. The reader
loses the highlight and keeps the sentence, which is the part that carries the meaning.

Three further rules the targeting mode already learned and this inherits:

- the overlay reads the **retired** frame's marks (`MarkRegistry` is double-buffered), so a step
  that changes what is on screen settles one frame later — visible as nothing, since the tour
  advances on a click
- the tour's rung sits **below** `Layer::HelpTarget` and above everything else: ⇧F1 over a tour
  still points at a control, and Escape peels the pointer first, then the tour
- clicking the highlighted element **works**. The tour's catcher occludes the window for the
  balloon's sake, but a do-it step that says "click the rail" and then eats the click is a bug, so
  the catcher has a hole at the target rectangle. This is the one genuinely new piece of overlay
  behaviour; the pointer mode, which is inert by design, has no equivalent.

A tour only points at singletons — a rail mode, a titlebar control, a panel's header. Pointing at
"the third row" needs the instance form `G313` says `UiId` does not have, so tour content may not
try, and `help-check` cannot catch it. One line in the content guide, and a gap.

## 6. When a tour runs

**Exactly one tour opens by itself: the first run.** No record for `first-run` in the UI settings and
an empty project catalogue → the welcome card opens once the window has settled.

Everything else **announces and waits**. A tour whose `announce = "notify"` and whose `since` is
newer than the last version this user saw raises a notification — *"New in 2026.10: missions. Show
me."* — and runs only if pressed. An established user in the middle of their work does not get their
window taken over by a card about a feature they did not ask for, and a notification is the surface
Ubiq already has for "something happened that you may care about". `announce = "open"` exists, is
honoured, and content should almost never use it.

The record is one JSON object inside the opaque UI settings blob:

```json
{ "tours": { "first-run": { "version": 3, "step": 4, "done": true } } }
```

`version` is the tour's, so re-offering a materially rewritten tour is a content decision. `step` is
where a tour that was interrupted resumes — the window closing mid-tour is the ordinary case, not a
failure. `done` is set by both finishing and skipping, because a tour that reappears after a skip is
the single most resented pattern in this genre.

`Help → Tours` in the panel's header lists every tour in the bundle and runs any of them again. A
tour you cannot replay is a tour you must not miss, which makes it stressful; a replayable one can
be short.

## 7. Help beside a dialog

Separate feature, same content, and independently shippable: **a modal may carry a help page, drawn
in a drawer along its side.**

`kit::modal_sized` takes one more argument, `help: Option<&'static str>` — a page id. When it is
`Some`, the modal's header draws a `?`; pressing it widens the card with a second column that
renders that page with the markdown renderer, scrolling on its own, links handled by
`ui::on_link`. Pressing it again collapses. The drawer is part of the modal card, so it moves,
dismisses and layers with it and needs no second `Layer` rung.

- **One drawer at a time.** `WorkbenchState::modal_help: Option<String>` holds the open page, and it
  belongs to the topmost modal; opening a second modal closes it. Modals stack rarely and a second
  drawer behind the first would be furniture.
- **Its open/closed state is remembered per modal id**, in the same UI settings blob as the tours.
  Someone who wants the help beside Settings wants it every time; someone who does not, never does.
- **The page id is the dialog's own claim**, which is rung 1 of the contextual ladder that
  `wip/help.md` §5 already defines — `fn help_page() -> Option<&'static str>`. So F1 in the dialog,
  the drawer, and the MCP server's `help_for_context` all land on the same page, from one
  declaration. This proposal is what finally gives that rung callers: Settings (per tab), New agent,
  New mission, Clone project, Remote hosts, KB source, Task import.
- ⇧F1 still works over a modal and still points at its controls. The drawer explains the dialog; the
  pointer explains one field. They are complements, and a reader will use both.

The cost is honest and small: a page per dialog that must be written and must stay true, and a
`help-check` rule that every id named by a `help_page()` exists — which needs the set of claimed ids
as data, the same requirement §8 has.

## 8. What the packer needs, and the one prerequisite

`just help-check` must validate a tour or the format is a trap: every `target` a real `UiId`, every
`page` a real page, every `advance` key known, every tour id unique, no empty tour.

The page and condition halves are local to the packer. The target half is not: the names live in a
Rust `const CATALOGUE` that Python cannot read. This is `G315` and the subject of
[`inbox/ui-id-tooling-proposal.md`](./ui-id-tooling-proposal.md), and onboarding is the use case
that settles it — **the catalogue becomes a data file** (`ui-targets.toml`, `include`d by the Rust
rather than transcribed), and the packer and the lint read the same file the interface does. Doing
it the other way — a `just` recipe that dumps the catalogue out of a Rust binary into `target/` for
Python to read — works, and pays for it with a build ordering nobody will remember.

Treat the data-file move as a **prerequisite of phase 2**, not of phase 1: a tour of centred cards
names no target and needs no catalogue.

## 9. Studio

Tours are namespaced with their bundle, exactly as pages are (`help-proposal.md` §9). Studio ships
its own tours, may point at `ui.*` names only Studio's build has, and the degradation rule of §5 is
what makes a Studio tour harmless if it is ever read by a base build. The first-run record is keyed
by qualified tour id, so `ubiq/first-run` and `studio/first-run` are two tours and a user running
both editions is asked once each — which is correct, since they are different products.

## 10. Phases

1. **Centred tours.** The step model in `ubiq-proto`, `help/tours/first-run.toml`, the packer's
   reading and validation of the page half, the state and the overlay drawing centred cards only,
   the seen-record, the first-run trigger, `Help → Tours`. Ships a real welcome flow with no
   dependency on the `UiId` catalogue.
2. **Coach marks.** `ui-targets.toml` (§8), target resolution off the retired frame, `highlight` /
   `balloon` / `place` promoted to `pub(crate)`, the hole in the catcher, the degradation rule,
   marks on whatever the first tour points at that is not already marked.
3. **Handover.** `Read more` on a step opens the Help panel at the page; the tour survives the panel
   opening beside it; the last step offers `Open the manual` and lands on `getting-started`.
4. **Do-it steps.** The five conditions, the ignore-unknown rule.
5. **The modal drawer.** `kit::modal_sized`'s `help` argument, `help_page()` on the seven dialogs,
   the remembered per-modal state. **Independent of 1–4** — if dialog help is worth more than the
   tour, do this first.
6. **Announcements.** `announce`, `since`, the version comparison, the notification.

Phases 1 and 5 are the useful minimum, and they do not depend on each other.

## 11. What this owes the library when it lands

- A feature document for onboarding under `features/` — deleting the tour from Ubiq would
  delete it.
  It owns the tour format, the trigger rules and the drawer. `wip/help.md` keeps the bundle and the
  panel; `wip/in-place-help.md` keeps `UiId` and the pointer.
- A fact-ownership row in `INDEX.md` §3 for the tour format and the onboarding record.
- Decision rows, next free number **D197**: a tour is bundle content rather than Rust; the welcome
  modal is step 0 of the tour rather than its own screen; only the first run opens itself and every
  other tour announces through a notification; a missing target degrades to a centred card; an
  unknown advance condition degrades to a passive step; the `UiId` catalogue moves to a data file;
  the modal drawer reuses the `help_page()` ladder rung rather than a second binding.
- Backlog rows from **G386**: no instance form for a target, so a tour cannot point at a row
  (`G313`); `help-check` cannot prove a `help_page()` id is claimed by a real dialog until the
  claims are data; a tour against a **remote** host, which inherits `wip/help.md`'s open question
  about where the bundle lives; no content lint on balloon body length.
- `tech/operations.md`: no new recipe, but `help-check`'s list of what it validates grows.

## 12. Deliberately not now

**An agent that runs the tour.** The help agent of `help-proposal.md` §12,
asked "show me how to start an agent", could drive the same step list and answer questions between
steps. It needs the tour mechanism first, and it needs the account policy question that proposal
already parked.

**Recording what the reader did.** Which step a tour was abandoned on is the one number that would
actually improve the content, and Ubiq collects no telemetry. Out of scope, and not by accident.

**Per-project onboarding.** A project shipping its own tour of its own conventions is a plausible
extension of the same format, and belongs with
[`inbox/plugin-system-proposal.md`](./plugin-system-proposal.md)'s contributed content rather than
here.

**Anything animated.** No spotlight fade, no scroll-into-view, no arrow that draws itself. A
rectangle and a balloon, drawn where the element is.

---

## Related docs

- [`inbox/help-proposal.md`](./help-proposal.md) — the help system, and §12 where this was deferred
- [`wip/help.md`](../wip/help.md) — what of help was actually built: the bundle, the panel, the ladder
- [`wip/in-place-help.md`](../wip/in-place-help.md) — `UiId`, the mark registry, the pointer overlay
- [`inbox/ui-id-tooling-proposal.md`](./ui-id-tooling-proposal.md) — the catalogue as a data file, and the lint
- [`features/workbench.md`](../features/workbench.md) — the rail, the modes, the regions a tour points at
- [`tech/ui-and-design.md`](../tech/ui-and-design.md) — the kit, the layers, the render model
