<!-- Adapted from Archify 3.0.1 references/authoring-contract.md (MIT, (c) tt-a1i). -->
# Geometry rules

Numbers Archify's validator enforces. This server enforces only the gates in the table of the main
guide; the rest is authoring guidance until its gate lands. A rule marked (any profile) is a hard
failure whenever it is checked.

- **Anchors** start at side midpoints. `left/right` change the horizontal endpoint, `top/bottom`
  the vertical one. A side is a direction contract: the first and last route segment are
  perpendicular to it, leaving/entering outward (`clean-flow/endpoint-side-direction`).
- **Explicit `via`**: with departure anchor S, first waypoint F, last waypoint L, arrival anchor T
  (SVG y grows downward):

  | Side | Departure S to F | Arrival L to T |
  |---|---|---|
  | top | `F.x == S.x`, `F.y < S.y` | `L.x == T.x`, `L.y < T.y` |
  | bottom | `F.x == S.x`, `F.y > S.y` | `L.x == T.x`, `L.y > T.y` |
  | left | `F.y == S.y`, `F.x < S.x` | `L.y == T.y`, `L.x < T.x` |
  | right | `F.y == S.y`, `F.x > S.x` | `L.y == T.y`, `L.x > T.x` |

  Explicit routes get no automatic port spread: do not reuse an anchor copied from an automatic route.
- **Port spread** (architecture, workflow, dataflow, lifecycle; not sequence, not explicit
  `via/channel*/labelAt` or non-`auto` routes): shared endpoints spread symmetrically with a 16 px
  corner gutter.
- **Route rhythm** (showcase): every nonzero segment >= 8 px, every interior segment >= 16 px.
- **Detour** (showcase): an explicit route fails `composition/excessive-route-detour` when its length
  is >= 2.5x an obstacle-aware legal route, +200 px, with a control point >= 96 px outside the content.
  Remove the unneeded `via` rather than enlarging the canvas.
- **Overlap and crossings**: unrelated collinear overlap >= 8 px fails showcase; a long run along a
  container border fails; an edge crossing an unrelated opaque node always fails (any profile);
  unrelated proper X crossings are a warning in `standard`, an error in `showcase`.
- **Labels**: mask width ~ `6.5 * ASCII units + 13` px (CJK = 2 units); clear gap must exceed mask
  + 8 px. Gap means space between boxes, not centre distance (200 px between 165 px nodes leaves
  35 px). Any `labelAt/labelDx/labelDy/labelSegment` (even 0) disables the automatic label
  relocation. A suggested `labelDx/labelDy` replaces the authored value; it is not added to it.
- **Canvas**: use the measured failing side of `layout/boundary-out-of-bounds`. Negative left/top
  needs an inward move; widening `viewBox` only fixes right/bottom. Boundaries may wrap members
  across rows.
