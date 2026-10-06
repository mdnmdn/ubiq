<!-- Adapted from Archify 3.0.1 SKILL.md and references/ (MIT, (c) tt-a1i). See NOTICE.md. -->
# Archify diagrams in Ubiq

You author typed JSON diagrams (`architecture`, `workflow`, `sequence`, `dataflow`, `lifecycle`). You
write a `<slug>.archify` file in the project (the same JSON; its type is its `diagram_type`),
validate it with the `archify_*` tools, and repair it from the diagnostics. Older `*.<type>.json`
files are still accepted: edit them where they are, but name a new diagram `.archify`. The Ubiq editor shows a `.archify` file as a diagram and reloads it whenever it changes on disk, so the
user watches the diagram appear. No Node, no HTML, no browser step: the file is the deliverable.

## Tools

| Tool | Use |
|---|---|
| `archify_schema(type)` | The schema, one minimal example and the field notes for a type. Call it before using a field or enum you have not used. |
| `archify_validate(path \| json, quality?)` | Validate a file (project-relative or absolute `path`) or an inline `json` document. Returns `{ok, stage, diagnostics[], notChecked}`. `type` comes from `diagram_type` (always, for a `.archify` file; else the `<type>.json` suffix). `quality` (`standard` \| `showcase`) overrides gate severity only. `repo_root` (a directory inside the project) names the Git checkout that source evidence is verified against; the default is the project's own root, which must be the repository's top-level directory. |
| `archify_layout(path \| json)` | Resolved boxes, exact route points, label positions and viewBox (repair evidence) for every type; architecture and workflow also for a layout the gates reject (workflow is the compiler receipt, `fixed-v1` or `readable-v2`). |
| `archify_render(path \| json, quality?)` | Compiles the file, opens or focuses its tab in the editor and returns `{ok, type, profile, counts, viewBox, diagnostics}`; a failure is the validate receipt and shows nothing. |
| `archify_new(type, name, dir?)` | Creates `<dir>/<slug>.archify` (default: the project root) holding a minimal valid starter of that type, never over an existing file, and opens its tab. Optional: you may instead write the file whole yourself (step 3). |
| `archify_guide(topic? \| scenario?)` | More detail. `scenario` (plain words, "order lifecycle from cart to refund") recommends one of 12 recipes: the type, what to include, starter prompts. Topics: `reference` (numbers, tables, full per-type placement), `authoring-defaults`, `architecture-layout-repair`, `geometry-rules`, `labels`, `repair`, `architecture`, `workflow`, `sequence`, `dataflow`, `lifecycle`, `mermaid-everyday`, `evidence`. |

A tool missing from the tool list, or answering "unavailable", is not built yet: skip that step and
never invent its result. The per-type topics hold the schema notes and a minimal example that passes
Archify's own validator.

## The loop

1. **Choose the type** from the question (router below). Ambiguous: pick the one whose unit matches
   what the reader will follow (a path, a step, a message, a record, a status).
2. **Read in one batch**: `archify_schema(<type>)` (it includes the example). This guide already
   carries the authoring defaults. Examples teach shape, not facts: use fresh ids, wording and
   layout. Do not probe, list directories or write throwaway diagrams first.
3. **Write the whole JSON in one go** to `<slug>.archify` (the user's named location, otherwise
   `diagrams/` at the project root; `diagram_type` inside says the type, no type in the name). Place and budget before writing coordinates (sections below),
   but do no coordinate planning in prose. Set `meta.quality_profile` to `"showcase"` unless the
   user wants dense `standard`. Use automatic routes first.
4. **Validate** with `archify_validate(path)`. A first draft needs no separate pre-check.
5. **Repair** by the order and the retry bounds at the end. `ok: false` is never success.
6. **Report**: file path, diagram type, the validation result (errors, warnings, quality used),
   what `notChecked` lists, and any remaining gap. Do not claim a visual inspection you did not do.

The document is always JSON on disk. Edit the file (or rewrite it whole) and validate again; a
half-written file is shown with the previous good picture plus the new diagnostics.

## Type router

