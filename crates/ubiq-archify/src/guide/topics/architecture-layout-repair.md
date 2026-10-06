<!-- Adapted from Archify 3.0.1 references/architecture-layout-repair.md (MIT, (c) tt-a1i). -->
# Architecture layout repair

Use when several routes or labels collide, or fixing one moves the defect onto another. Keep every
component, relationship, label, boundary and source; change placement.

1. **Scope.** An isolated defect while the main chain reads clearly: fix it locally. A blocked main
   chain, several tangled routes, or a defect that moves: reflow the connected scene in one edit.
2. **Trace.** Write the reader's main path as an ordered list of existing edges. Identify shared
   state and real feedback cycles. Every neighbouring pair on the main path must have the
   relationship being explained; other components sit on branches beside their owner.
   `archify_layout` (when available) shows resolved boxes, frames, points and labels: use it once,
   do not guess repeated waypoint coordinates. `directCorridorBlockers` names nodes between aligned
   endpoints: evidence, not a failure by itself.
3. **Place.** Main-path neighbours in reading order. Shared state between its readers/writers, on
   an adjacent row if needed. Arrange a feedback cycle in edge order around an open rectangle:
   for `client -> API -> dispatcher -> worker -> collector -> runner -> dispatcher`, the first four
   on the upper row, collector below worker, runner below dispatcher; the return then uses the
   lower row. This shows adjacency, not coordinates to copy.
4. **Routes.** Keep automatic endpoints. After moving nodes, delete stale generated `via`,
   `channelX/Y` and `label*` overrides. Constrain a side only for a branch/return that needs a
   specific corridor, and check it against every affected relationship. If that creates another
   conflict, return to connected placement instead of cycling through side combinations.
5. **Boundaries.** External actors stay outside the boundary rectangle including its padding (not
   listing a node in `wraps` does not exclude it visually). Keep an internal relationship and its
   label inside the shared boundary unless crossing conveys a real fact.
6. **Labels.** On `composition/label-gap` enlarge the clear gap or move the connected group to
   another row. Keep the label beside its own route.
7. Validate once after the edit. One bounded second repair for a remaining specific defect; then
   report the concrete gap rather than continuing blind coordinate changes.
