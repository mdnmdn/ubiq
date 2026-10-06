//! Edge-label anchors and boxes (P2.4). `00` §5.7.
//!
//! [`label_point`] is `labelPoint` of `renderers/shared/geometry.mjs:1750`, the default anchor
//! every type starts from. The dataflow label box is `flowLabelSize` plus the rect of
//! `renderFlowLabel` (`render-dataflow.mjs:67-73, 461-471`). [`place_automatic_labels`] and
//! [`reserved_label_rect`] are `renderers/architecture/labels.mjs` (P3.3 needs the reservation);
//! the spatial index there is a speed-up over the same boolean test, so this scans the segments.
//! `gridSweep` (not used by architecture) is not ported. The showcase relocation is wired into the
//! architecture build by `layout::architecture_build::relocate_labels`.

use crate::geom::{Pt, Rect, Seg, normalize, rects_overlap, segment_rect_clearance_within};
use crate::text::{self, DiagramType, Profile};

/// The authored placement hints of an edge (`labelAt`, `labelDx`, `labelDy`, `labelSegment`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hints {
    pub at: Option<Pt>,
    pub dx: Option<f64>,
    pub dy: Option<f64>,
    pub segment: Option<f64>,
}

/// `labelPoint(item, points)`: `labelAt` wins; a 2-point route puts the label at `(midX + dx,
/// y0 - 10 + dy)`; a polyline at the midpoint of segment `labelSegment ?? 1` (clamped to the
/// route), `y - 10`. A fractional `labelSegment` indexes `points[1.5]` in JS (a crash); it is
/// truncated here. The schema says integer.
pub fn label_point(hints: &Hints, points: &[Pt]) -> Pt {
    if let Some(at) = hints.at {
        return at;
    }
    let (dx, dy) = (hints.dx.unwrap_or(0.0), hints.dy.unwrap_or(0.0));
    if points.len() == 2 {
        return [(points[0][0] + points[1][0]) / 2.0 + dx, points[0][1] - 10.0 + dy];
    }
    let last = points.len().saturating_sub(2) as f64;
    let index = hints.segment.unwrap_or(1.0).max(0.0).min(last) as usize;
    let (a, b) = (points[index], points[index + 1]);
    [(a[0] + b[0]) / 2.0 + dx, (a[1] + b[1]) / 2.0 - 10.0 + dy]
}

/// `layout.labelH` (`render-dataflow.mjs:64`): the box height without a classification.
pub const DATAFLOW_LABEL_H: f64 = 16.0;
/// The box height with a classification line (`:71`).
pub const DATAFLOW_CLASSIFIED_H: f64 = 27.0;
/// The label rect starts `11` above the baseline (`:469`); the classification baseline is `11` below (`:466`).
pub const DATAFLOW_LABEL_RISE: f64 = 11.0;

/// A dataflow label: the text anchor `at` (the label's baseline, centred) and its mask rect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowLabel {
    pub at: Pt,
    pub rect: Rect,
    /// Whether the second (classification) line is drawn.
    pub classified: bool,
}

/// `flowLabelSize(flow)`: width from the longer of the label and the classification
/// ([`text::edge_label_width`], rounded to 0.1), height 16 or 27. An empty classification is none.
pub fn dataflow_label_size(label: &str, classification: Option<&str>) -> (f64, f64) {
    let classification = classification.filter(|c| !c.is_empty());
    let mut lines = vec![label];
    lines.extend(classification);
    let width = text::edge_label_width(DiagramType::Dataflow, &lines, Profile::Standard);
    (width, if classification.is_some() { DATAFLOW_CLASSIFIED_H } else { DATAFLOW_LABEL_H })
}

