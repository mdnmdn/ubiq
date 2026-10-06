# Golden fixtures

Oracle data for `ubiq-archify`: the frozen output of the Archify 3.0.1 Node oracle. Generated once,
committed, never regenerated at build or test time. Regenerating is outside this tree: a changed
golden is a change in Archify's behaviour.

## Files

| File | Content |
|---|---|
| `<name>.golden.json` | one case (below) |
| `manifest.json` | one row per case: `name group type source rule expect_codes diagnostics` (a per-quality summary of the codes Archify emitted) |

Case names: `example.*` are not prefixed (`web-app.architecture`, `agent-run.lifecycle`, ...);
`fixture.<dir>.<file>` come from `archify/test/fixtures/`; `neg.<slug>` are mutations.

## Groups

- **example** (15): every `archify/examples/*.json` diagram (the 5 types).
- **fixture** (21): `test/fixtures/` documents: `v1-baseline/*` (the v1 form of every type),
  `v1-workflow-*`, the Japanese and Korean locale architecture documents, `lifecycle-planner/*`,
  `issue-126/*` (workflow pins and labels), the workflow routing documents. All pass at their own
  profile; a few fail under `--quality showcase`, which the receipt records. The locale catalogs in
  `examples/locales/` are not diagrams and are not goldens: `fidelity_cases.json` (below) applies
  them to the five example documents.
