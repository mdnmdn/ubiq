<!-- Adapted from Archify 3.0.1 references/{authoring-defaults,authoring-contract,architecture-layout-repair}.md (MIT, (c) tt-a1i). -->
# Reference

The long material behind the main guide: numbers, per-type placement, repair contracts, presentation
defaults. Read the part you need; nothing here changes the loop. Numbers marked *enforced* are
checked by Archify's validator and by this server once the gate for that type is listed in the main
guide's table (check `notChecked` in a receipt); the rest is authoring guidance.

## Topics

| Topic | Holds |
|---|---|
| `authoring-defaults` | Composition, meaning and presentation defaults |
| `architecture-layout-repair` | Reflowing a tangled architecture in one coherent edit |
| `geometry-rules` | Anchors, explicit `via` table, port spread, rhythm, detour, crossings, canvas |
| `labels` | Label repair order, legend, title |
| `repair` | Diagnostic shape and the repair loop |
| `architecture` `workflow` `sequence` `dataflow` `lifecycle` | Schema notes and a validated minimal example |
| `mermaid-everyday` | Mermaid mapping, everyday subjects, icons |
| `evidence` | Repository evidence |

## 1. Text and width models

Text is estimated, never measured. `units(s)` = 1 per character, 2 for East-Asian wide/fullwidth
characters and emoji, 0 for variation selectors. Rendering uses `width = units * 0.6 * fontSize`
(monospace). Validation uses its own per-type constants, which are slightly larger on purpose:

| Type | Node/participant label constant `k` (px/unit) | Allowed over width `p` | Edge-label box |
|---|---|---|---|
| architecture | 6.6 | 8 | `max(30, 4.8u + 10)` |
| workflow | 6.8 | 6 | `max(30, 4.8u + 10)` |
| sequence | 6.8 | 6 | `max(34, 6.6u + 12)` showcase, `5.2u + 12` standard |
| dataflow | 6.2 | 6 | `max(34, 4.9u + 12)` |
| lifecycle | 6.2 | 6 | `max(32, 4.9u + 12)` |

*Enforced* (`layout/constraint`, "label wider than node"): `k * units > width + p`. Longest line of an
edge label counts (for dataflow the `classification`, for lifecycle the `note`). Boundary title:
`max(30, 0.6 * u * fs + 10)`.

Conservative budgets for a first draft (the authoring contract's figures, kept because they leave
slack over every constant above):

- edge-label mask `6.5u + 13` px; clear gap of a labelled main-path edge `6.5u + 21` px; the gap
  must exceed mask + 8 px;
- sublabel width `5.4u + 8` px (9 px preferred text at 0.6 em);
- node label: `k * u <= width - 8`, so the renderer never has to shrink the font toward its floor.

Worked example, 10-unit edge label: budget mask 78 px, gap 86 px; enforced box 58 px (A, W),
61 px (D, L), 78 px (S showcase); `composition/label-gap` (A) asks for `ceil(58 + 16) = 74` px. In
sequence the enforced box already equals the budget, so there is no slack to give back.

Default node widths: architecture grid cell 130 px (free node 120), dataflow 112, workflow 92,
sequence participant 86 (spread columns up to 190). Sublabels and tags may shrink to their floor
(6-7 px) before `geom/text-min-fit` fires; shorten before widening.

## 2. Validator thresholds by type

Enforced where the type's geometry gate runs; otherwise apply by hand.

| Rule | A | W | S | D | L |
|---|---|---|---|---|---|
| Node overlap, minimum gap (px) | 8 | 8 (same lane) | n/a | 10 | 10 |
| Edge / message minimum length (px) | 24 | 28 | 60 between columns | 34 | 32 |
| Nodes inside the canvas | viewBox | lane, below title strip | last participant `cx + w/2 <= W - 40` | `x` in [24, W-24], `y` in [104, H-74] | `x` in [28/32, W-28/32] |
| Other | self-loop ports >= 24 apart | v1 viewBox width >= 696 | height >= 327; message `y` in [160, H-83]; messages < 28 apart in `y` must not share horizontal span | `via` segments orthogonal | v1 viewBox height >= 566 |