/// The anchor and mask rect of a dataflow flow label on `points`.
pub fn dataflow_label(hints: &Hints, points: &[Pt], label: &str, classification: Option<&str>) -> FlowLabel {
    let at = label_point(hints, points);
    let (width, height) = dataflow_label_size(label, classification);
    FlowLabel {
        at,
        rect: Rect::new(at[0] - width / 2.0, at[1] - DATAFLOW_LABEL_RISE, width, height),
        classified: classification.is_some_and(|c| !c.is_empty()),
    }
}

/// The architecture connection label box (`connectionLabelBoxAt`, `render-architecture.mjs:162`):
/// `max(30, units * 4.8 + 10)` wide, 14 high, its top 10 above the anchor.
pub fn architecture_label_rect(label: &str, at: Pt) -> Rect {
    let width = text::edge_label_width(DiagramType::Architecture, &[label], Profile::Standard);
    Rect::new(at[0] - width / 2.0, at[1] - 10.0, width, ARCHITECTURE_LABEL_H)
}

/// The architecture label box height.
pub const ARCHITECTURE_LABEL_H: f64 = 14.0;

/// An edge-label plate as `placeAutomaticLabels` reads it (`architecture/labels.mjs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plate {
    /// `relationIndex`: the route whose segments the plate may sit on.
    pub rel: i64,
    /// `labelAt`/`labelDx`/`labelDy`/`labelSegment` authored: never moved.
    pub pinned: bool,
    pub rect: Rect,
    /// The text anchor (`lx`, `ly`).
    pub at: Pt,
}

impl Plate {
    /// `rectAt(label, lx, ly)`: same size, `x = lx - width / 2`, `y = ly - 10`.
    fn at(&self, lx: f64, ly: f64) -> Plate {
        Plate { rect: Rect::new(lx - self.rect.width / 2.0, ly - 10.0, self.rect.width, self.rect.height), at: [lx, ly], ..*self }
    }
}

/// The scene `placeAutomaticLabels` places into (without `gridSweep`, which architecture never
/// asks for).
#[derive(Clone, Copy, Debug)]
pub struct Placement<'a> {
    /// `(relationIndex, points)` per route.
    pub routes: &'a [(i64, &'a [Pt])],
    pub components: &'a [Rect],
    pub titles: &'a [Rect],
    pub view_box: [f64; 2],
    pub placement_bottom: f64,
    pub fallback_ring: bool,
    pub keep_fallback_near_route: bool,
}

struct RouteSeg {
    rel: i64,
    seg: Seg,
}

const LABEL_EPS: f64 = 0.0001;

/// `placeAutomaticLabels`: every unpinned label that is out of the canvas, on a component or title,
/// on another label or masking another route is moved to the first clear spot beside its own
/// segments (fractions 0.5, 0.25, 0.75, 0.125, 0.875; 4 offsets each), then, with
/// `fallback_ring`, to the 14-offset ring around its anchor and its segment midpoints. A label
/// with no clear spot keeps its collision for validation to report.
pub fn place_automatic_labels(labels: &[Plate], p: &Placement) -> Vec<Plate> {
    let segments = route_segments(p.routes);
    let mut placed = labels.to_vec();
    for index in 0..placed.len() {
        place_one(&mut placed, index, &segments, p);
    }
    placed
}

/// `reservedLabelRect`: where one unpinned label would go given only its own route, the routes and
/// labels so far and the components (no titles, no canvas, no ring); `None` when that spot still
/// overlaps a component.
pub fn reserved_label_rect(label: Plate, points: &[Pt], routes: &[&[Pt]], labels: &[Rect], components: &[Rect]) -> Option<Plate> {
    let mut all: Vec<(i64, &[Pt])> = vec![(-1, points)];
    all.extend(routes.iter().enumerate().map(|(i, r)| (i as i64, *r)));
    let mut placed = vec![Plate { rel: -1, ..label }];
    placed.extend(labels.iter().map(|r| Plate { rel: -2, pinned: false, rect: *r, at: [r.cx(), r.y + 10.0] }));
    let p = Placement {
        routes: &all,
        components,
        titles: &[],
        view_box: [f64::INFINITY, f64::INFINITY],
        placement_bottom: f64::INFINITY,
        fallback_ring: false,
        keep_fallback_near_route: false,
    };
    // Only the first plate is returned; the others are obstacles (moving them changes nothing).
    place_one(&mut placed, 0, &route_segments(&all), &p);
    let rect = placed[0];
    (!components.iter().any(|c| rects_overlap(&rect.rect, c, -2.0))).then_some(rect)
}

