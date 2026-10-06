<!-- Adapted from Archify 3.0.1 references/authoring-defaults.md (MIT, (c) tt-a1i). -->
# Authoring defaults

The composition and presentation defaults behind the main guide's authoring rules.

## Composition and meaning

- Architecture defaults to a system overview led by the main user journey, unless asked for a
  narrower mechanism, module map or deployment topology. Name a narrower scope in the title.
- Group cooperating roles in an accurately named subsystem only if that still explains the
  interaction. Keep roles separate when grouping would hide control ownership, a trust or
  persistence boundary, or lifecycle behaviour.
- Keep secondary or opt-in capabilities in concise cards unless their path matters.
- Preserve every requested responsibility, direction, protocol and behaviour-changing condition.
  Approval, authorization and state gates go on the node or edge they affect.
- There is no node, edge or boundary quota. Cards answer extra questions; they never replace topology.
- Edge labels carry meaning (action, protocol, direction, async, cross-boundary). An edge may start
  unlabelled only if both endpoints fully imply it.

## Layout and budgets

Placement (main path, branch or store, return, second entrance, fan-out `32 + 14(k-1)` px, the
corridor trace) and the conservative label budgets are in the main guide, sections "Placement" and
"Label budgets"; the width constants and per-type thresholds are in `archify_guide("reference")`.
Omit `meta.viewBox` for a fresh architecture.

## Presentation

Omit `visual_preset` (classic), `subtitle`, `legend` (auto lists only kinds present) and
`engineering_profile` unless explicitly requested. One primary language; `meta.locale` only `en`.
New `workflow` and `lifecycle`: schema_version 2. Sequence: start with fixed columns;
`meta.column_fit: "spread"` when labels need width or the viewBox is wide.