Always errors, any profile: an edge through an unrelated node (`clean-flow/edge-through-node`,
2 px clearance), a first or last route segment not perpendicular to its side
(`clean-flow/endpoint-side-direction`), duplicate ids, dangling references. Severity follows the
profile (showcase is strictest; standard downgrades some to warnings, e.g. crossings; the receipt
says which): proper crossings of unrelated routes (`composition/proper-crossing`),
unrelated collinear overlap >= 8 px (`ambiguous-corridor`), a long run along a container border
(`container-border-run`), segments < 8 px or interior segments < 16 px, labels within 4 px of a
foreign route (`label-route-clearance`) or outside the canvas (`label-canvas-containment`),
arrowheads closer than `3.5 * (stroke1 + stroke2)` (`arrowhead-collision`), and an explicit
architecture route >= 2.5x its obstacle-aware shortest, +200 px, with a control point >= 96 px
outside the content (`excessive-route-detour`: delete the `via`, do not enlarge the canvas).
Reader-viewport checks (`desktop-readability`, `viewport-height`) need a browser and are not run here:
keep a diagram readable at 1440 x 900 with no horizontal overflow and text near 7.5 px or larger.

## 3. Per-type placement in full

**Architecture.** Choose overview or mechanism detail first. One obvious primary reading path, which
may step across rows. Keep the overview at its chosen abstraction and expand implementation detail
only when it answers the reader's question. Grid placement is preferred, free `pos`/`size` is a
bounded exception. Keep external actors outside the system boundary when that is true. After a
reflow, re-check every connected route; do not cycle through side combinations, go back to
placement (`archify_guide("architecture-layout-repair")`). An automatic canvas includes route points
as well as nodes, frames and labels; an authored `viewBox` is authoritative, and
`layout/route-out-of-bounds` (showcase) names clipped route points.

**Workflow.** Lanes express responsibility or phase, columns 0..5 progression. Keep the happy path
monotonic, edge labels intact, retries and exception returns outside the main lane corridor. Stacked
sequential stages: one v2 lane and one group, centre nodes with symmetric `yOffset` (`-90 / 0 / 90`),
omit `viewBox`. Several steps sharing the last column with large `yOffset` values make lanes tall:
redistribute steps across columns and lanes, keeping the main path monotonic. Never merge lanes, drop
pins or flip `schema_version` to escape a constraint; if ownership or pins prevent a reflow, report
that. Let the compiler allocate a label's measured mask before applying a diagnosed `labelAt`,
`labelDx/labelDy` or `labelSegment`.

**Sequence.** Participants in conversation order; messages own the vertical order. Use
return/async/security variants for meaning, never decoration. Start with fixed columns; use
`meta.column_fit: "spread"` when the viewBox is wide or labels need width. A message label needs the
column span between its endpoints; a long label belongs in `note` or a shorter wording.

**Dataflow.** Stages are transformation or custody; rows separate parallel streams. Label only data
contracts, classifications or cross-boundary movement that is not obvious.

**Lifecycle.** v2 (new): one row per populated lane, `main` first, `terminal` last, `col` 0..4 one
shared x grid; the renderer sizes the canvas, widens a column gap for a same-row label and routes
automatic transitions orthogonally through the row gaps. v1 (legacy): main phases use columns 0..4,
event and terminal bands use 0..2 (column N aligns with main column N+2), every other lane shares one
middle band, and states sharing a column there need distinct `yOffset`.

## 4. Repair contracts