- **negative** (162): small mutations of valid documents (bases in the oracle's negatives), one or more per
  rule family of the Archify spec section 3.4 a-c. `rule` is the 00 rule id (or
  Archify's code where 00 has none), `expect_codes` the code(s) the mutation was aimed at, and
  `validate_receipt` what Archify actually said. **Where they differ, the receipt wins**: the golden is
  Archify's behaviour, not the map's.

## Case shape

```text
name, group, type, source          provenance (source is a path in the Archify tree, or "mutation of <base>")
rule?, expect_codes?, note?        negatives only
source_sha256                      sha-256 of the document bytes as written to disk
source_doc | source_text           the parsed document; source_text (raw string) when it is not valid JSON
source_missing?                    true for the missing-file case (no document exists)
quality                            the document's own meta.quality_profile, or null
schema_version                     the document's schema_version, or null
validate_receipt                   { authored, standard, showcase } -> { exit_code, receipt?, stdout?, stderr? }
layout_json?                       architecture/workflow only: { exit_code, receipt | stdout | stderr } for `validate --layout-json`
layout_json_by_quality?            standard/showcase layout receipts, only for those that differ (examples/fixtures)
render?                            extracted geometry of the rendered SVG (examples and fixtures, not negatives)
render_by_quality?                 standard/showcase renders, only for qualities whose geometry differs
render_error?                      when `render` failed: { ok:false, exit_code, stderr }
```

### validate_receipt

The three keys are three runs of `archify validate <type> <doc> --json`: `authored` (no flag, the
document's own `meta.quality_profile`), `--quality standard` and `--quality showcase`. `receipt` is
Archify's JSON (00 section 3.6): `ok`, `stage` (`input` | `render` | `check`), `diagnostics[]` (`code`,
`severity`, `message`, `subject`, `evidence`, `supportedFixes`, `suppresses?`) on failure; `checks[]`,
`composition{profile,status,summary,metrics,issues,routeReview}` on success. When the CLI printed no
JSON, `stdout`/`stderr` hold the raw text. Paths are normalised to `<input>` and `<cwd>`.
Warnings that Archify only prints (the `i18n/*` ones) are in `stderr`, not in the receipt.
`fidelity_cases.json` has them in full (below).

## `fidelity_cases.json` (P9.1)

`{ locales, cases }`, from `gen-fidelity-cases.mjs` (`tests/fidelity.rs`). `locales` holds the
`examples/locales/*.json` catalogues once; a document's `meta.translations: "@<tag>"` refers to one.
Cases, by `kind`:

- `render` (131): one document rendered by `archify render`, with `stderr` (the `archify: ...`
  warnings), `nodes[{id, context, brand?{mark, title, status, source, x, y, d, inset, scale, hex}}]`
  and `legend{title, items[{kind, label, x, baseline, width}]}`. `brands.<type>.<n>` put every one of
  the 107 marks on the nodes of an example in six spellings (id, title, alias, domain URL, padded,
  upper case); `locale.<type>.<variant>` apply `zh-CN`, `xx`, partial and complete catalogues and
  overrides of `en` and `zh-CN`.
- `diag` (75): the same locale documents with an unknown brand on the first node, run through the
  renderer with `ARCHIFY_DIAGNOSTIC_FORMAT=json`: `diagnostics` is what Archify recorded, the V4
  warnings (with their evidence) then the `brand/unknown` error.
- `validate` (20): `brand/*` failures through `validate --json`, with the receipt.

Archify's own CLI mangles a failing run that also warned (it cannot parse its stderr and answers
`internal/unclassified`); the `diag` cases take the recorded diagnostics from the renderer instead.

### layout_json

`archify validate <type> <doc> --layout-json`. Architecture: `{ok, diagram_type, layout, viewBox,
components[], boundaries[], connections[{points, labelAt}], labels[]}`. Workflow: the compiler
receipt `{contract: fixed-v1 | readable-v2, viewBox, requiredViewBox, columns, nodes, edges[{points}],
labels, diagnostics}`. A rejected layout keeps its diagnostics (exit 1).

### render

Extracted with a regex/attribute parser (`svg-extract.mjs`) from `archify render` output. All
numbers are Archify's un-rounded values.

| Field | Meaning |
|---|---|
| `viewBox` | `[minX, minY, width, height]` |
| `svg_attrs` | the root `data-*` attributes (`data-preset`, `data-quality-profile`, `data-layout-contract`, `data-sequence-column-fit`, ...) |
| `title` | the `<title>` |
| `nodes[]` | `{id, kind, label, sublabel?, tag?, context, rect{x,y,width,height,rx}, class, stroke_width, texts[], sigil?, initial_marker?, data}`; sequence participants and lifecycle states are nodes too |
| `edges[]` | `{key, id?, from, to, label?, points, d, variant, stroke_width, dash?, role?, routing?, crossover?, independent?, marker_end}`; `points` is `data-composition-points`, the canonical un-rounded route, and `d` the painted path with rounded corners |
| `labels[]` | `{edge_key, edge_id?, from, to, text, rect, texts[]}`: the mask rect behind each edge label and its text lines |
| `frames[]` | `{kind, id, label?, class, rect}`: `data-composition-frame-*` (architecture boundaries, workflow lanes, groups and exception lanes, dataflow stages, sequence segments) |
| `legend` | `{bridge, title, items[{semantic_kind, kind?, label?, x, baseline, width, shapes[]}]}`; absent when no legend is drawn |
| `decor` | everything else, keyed by the SVG section comment (`Lifelines`, `Activations`, `Segment Labels`, `Swimlanes`, `Phase headers`, `Lifecycle bands`, `Boundary labels`, `Data Stages`, ...): `{tag, class, x, y, width, height, d, text, data}` shapes |

`texts[]` entries: `{x, y, font_size, text, font_weight?, anchor?, detail?, class, role?}`.

## Negative coverage

Every family of 00 section 3.4 a-c has at least one case, except:

- `schema/not`, `schema/portablePath`, `portable-path/*`: unreachable through `validate`; the
  `output/meta-*` pre-check fires first (see below).
- `output/meta-resolved-extension`, `output/meta-outside-cwd`: need symlinks on disk, not generated.
- `composition/arrowhead-collision`, `composition/label-band-title-overlap`,
  `workflow/solver-budget-exhausted`, `workflow/input-contract`, `workflow/invalid-node-column`
  (the schema rejects first): no mutation reached them.
- `sequence/timeline` height rule (H < 327): the schema floor is 480.
- `repository-evidence/*` verification codes (git): deferred, D19.
- `artifact/*` and `viewer/*`: the stage-V10/V11 checks fire on emitted HTML only.

`manifest.json` lists, per case, the codes Archify emitted at each quality.

## Known traps (read before comparing)

- With no `meta.quality_profile`, `validate` reports `composition.profile: "standard"`, not the
  advisory mode 00 section 3.3 describes. Most architecture gates read the **authored** profile, so
  the negatives that need showcase set it in the document; `--quality` only partly overrides it.
- Schema failures come back with `stage: "render"` (the renderer child runs the schema), except a
  missing `meta.output`, which the CLI's own pre-check reports.
- A present `meta.output` is checked before the schema: bad paths surface as `output/meta-absolute`,
  `output/meta-path-syntax` (the reason is `evidence.reason`) and `output/meta-extension`, not as
  `schema/portablePath` or `portable-path/*`.
- A missing `pos` in a free or grid architecture crashes the renderer; Archify reports
  `internal/unclassified` ("Cannot read properties of undefined").
- Workflow v2 is a solver: pins and presets it cannot honour surface as `workflow/explicit-pin-conflict`
  or `workflow/route-preset-conflict`; `workflow/short-edge` is never emitted as a code in 3.0.1.
- Repository-evidence path/line/file checks need `--repo-root` on a git checkout; they are not
  generated (deferred, D19). Only the checks that precede `root-required` are.
- Error message text is V8's for `input/json-parse`; compare by `code`, `subject.path`, `severity`.