fn route_segments(routes: &[(i64, &[Pt])]) -> Vec<RouteSeg> {
    routes
        .iter()
        .flat_map(|(rel, points)| {
            let normalized = normalize(points);
            normalized.windows(2).map(|w| RouteSeg { rel: *rel, seg: Seg::new(w[0], w[1]) }).collect::<Vec<_>>()
        })
        .collect()
}

fn place_one(placed: &mut [Plate], index: usize, segments: &[RouteSeg], p: &Placement) {
    let label = placed[index];
    if label.pinned {
        return;
    }
    let inside = |r: &Rect| r.x >= 0.0 && r.y >= 0.0 && r.x + r.width <= p.view_box[0] && r.y + r.height <= p.view_box[1];
    let masks_route = |plate: &Plate| {
        segments.iter().any(|s| {
            s.rel != plate.rel && segment_rect_clearance_within(&s.seg, &plate.rect, 4.0).is_none_or(|c| c + LABEL_EPS < 4.0)
        })
    };
    let overlaps_label = |placed: &[Plate], rect: &Rect, gap: f64| {
        placed.iter().enumerate().any(|(k, other)| k != index && rects_overlap(rect, &other.rect, gap))
    };
    let clear = |placed: &[Plate], plate: &Plate| {
        let r = &plate.rect;
        inside(r)
            && r.y + r.height <= p.placement_bottom
            && !p.components.iter().chain(p.titles).any(|o| rects_overlap(r, o, 2.0))
            && !overlaps_label(placed, r, 2.0)
            && !masks_route(plate)
    };
    if inside(&label.rect)
        && !p.components.iter().any(|c| rects_overlap(&label.rect, c, -2.0))
        && !p.titles.iter().any(|t| rects_overlap(&label.rect, t, 0.0))
        && !overlaps_label(placed, &label.rect, 0.0)
        && !masks_route(&label)
    {
        return;
    }
    let (w, h) = (label.rect.width, label.rect.height);
    let own: Vec<&Seg> = segments.iter().filter(|s| s.rel == label.rel).map(|s| &s.seg).collect();
    for seg in &own {
        let (a, b) = (seg.start, seg.end);
        let mut candidates: Vec<Pt> = Vec::new();
        if (a[1] - b[1]).abs() < LABEL_EPS && (a[0] - b[0]).abs() >= w + 16.0 {
            for fraction in [0.5, 0.25, 0.75, 0.125, 0.875] {
                let x = a[0] + (b[0] - a[0]) * fraction;
                if (x - a[0]).abs().min((x - b[0]).abs()) < 8.0 {
                    continue;
                }
                candidates.extend([[x, a[1] - 10.0], [x, a[1] + 20.0], [x, a[1] - 18.0], [x, a[1] + 28.0]]);
            }
        } else if (a[0] - b[0]).abs() < LABEL_EPS && (a[1] - b[1]).abs() >= h + 16.0 {
            for fraction in [0.5, 0.25, 0.75, 0.125, 0.875] {
                let y = a[1] + (b[1] - a[1]) * fraction;
                if (y - a[1]).abs().min((y - b[1]).abs()) < 8.0 {
                    continue;
                }
                candidates.extend([
                    [a[0] - w / 2.0 - 6.0, y + 3.0],
                    [a[0] + w / 2.0 + 6.0, y + 3.0],
                    [a[0] - w / 2.0 - 14.0, y + 3.0],
                    [a[0] + w / 2.0 + 14.0, y + 3.0],
                ]);
            }
        }
        if let Some(found) = candidates.into_iter().map(|[x, y]| label.at(x, y)).find(|c| clear(placed, c)) {
            placed[index] = found;
            return;
        }
    }
    if !p.fallback_ring {
        return;
    }
    let bases = std::iter::once(label.at).chain(own.iter().map(|s| [(s.start[0] + s.end[0]) / 2.0, (s.start[1] + s.end[1]) / 2.0]));
    let step = w / 2.0 + 12.0;
    let wide = w + 20.0;
    let ring = [
        [0.0, -28.0],
        [0.0, 38.0],
        [-step, -28.0],
        [step, -28.0],
        [-step, 38.0],
        [step, 38.0],
        [-wide, -52.0],
        [wide, -52.0],
        [-wide, 62.0],
        [wide, 62.0],
        [-wide, -76.0],
        [wide, -76.0],
        [-wide, 86.0],
        [wide, 86.0],
    ];
    let near_route = |plate: &Plate| {
        !p.keep_fallback_near_route
            || own.iter().any(|s| segment_rect_clearance_within(s, &plate.rect, h * 2.0).is_none_or(|c| c <= h * 2.0))
    };
    let fallback = bases
        .flat_map(|[bx, by]| ring.iter().map(move |[dx, dy]| [bx + dx, by + dy]))
        .map(|[x, y]| label.at(x, y))
        .find(|c| clear(placed, c) && near_route(c));
    if let Some(found) = fallback {
        placed[index] = found;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_at_wins_then_two_point_then_segment_midpoint() {
        let two = [[0.0, 50.0], [100.0, 50.0]];
        assert_eq!(label_point(&Hints::default(), &two), [50.0, 40.0]);
        assert_eq!(label_point(&Hints { dx: Some(3.0), dy: Some(-2.0), ..Hints::default() }, &two), [53.0, 38.0]);
        assert_eq!(label_point(&Hints { at: Some([7.0, 8.0]), ..Hints::default() }, &two), [7.0, 8.0]);
        let poly = [[0.0, 0.0], [10.0, 0.0], [10.0, 40.0], [60.0, 40.0]];
        // Segment 1 by default: (10,0)-(10,40).
        assert_eq!(label_point(&Hints::default(), &poly), [10.0, 10.0]);
        assert_eq!(label_point(&Hints { segment: Some(2.0), ..Hints::default() }, &poly), [35.0, 30.0]);
        // Clamped to the last segment, and to the first.
        assert_eq!(label_point(&Hints { segment: Some(9.0), ..Hints::default() }, &poly), [35.0, 30.0]);
        assert_eq!(label_point(&Hints { segment: Some(-4.0), ..Hints::default() }, &poly), [5.0, -10.0]);
        // A three-point route takes segment 1, the second.
        let three = [[0.0, 0.0], [10.0, 0.0], [10.0, 20.0]];
        assert_eq!(label_point(&Hints::default(), &three), [10.0, 0.0]);
    }

    #[test]
    fn dataflow_box_grows_with_the_classification_line() {
        let plain = dataflow_label(&Hints::default(), &[[0.0, 20.0], [100.0, 20.0]], "OrderPlaced", None);
        assert_eq!((plain.rect.height, plain.classified), (16.0, false));
        let two = dataflow_label(&Hints::default(), &[[0.0, 20.0], [100.0, 20.0]], "OrderPlaced", Some("schema v1"));
        assert_eq!((two.rect.height, two.classified), (27.0, true));
        // 11 units * 4.9 + 12 = 65.9
        assert_eq!(two.rect.width, 65.9);
        assert_eq!(two.rect.x, 50.0 - 65.9 / 2.0);
        assert_eq!(two.rect.y, 10.0 - 11.0);
    }
}