| Type | Use for | Unit |
|---|---|---|
| `architecture` | Components, services, cloud/security boundaries, infrastructure; what something everyday is made of | box + connection, on a grid |
| `workflow` | Processes, approval gates, tool calls, runbooks, CI/CD; step-by-step plans | step in a lane/column (`col` 0..5) |
| `sequence` | API call chains, request lifecycles, async traces; back-and-forth between people | message, ordered by `y` |
| `dataflow` | Pipelines, ETL/ELT, lineage, governance; where money or documents go | node in a stage/row, flow named by data |
| `lifecycle` | State/status transitions, retries, waiting and terminal states; where an order stands | state in a lane/column (`col` 0..4) |

Use schema_version `2` for new `workflow` and `lifecycle`, `1` for the others. Everyday subjects
(leave plan, approval, renting, order status) use the same five types: use everyday `icon` values and
`meta.legend` label overrides, and ask for missing personal facts instead of inventing dates,
amounts or rules. Mermaid input: read it for topology, author fresh JSON (`archify_guide("mermaid-everyday")`).

## Authoring rules

1. **Question first, then type.** Overview by default (architecture: the main user journey); a
   deliberately narrower scope (one mechanism, a module map, a deployment) is named in the title.
2. **No invented fields.** Every object rejects unknown properties. Check the schema before a new
   field, enum or constrained string. Common enums: node `type` = `frontend backend database cloud
   security messagebus external`; relationship `variant` = `default emphasis security dashed`
   (sequence adds `return`).
3. **Ids** match `^[a-zA-Z][a-zA-Z0-9_-]*$` and are unique within their collection. Give relationships
   an `id` (architecture `connections`, workflow `edges`, dataflow `flows`, sequence `messages`) and
   keep it stable across edits: it is the durable address of that arrow.
4. **References resolve.** `from`/`to`, `wraps`, `lane`, `stage`, `mainPath` and `activations.participant`
   name existing ids. A dangling reference or duplicate id is an error.
5. **Envelope.** `schema_version`, `diagram_type` and `meta{title, output}` are required.
   `meta.output` is only checked for syntax (a portable relative path ending `.html`, e.g.
   `"<slug>.html"`); nothing is written there. One concise `title`. Omit `subtitle`, `visual_preset`,
   `legend`, `engineering_profile` and (architecture) `viewBox` unless asked.
6. **Preserve what was asked.** Every requested responsibility, direction, protocol and
   behaviour-changing condition appears. No node, edge or boundary quota. Cards (`cards[]`, notes)
   answer extra questions and never replace topology; secondary or opt-in capabilities go in cards
   unless their path matters. User-supplied or agreed topology is fixed: do not merge or drop nodes
   to get fewer crossings.
7. **Gates belong on the arrow.** An approval, authorization or state condition is shown on the
   affected node or edge, not only in a card.
8. **Grouping.** Group cooperating roles into one accurately named subsystem only if the diagram
   still explains the interaction; keep them apart when grouping hides control ownership, a trust or
   persistence boundary, or lifecycle behaviour.
9. **Boundaries** (`region`, `security-group`) only for real isolation, ownership, runtime or
   persistence. A one-node boundary needs an explicit isolation fact. External actors stay outside,
   including the boundary's padding (leaving a node out of `wraps` does not exclude it visually).
10. **Automatic routing first.** Add `fromSide/toSide`, `via`, `channelX/Y`, `route`,
    `labelAt/labelDx/labelDy/labelSegment` only for a necessary branch/return, supplied geometry or a
    diagnosed defect. They also switch off automatic port spread and label fallback. After moving a
    node, delete the stale generated overrides of its edges.
11. **Labels are data.** Keep protocol, action, direction, sync/async and cross-boundary meaning.
    Omit a label only when both endpoints fully imply it. Never delete one to fix spacing: move the
    label, then adjust route/spacing, then shorten the wording.
12. **One language** for authored text. Set `meta.locale` only to `en` (built-in); otherwise omit it
    and tell the user the viewer chrome stays English. Keep product names, identifiers, protocols
    and paths as written.
13. **Real repository:** attach `sources` to key nodes only for facts you actually read at that
    location; never infer runtime causality from file names (`archify_guide("evidence")`).
