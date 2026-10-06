<!-- Adapted from Archify 3.0.1 SKILL.md and references/ (MIT, (c) tt-a1i). -->
# Repair loop

`archify_validate` returns `{ok, stage, diagnostics[]}`. Each diagnostic has a stable `code`
(`schema/<keyword>`, `graph/...`, `composition/...`, `clean-flow/...`, `layout/...`, `i18n/...`), a
`severity`, a `message`, a `subject` (JSON path plus the nearest `id`/`label`), `evidence` (measured
values) and `supportedFixes`.

1. Stages run in order and a failing stage stops the pipeline: shape (schema), then references and
   graph, then geometry. Warnings (`i18n/*`) never block.
2. Fix in this order: quality profile and schema; graph (ids, references, cells); node overlap or
   range; edge through node and endpoint direction; crossings, corridors, border runs, detours,
   rhythm; label vs node, label vs label, label vs route; labels off canvas.
3. Common schema fixes: `schema/additionalProperties` = remove the unsupported property;
   `schema/required` = add it; `schema/enum` = choose one of the listed values.
4. Edit the connected neighbourhood, validate again. Compare by `code + subject + stage`, not count.
5. Preserve requested meaning and source evidence. Do not drop `semanticChecks`, an enabled
   `engineering_profile`, or a semantic label to pass.
6. Limits: two focused repairs of one issue, then look at the geometry (`archify_layout`), then one
   more evidence-based retry, then report the concrete gap. At most six rounds per document (see
   "Bounded retries" in the main guide).
7. A passing result proves only the gates listed in the main guide. Say so; do not claim more.
