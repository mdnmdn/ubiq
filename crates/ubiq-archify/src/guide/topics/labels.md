<!-- Adapted from Archify 3.0.1 references/authoring-contract.md (MIT, (c) tt-a1i). -->
# Labels

Relationship labels are semantic data: protocol, action, direction, sync vs async, cross-boundary
mechanism.

- **Collision repair order**: move the label (`labelAt`, `labelDx/labelDy`, `labelSegment`), then
  adjust route or spacing, then shorten the wording keeping its meaning. Deleting a label is not a
  spacing repair; omit one only if both endpoints already imply it and it carries none of the above.
- If a diagnostic gives `labelAt`, use that point.
- Apply one diagnosed geometry control at a time, unless several edges share a constrained channel:
  then plan the smallest coupled change and validate it together.
- **Budgets** (units: ASCII = 1, CJK = 2): label mask `6.5 * units + 13` px; first-draft clear gap
  of a labelled main-path edge `6.5 * units + 21` px; clear gap must exceed mask + 8 px; node
  sublabel width `5.4 * units + 8` px.
- A long sublabel: keep the role or protocol exact and short; put the extra fact in a card or `tag`.
- **Legend**: omit `meta.legend` (auto lists only kinds present). `mode: "all" | "hidden" | "auto"`;
  `entries.<kind>.{label<=80, visible}` change wording only. Never use the legend to cover for a
  missing node.
- **Title**: one concise `meta.title`; no `subtitle` unless asked.