14. **Unknowns stay visible.** Name uncertainty in a card or label; never guess a fact to fill a box.

## Placement: decide it before the coordinates

The router cannot rearrange boxes, so placement decides whether lines stay straight. Classify every
relationship, then place (architecture first; the other types apply the same thinking to lane/col,
stage/row, participant order):

- **Main path**: the reader's journey, neighbours adjacent in reading order. A medium path steps
  through rows instead of forming a shallow strip. Start the main actor and its first step near the
  origin.
- **Branch or store**: directly above/below the node that owns it, centred, so the edge is one
  straight segment. All branches/stores of one row go on the same side.
- **Return** (back to an earlier node): leave from the side of the main path with no branches or
  stores, so it runs through an empty corridor. A feedback cycle goes around an open rectangle in
  edge order (`archify_guide("architecture-layout-repair")`).
- **Second entrance** into a node that already has an incoming edge: arrive from another side.
- **Fan-out**: a side with k relationships needs `32 + 14(k-1)` px (four need 74). Spread a hub's
  counterparts over two or three sides, or enlarge the hub. Centre a parent on its children.
- **Trace before writing**: each non-main relationship's straight or one-bend corridor must not pass
  another node or cross another relationship. If it does, move the endpoint that is off the main path.
- A shared store sits between its readers and writers, on an adjacent row if needed.

## Label budgets (conservative, for the first draft)

Text is estimated, never measured; ASCII = 1 unit, CJK and emoji = 2. Two figures exist and they
differ on purpose (00 §9 Q3): **budget** with the conservative number below while drafting, and
expect the validator to enforce the smaller one. A diagram that passes does not need more room just
to meet the budget; one that fails by the enforced figure must shrink or widen.

| Thing | Budget while drafting | Enforced (validator) |
|---|---|---|
| Edge label mask | `6.5u + 13` px | box `4.8u + 10` (A, W), `4.9u + 12` (D, L), `6.6u + 12` showcase (S) |
| Clear gap of a labelled main-path edge | `6.5u + 21` px | `composition/label-gap` (A): `ceil(mask + 16)` |
| Sublabel width | `5.4u + 8` px | minimum font must fit `width - 8` |
| Node label fits its node | `k·u <= width - 8` | fails when `k·u > width + p` |

Node-label constant `k` per type: **architecture 6.6** (`p` 8), **sequence and workflow 6.8** (`p` 6),
**dataflow and lifecycle 6.2** (`p` 6). With the default widths that is about 17 units in an
architecture grid cell (130 px), 16 in a dataflow node (112 px), 12 in a workflow node (92 px) and
11 in a sequence participant (86 px; `column_fit: "spread"` widens columns). A longer label: shorten
it, move detail to `sublabel`, `tag` or a card, or set the node `width`. Gap means space between
boxes, not centre distance (200 px between 165 px nodes leaves 35). Keep labels to a few words
(`METHOD /path`, a verb, a data asset). Full formulas and per-type constants: `archify_guide("reference")`.

## Mode must-knows

- **architecture** (v1): grid (`layout {mode:"grid"}`, `row`/`col`) instead of free `pos`/`size`
  (free only as a bounded exception). Omit `meta.viewBox`; the canvas measures itself. `emphasis` =
  main path, `security` = auth/consent/policy/PII, `dashed` = async/batch/event. Boundaries may
  wrap members across rows. `engineering_profile: "deployment-ownership"` only for an explicit
  production-topology or ownership review, and then never removed to pass.
- **workflow** (v2 for new): lane = responsibility (`variant: "exception"` for human wait, denial,
  retry, failure), `col` 0..5 = progression. Set `mainPath` for the happy path (consecutive ids need
  an edge and move left to right). Retries and returns stay outside the main corridor. Prefer a
  `route` preset (`drop`, `outside-right`, `return-left`, `bottom-channel`, `up-channel`) before raw
  `via`. `semanticChecks` only for facts the evidence establishes. Never flip `schema_version` on a
  document with absolute coordinates. Stacked sequential stages: one lane, one group, no `viewBox`.