**Composition repair** (the abstraction was wrong). Before regrouping, map every affected role,
relationship direction, protocol, boundary, condition and source to its surviving node or
relationship. A startup citation does not prove a message protocol. Keep user-supplied topology
fixed; fewer routes alone do not justify merging. A boundary around one node needs an explicit
isolation fact and must not merely repeat its label.

**Label repair.** Move the label, adjust route or spacing, then shorten wording while keeping its
meaning. Omit wording only when both endpoints imply it and it carries no protocol, action,
direction, sync/async or cross-boundary fact; if you start unlabelled, say why. Spacing means clear
gap, not centre distance; a measured mask beats the budget. Apply one diagnosed geometry control at a
time unless several edges share a constrained channel, then plan the smallest coupled change from
measured geometry and validate it together. Any `labelAt/labelDx/labelDy/labelSegment`, even 0,
disables the automatic label fallback.

**Schema lookup.** Read the type's schema and the shared definitions before adding a field, enum or
constrained string: `archify_schema(type)`. Shared enums: node `type` (7 values), `variant` (4, +
`return` in sequence), boundary `kind` (`region`, `security-group`), `nodeIcon`. An example shows
structure, not every legal value.

**Evidence.** `meta.repository` and `sources` are checked for shape only; the truth of a citation is
yours (`archify_guide("evidence")`).

## 5. Presentation defaults

- **Language.** One primary authored language: the user's choice, else the request's, else the
  conversation's. `meta.locale` controls only viewer chrome (`<html lang>`, default legend labels,
  controls), never your text. `en` and `zh-CN` are built in. For any other language omit
  `meta.locale`, write every reader-facing string in that language, and tell the user the viewer
  chrome stays English. Never mix a locale tag with no catalog, never substitute one language's tag
  for another. Keep exact product, code, protocol, command, API and environment names as written.
- **Legend.** Omit `meta.legend`: `auto` lists the kinds present. `mode`: `all | hidden | auto`;
  `entries.<kind>.{label (<= 80), visible}` change wording only. Adding `meta.legend` makes the
  presentation strict: if its labels cannot fit the canvas, shorten, hide, or widen the `viewBox`
  (`legend/*` diagnostics). Never use it to cover a missing node, state, message or flow.
- **Style.** Omit `meta.visual_preset` (classic) unless the user names `signal-flow`, `blueprint` or
  `editorial`. Omit `meta.subtitle` unless asked, and never restate the title in it.
- **Engineering profile.** Omit `meta.engineering_profile` for an ordinary overview; "region" or
  "security boundary" wording alone does not enable it. `deployment-ownership` needs an explicit
  production-topology, ownership or fail-closed review and known facts, and then is repaired, not
  removed.
- **Icons.** Optional `icon` on components, nodes, participants, states: everyday `calendar clock
  person briefcase flag moon`, `none` hides it, absent keeps the type default. Icons are decorative
  and do not change colour or grouping: the label still carries the meaning. Pair one with a
  `meta.legend.entries.<kind>.label` rename for everyday subjects.

## 6. Diagnostic code families

`input/*` (read, `json-parse`), `output/*` and `portable-path/*`, `schema/<keyword>`
(`additionalProperties` = remove it, `required` = add it, `enum` = pick a listed value),
`relationship/duplicate-id`, `layout/*` (`constraint` is the catch-all for sizes, ranges, overlaps and
label fits; `boundary-out-of-bounds`, `route-out-of-bounds`), `clean-flow/*`, `composition/*`,
`legend/*`, `workflow/*`, `engineering/*`, `repository-evidence/*`, `i18n/*` (warnings, never block).
A code is stable; the message prose is not.

## 7. Final report

State: file path; type and `schema_version`; `quality_profile` used and any `quality` override;
`ok`, error and warning counts with the codes of anything left; the receipt's `notChecked` list; the
repair rounds used; and, if you stopped on the retry bound, the unresolved diagnostics and your
suspected constraint. Do not claim a visual review, a rendering you did not see, or a check outside
the enforced table.