- **sequence** (v1): participants in conversation order, left to right. `y` strictly increasing,
  about 25-30 apart, at least 160; messages closer than 28 must not share horizontal span; columns at
  least 60 apart; no self-messages (`from` != `to`). `return` = quiet response, `dashed` = async.
  `segments` are phase guides, `activations` mark busy spans. No automatic port spread.
- **dataflow** (v1): `stage` = transformation or custody (source, ingest, process, store,
  consume), `row` separates parallel streams (0..4). Flows go forward through stages; a flow within
  one stage needs different rows. The flow label names the data asset, not the transport;
  `classification` for sensitivity, `security` for PII or policy paths, `dashed` for batch or async.
- **lifecycle** (v2 for new): each populated lane is one row, `main` first, `terminal` last. `col`
  0..4 is one x grid shared by every row: put an interruption or exit in the column of the state it
  leaves so its transition is a straight vertical. Author every transition including the main path.
  Keep transition labels short. A recoverable failure needs a real transition back to an active
  state; a "retry" card is not topology. Omit `viewBox`.

## Quality profiles and what is enforced

`meta.quality_profile`: absent = advisory, `standard`, or `showcase` (the default to author for).
The `quality` argument of `archify_validate` overrides severity only, never the layout itself.

Which gates run depends on the build. Trust the table below and the `notChecked` field of every
receipt, not this prose: if `archify_validate` reports `notChecked`, those gates are not enforced and
a pass says nothing about them. At the time of writing all five types (architecture, dataflow,
sequence, lifecycle v1 and v2, workflow v1 and v2) run complete geometry, the post-render artifact
checker on the drawn routes (rounded corners included), and, for a call that names a Git checkout,
the verification of repository evidence against the committed blobs at the pinned SHA. Not run: the
browser stage (`viewer/*`) and, with no checkout, the evidence citations (`root-required`), so a pass
says nothing about those; say so in the report. If a build ever lists a type's geometry under
`notChecked`, the numbers in this
guide are your only protection for it: apply them by hand and say that layout is unchecked. Where a type renders in the
diagram view is separate from validation; `archify_render` opens the tab and reports counts, not what the picture looks like: never claim a
rendering you did not see.

<!-- GATES_TABLE -->

## Repair order

Run `archify_validate` after every edit. Consume `diagnostics[]` by stable `code`, exact
`subject` (path, id), `evidence` and `supportedFixes`; use a diagnostic's own `labelAt` if given.
Compare diagnostics by `code + subject + stage`, never by count. Fix in this order:

1. `meta.quality_profile` and schema errors (stage stops here: later checks only run on a valid shape).
2. Graph errors: duplicate ids, dangling references, bad cells or ranges.
3. Node overlap and out-of-range placement.
4. Edge through a node, endpoint direction.
5. Crossings, ambiguous corridors, border runs, detours, route rhythm.
6. Label vs node, label vs label, then label vs route.
7. Labels off the canvas: move the label (a suggested `labelDx/labelDy` replaces the authored
   value, it is not added) or widen `meta.viewBox`; negative left/top needs an inward move.

A failing stage hides the later ones, so a repair that passes may reveal new diagnostics: that is
progress, not regression. Several diagnostics on the same nodes usually mean one placement problem:
fix the placement once (`archify_guide("architecture-layout-repair")`) rather than nudging labels
one by one. Never weaken the document (drop a `semanticChecks` entry, an `engineering_profile`, a
semantic label, a node, a boundary) just to pass.

## Bounded retries

A round is one edit plus one validate. Per issue (the same `code + subject`): two focused repair
rounds, then one evidence-based round (`archify_layout`, else the diagnostic's
`evidence`), then stop. Per document: **six rounds** without `ok: true`, then stop. A round that
clears a diagnostic and exposes a later-stage one is progress; the same diagnostic surviving three
rounds, or an oscillation between two, is not. Do not continue with blind coordinate changes.

When you stop, leave the last candidate on disk, do not call it done, and report: the file, the
remaining diagnostics (`code`, `subject`), what each round tried, and the constraint you suspect
(for example fixed user geometry or a label that cannot fit). Ask the user whether to relax the
scope instead of weakening the document yourself.
