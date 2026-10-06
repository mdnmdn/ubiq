//! Rects, segments and their predicates: port of the primitives in
//! `renderers/shared/geometry.mjs` (`rectsOverlap` … `segmentsIntersect`).
//!
//! Two epsilons, both Archify's: [`NUMERIC_CLEARANCE_PX`] (`1e-4`) for rect separation, and the
//! `1e-4` orientation/collinearity tolerance for segments. Pure `f64`.
//!
//! Negative-gap convention: `rects_overlap(a, b, -2)` shrinks the hit box, so rects must overlap by
//! *more* than 2 px to count. That is the sign the label-collision rules rely on.

/// A point. Archify writes points as `[x, y]`.
pub type Pt = [f64; 2];

/// The numeric tolerance Archify's route and layout geometry compares with (`0.0001`).
pub const NUMERIC_CLEARANCE_PX: f64 = 0.0001;

/// Threshold below which a segment is treated as a point (`0.0000001`).
const DEGENERATE: f64 = 0.000_000_1;

/// An axis-aligned rect, the `{x, y, width, height, cx, cy}` Archify's renderers pass around.
/// `cx`/`cy` are derived, `x + width / 2`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// A rect from its centre; `cx()`/`cy()` return the arguments exactly for even sizes.
    pub fn from_center(cx: f64, cy: f64, width: f64, height: f64) -> Self {
        Self {
            x: cx - width / 2.0,
            y: cy - height / 2.0,
            width,
            height,
        }
    }

    pub fn cx(&self) -> f64 {
        self.x + self.width / 2.0
    }

    pub fn cy(&self) -> f64 {
        self.y + self.height / 2.0
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// All four numbers are finite (`isFinitePoint`).
    pub fn is_finite(&self) -> bool {
        is_finite_point(&[self.x, self.y, self.width, self.height])
    }

    /// The rect grown by `gap` on every side (shrunk when negative).
    pub fn expanded(&self, gap: f64) -> Rect {
        Rect::new(
            self.x - gap,
            self.y - gap,
            self.width + 2.0 * gap,
            self.height + 2.0 * gap,
        )
    }
}

/// A segment, `{start, end}`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub start: Pt,
    pub end: Pt,
}

impl Seg {
    pub const fn new(start: Pt, end: Pt) -> Self {
        Self { start, end }
    }

    pub fn length(&self) -> f64 {
        (self.end[0] - self.start[0]).hypot(self.end[1] - self.start[1])
    }

    pub fn reversed(&self) -> Seg {
        Seg {
            start: self.end,
            end: self.start,
        }
    }
}

/// `isFinitePoint`: every coordinate is a finite number.
pub fn is_finite_point(coords: &[f64]) -> bool {
    coords.iter().all(|c| c.is_finite())
}

/// `rectsOverlap(a, b, gap)`. Edge-touching is not overlap at `gap = 0`; a separation short by
/// float noise (`< 1e-4`) is met; non-finite geometry means "unknown", never "overlapping".
pub fn rects_overlap(a: &Rect, b: &Rect, gap: f64) -> bool {
    if !(a.is_finite() && b.is_finite()) {
        return false;
    }
    !(a.x + a.width + gap <= b.x + NUMERIC_CLEARANCE_PX
        || b.x + b.width + gap <= a.x + NUMERIC_CLEARANCE_PX
        || a.y + a.height + gap <= b.y + NUMERIC_CLEARANCE_PX
        || b.y + b.height + gap <= a.y + NUMERIC_CLEARANCE_PX)
}

/// `segmentIntersectsRect(segment, rect, gap)`: the rect is expanded by `gap` first.
pub fn segment_intersects_rect(seg: &Seg, rect: &Rect, gap: f64) -> bool {
    let (x1, y1, x2, y2) = (
        rect.x - gap,
        rect.y - gap,
        rect.x + rect.width + gap,
        rect.y + rect.height + gap,
    );
    let (a, b) = (seg.start, seg.end);
    if point_in_box(a, x1, y1, x2, y2) || point_in_box(b, x1, y1, x2, y2) {
        return true;
    }
    segments_intersect(a, b, [x1, y1], [x2, y1])
        || segments_intersect(a, b, [x2, y1], [x2, y2])
        || segments_intersect(a, b, [x2, y2], [x1, y2])
        || segments_intersect(a, b, [x1, y2], [x1, y1])
}

/// `segmentRectClearance`: distance from the segment to the rect, `0` when they touch; `None` for
/// non-finite or negative-sized input (JS `null`).
pub fn segment_rect_clearance(seg: &Seg, rect: &Rect) -> Option<f64> {
    let (start, end) = (seg.start, seg.end);
    if !is_finite_point(&[
        start[0],
        start[1],
        end[0],
        end[1],
        rect.x,
        rect.y,
        rect.width,
        rect.height,
    ]) {
        return None;
    }
    if rect.width < 0.0 || rect.height < 0.0 {
        return None;
    }
    if segment_intersects_rect(seg, rect, 0.0) {
        return Some(0.0);
    }
    let (right, bottom) = (rect.right(), rect.bottom());
    let mut clearance = point_rect_distance(start, rect).min(point_rect_distance(end, rect));
    let corners = [
        [rect.x, rect.y],
        [right, rect.y],
        [right, bottom],
        [rect.x, bottom],
    ];
    for corner in corners {
        let distance = point_segment_distance(corner, start, end);
        if distance < clearance {
            clearance = distance;
        }
    }
    Some(clearance)
}

/// `segmentRectClearanceWithin`: as [`segment_rect_clearance`], but returns `+inf` without computing
/// the distance when an axis gap alone already exceeds `limit`.
pub fn segment_rect_clearance_within(seg: &Seg, rect: &Rect, limit: f64) -> Option<f64> {
    let (start, end) = (seg.start, seg.end);
    if !is_finite_point(&[
        start[0],
        start[1],
        end[0],
        end[1],
        rect.x,
        rect.y,
        rect.width,
        rect.height,
    ]) {
        return None;
    }
    if rect.width < 0.0 || rect.height < 0.0 {
        return None;
    }
    if segment_intersects_rect(seg, rect, 0.0) {
        return Some(0.0);
    }
    let (min_x, max_x) = (start[0].min(end[0]), start[0].max(end[0]));
    let (min_y, max_y) = (start[1].min(end[1]), start[1].max(end[1]));
    let gap_x = (rect.x - max_x).max(min_x - rect.right());
    let gap_y = (rect.y - max_y).max(min_y - rect.bottom());
    if gap_x.max(gap_y) > limit {
        return Some(f64::INFINITY);
    }
    segment_rect_clearance(seg, rect)
}

/// `segmentRectIntersectionLength`: length of the segment inside the rect (Liang–Barsky clip).
/// `None` for non-finite or negative-sized input.
pub fn segment_rect_intersection_length(seg: &Seg, rect: &Rect) -> Option<f64> {
    let (start, end) = (seg.start, seg.end);
    if !is_finite_point(&[
        start[0],
        start[1],
        end[0],
        end[1],
        rect.x,
        rect.y,
        rect.width,
        rect.height,
    ]) {
        return None;
    }
    if rect.width < 0.0 || rect.height < 0.0 {
        return None;
    }
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let length = dx.hypot(dy);
    if length <= DEGENERATE {
        return Some(0.0);
    }
    let bounds = [
        (-dx, start[0] - rect.x),
        (dx, rect.x + rect.width - start[0]),
        (-dy, start[1] - rect.y),
        (dy, rect.y + rect.height - start[1]),
    ];
    let (mut enter, mut leave) = (0.0f64, 1.0f64);
    for (direction, distance) in bounds {
        if direction.abs() <= DEGENERATE {
            if distance < -DEGENERATE {
                return Some(0.0);
            }
            continue;
        }
        let ratio = distance / direction;
        if direction < 0.0 {
            enter = enter.max(ratio);
        } else {
            leave = leave.min(ratio);
        }
        if enter > leave + DEGENERATE {
            return Some(0.0);
        }
    }
    Some(length * (leave - enter).max(0.0))
}

/// `pointRectDistance`.
pub fn point_rect_distance(point: Pt, rect: &Rect) -> f64 {
    let dx = (rect.x - point[0]).max(0.0).max(point[0] - rect.right());
    let dy = (rect.y - point[1]).max(0.0).max(point[1] - rect.bottom());
    dx.hypot(dy)
}

/// `pointSegmentDistance` / `pointSegmentDistanceXY`.
pub fn point_segment_distance(point: Pt, start: Pt, end: Pt) -> f64 {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let length_squared = dx * dx + dy * dy;
    if length_squared <= DEGENERATE {
        return (point[0] - start[0]).hypot(point[1] - start[1]);
    }
    let t = (((point[0] - start[0]) * dx + (point[1] - start[1]) * dy) / length_squared)
        .clamp(0.0, 1.0);
    (point[0] - (start[0] + t * dx)).hypot(point[1] - (start[1] + t * dy))
}

/// `crossProduct(a, b, c)`: `(b - a) x (c - a)`.
pub fn cross_product(a: Pt, b: Pt, c: Pt) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// `collinearForward(a, b, c)`: `b` lies on the line `a → c` and the path does not fold back at `b`.
pub fn collinear_forward(a: Pt, b: Pt, c: Pt) -> bool {
    if cross_product(a, b, c).abs() > NUMERIC_CLEARANCE_PX {
        return false;
    }
    (b[0] - a[0]) * (c[0] - b[0]) + (b[1] - a[1]) * (c[1] - b[1]) >= -NUMERIC_CLEARANCE_PX
}

/// `properSegmentIntersection`: the crossing point of two segments that cross strictly inside both
/// (touching, T-junctions and collinear overlap are `None`).
pub fn proper_segment_intersection(a: Pt, b: Pt, c: Pt, d: Pt) -> Option<Pt> {
    let ab_c = cross_product(a, b, c);
    let ab_d = cross_product(a, b, d);
    let cd_a = cross_product(c, d, a);
    let cd_b = cross_product(c, d, b);
    let eps = NUMERIC_CLEARANCE_PX;
    let opposite = |l: f64, r: f64| (l > eps && r < -eps) || (l < -eps && r > eps);
    if !opposite(ab_c, ab_d) || !opposite(cd_a, cd_b) {
        return None;
    }
    let denominator = (a[0] - b[0]) * (c[1] - d[1]) - (a[1] - b[1]) * (c[0] - d[0]);
    if denominator.abs() < eps {
        return None;
    }
    let ab = a[0] * b[1] - a[1] * b[0];
    let cd = c[0] * d[1] - c[1] * d[0];
    Some([
        (ab * (c[0] - d[0]) - (a[0] - b[0]) * cd) / denominator,
        (ab * (c[1] - d[1]) - (a[1] - b[1]) * cd) / denominator,
    ])
}

/// The overlap of two collinear axis-aligned segments (`collinearAxisOverlap`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisOverlap {
    pub length: f64,
    pub start: Pt,
    pub end: Pt,
}

/// `collinearAxisOverlap(a, b, c, d)`: `None` unless both segments are horizontal (or both vertical)
/// on the same line and overlap by more than `1e-4`.
pub fn collinear_axis_overlap(a: Pt, b: Pt, c: Pt, d: Pt) -> Option<AxisOverlap> {
    let eps = NUMERIC_CLEARANCE_PX;
    let horizontal =
        (a[1] - b[1]).abs() <= eps && (c[1] - d[1]).abs() <= eps && (a[1] - c[1]).abs() <= eps;
    let vertical =
        (a[0] - b[0]).abs() <= eps && (c[0] - d[0]).abs() <= eps && (a[0] - c[0]).abs() <= eps;
    if !horizontal && !vertical {
        return None;
    }
    let axis = if horizontal { 0 } else { 1 };
    let low = a[axis].min(b[axis]).max(c[axis].min(d[axis]));
    let high = a[axis].max(b[axis]).min(c[axis].max(d[axis]));
    if high - low <= eps {
        return None;
    }
    let fixed = if horizontal { a[1] } else { a[0] };
    Some(if horizontal {
        AxisOverlap {
            length: high - low,
            start: [low, fixed],
            end: [high, fixed],
        }
    } else {
        AxisOverlap {
            length: high - low,
            start: [fixed, low],
            end: [fixed, high],
        }
    })
}

/// `segmentsIntersect(a, b, c, d)`: closed-segment intersection with the `1e-4` orientation band.
pub fn segments_intersect(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let o1 = orientation(a, b, c);
    let o2 = orientation(a, b, d);
    let o3 = orientation(c, d, a);
    let o4 = orientation(c, d, b);
    if o1 == 0 && on_segment(a, c, b) {
        return true;
    }
    if o2 == 0 && on_segment(a, d, b) {
        return true;
    }
    if o3 == 0 && on_segment(c, a, d) {
        return true;
    }
    if o4 == 0 && on_segment(c, b, d) {
        return true;
    }
    o1 != o2 && o3 != o4
}

fn orientation(a: Pt, b: Pt, c: Pt) -> u8 {
    let value = (b[1] - a[1]) * (c[0] - b[0]) - (b[0] - a[0]) * (c[1] - b[1]);
    if value.abs() < NUMERIC_CLEARANCE_PX {
        0
    } else if value > 0.0 {
        1
    } else {
        2
    }
}

fn on_segment(a: Pt, b: Pt, c: Pt) -> bool {
    b[0] <= a[0].max(c[0])
        && b[0] >= a[0].min(c[0])
        && b[1] <= a[1].max(c[1])
        && b[1] >= a[1].min(c[1])
}

fn point_in_box(p: Pt, x1: f64, y1: f64, x2: f64, y2: f64) -> bool {
    p[0] >= x1 && p[0] <= x2 && p[1] >= y1 && p[1] <= y2
}

/// Which border of a frame a [`BorderSegment`] runs along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderSide {
    Top,
    Right,
    Bottom,
    Left,
    Line,
}

/// A frame (boundary, group or phase) as `frameBorderSegments` reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Frame {
    /// `shape: 'line'`, a single rule.
    Line { start: Pt, end: Pt },
    /// A rounded rect; `radius` is clamped to half the shorter side.
    Rect { rect: Rect, radius: f64 },
}

/// One straight stretch of a frame border (corners excluded).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderSegment {
    pub side: BorderSide,
    pub start: Pt,
    pub end: Pt,
}

/// `frameBorderSegments(frame)`: the four straight sides between the corner arcs (clockwise from
/// the top), dropping any shorter than `1e-4`; a line frame is one segment. Invalid frames give `[]`.
pub fn frame_border_segments(frame: &Frame) -> Vec<BorderSegment> {
    match *frame {
        Frame::Line { start, end } => {
            if is_finite_point(&[start[0], start[1], end[0], end[1]]) {
                vec![BorderSegment {
                    side: BorderSide::Line,
                    start,
                    end,
                }]
            } else {
                Vec::new()
            }
        }
        Frame::Rect { rect, radius } => {
            if !rect.is_finite() || rect.width <= 0.0 || rect.height <= 0.0 {
                return Vec::new();
            }
            let radius = if radius.is_nan() { 0.0 } else { radius };
            let radius = 0.0f64.max(radius.min(rect.width / 2.0).min(rect.height / 2.0));
            let (left, right, top, bottom) = (rect.x, rect.right(), rect.y, rect.bottom());
            [
                (BorderSide::Top, [left + radius, top], [right - radius, top]),
                (
                    BorderSide::Right,
                    [right, top + radius],
                    [right, bottom - radius],
                ),
                (
                    BorderSide::Bottom,
                    [right - radius, bottom],
                    [left + radius, bottom],
                ),
                (
                    BorderSide::Left,
                    [left, bottom - radius],
                    [left, top + radius],
                ),
            ]
            .into_iter()
            .filter(|(_, s, e)| (e[0] - s[0]).hypot(e[1] - s[1]) > NUMERIC_CLEARANCE_PX)
            .map(|(side, start, end)| BorderSegment { side, start, end })
            .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn rects_overlap_separated() {
        assert!(!rects_overlap(
            &rect(0., 0., 10., 10.),
            &rect(20., 0., 10., 10.),
            0.0
        ));
    }

    #[test]
    fn rects_overlap_clearly() {
        assert!(rects_overlap(
            &rect(0., 0., 10., 10.),
            &rect(5., 5., 10., 10.),
            0.0
        ));
    }

    #[test]
    fn rects_overlap_edge_touching_is_not_overlap() {
        assert!(!rects_overlap(
            &rect(0., 0., 10., 10.),
            &rect(10., 0., 10., 10.),
            0.0
        ));
    }

    #[test]
    fn rects_overlap_positive_gap() {
        assert!(!rects_overlap(
            &rect(0., 0., 10., 10.),
            &rect(18., 0., 10., 10.),
            8.0
        ));
        assert!(rects_overlap(
            &rect(0., 0., 10., 10.),
            &rect(17., 0., 10., 10.),
            8.0
        ));
    }

    #[test]
    fn rects_overlap_one_ulp_shortfall_is_met() {
        let left = rect(873.6 - 80., 93., 160., 52.);
        let right = rect(1041.6 - 80., 93., 160., 52.);
        assert!(
            left.x + left.width + 8. > right.x,
            "must reach the check as a float shortfall"
        );
        assert!(!rects_overlap(&left, &right, 8.0));
        assert!(!rects_overlap(&right, &left, 8.0));
        let above = rect(93., 873.6 - 80., 52., 160.);
        let below = rect(93., 1041.6 - 80., 52., 160.);
        assert!(above.y + above.height + 8. > below.y);
        assert!(!rects_overlap(&above, &below, 8.0));
        assert!(!rects_overlap(&below, &above, 8.0));
    }

    #[test]
    fn rects_overlap_tolerance_stays_far_below_a_repairable_shortfall() {
        assert!(rects_overlap(
            &rect(0., 0., 160., 52.),
            &rect(167.999, 0., 160., 52.),
            8.0
        ));
        assert!(rects_overlap(
            &rect(0., 0., 160., 52.),
            &rect(167.9, 0., 160., 52.),
            8.0
        ));
        assert!(!rects_overlap(
            &rect(0., 0., 160., 52.),
            &rect(168., 0., 160., 52.),
            8.0
        ));
    }

    #[test]
    fn rects_overlap_negative_gap_shrinks_the_hit_box() {
        assert!(!rects_overlap(
            &rect(0., 0., 10., 10.),
            &rect(9., 0., 10., 10.),
            -2.0
        ));
        assert!(rects_overlap(
            &rect(0., 0., 10., 10.),
            &rect(7., 0., 10., 10.),
            -2.0
        ));
    }

    #[test]
    fn rects_overlap_non_finite_is_not_overlap() {
        let nan = rect(f64::NAN, f64::NAN, 120., 60.);
        assert!(!rects_overlap(&nan, &nan, 8.0));
        assert!(!rects_overlap(&nan, &rect(0., 0., 10., 10.), 8.0));
        assert!(!rects_overlap(&rect(0., 0., 10., 10.), &nan, 8.0));
        assert!(!rects_overlap(
            &rect(0., 0., 10., 10.),
            &rect(20., 0., f64::NAN, 10.),
            0.0
        ));
        assert!(!rects_overlap(
            &rect(0., 0., 10., 10.),
            &rect(5., 5., 10., f64::INFINITY),
            0.0
        ));
    }

    #[test]
    fn is_finite_point_rejects_nan_and_infinity() {
        assert!(is_finite_point(&[1., 2., 3., 4.]));
        assert!(!is_finite_point(&[1., f64::NAN]));
        assert!(!is_finite_point(&[1., f64::INFINITY]));
    }

    #[test]
    fn segment_intersects_rect_detects_an_edge_crossing_a_node_box() {
        let b = rect(8., 0., 4., 10.);
        assert!(segment_intersects_rect(
            &Seg::new([0., 5.], [20., 5.]),
            &b,
            0.0
        ));
        assert!(!segment_intersects_rect(
            &Seg::new([0., 20.], [20., 20.]),
            &b,
            0.0
        ));
    }

    #[test]
    fn segment_intersects_rect_gap_expands_the_box() {
        let b = rect(8., 0., 4., 10.);
        let s = Seg::new([0., 12.], [20., 12.]);
        assert!(!segment_intersects_rect(&s, &b, 1.0));
        assert!(segment_intersects_rect(&s, &b, 2.0));
    }

    #[test]
    fn segment_rect_clearance_measures_horizontal_vertical_and_reversed_diagonals() {
        let b = rect(10., 10., 10., 10.);
        assert_eq!(
            segment_rect_clearance(&Seg::new([0., 6.], [30., 6.]), &b),
            Some(4.0)
        );
        assert_eq!(
            segment_rect_clearance(&Seg::new([6., 0.], [6., 30.]), &b),
            Some(4.0)
        );
        assert_eq!(
            segment_rect_clearance(&Seg::new([0., 0.], [8., 8.]), &b),
            Some(8.0f64.sqrt())
        );
        assert_eq!(
            segment_rect_clearance(&Seg::new([8., 8.], [0., 0.]), &b),
            Some(8.0f64.sqrt())
        );
        assert_eq!(
            segment_rect_clearance(&Seg::new([0., 15.], [30., 15.]), &b),
            Some(0.0)
        );
    }

    #[test]
    fn label_route_clearance_locks_tangent_sub_threshold_boundary_and_reversed() {
        let b = rect(10., 10., 10., 10.);
        let cases: [(Seg, f64, f64); 9] = [
            (Seg::new([0., 10.], [30., 10.]), 0.0, 10.0),
            (Seg::new([10., 0.], [10., 30.]), 0.0, 10.0),
            (Seg::new([0., 0.], [30., 30.]), 0.0, 200.0f64.sqrt()),
            (Seg::new([0., 0.], [10., 10.]), 0.0, 0.0),
            (Seg::new([0., 8.1], [30., 8.1]), 1.9, 0.0),
            (Seg::new([0., 8.], [30., 8.]), 2.0, 0.0),
            (Seg::new([0., 6.1], [30., 6.1]), 3.9, 0.0),
            (Seg::new([0., 6.], [30., 6.]), 4.0, 0.0),
            (Seg::new([0., 0.], [5., 0.]), 125.0f64.sqrt(), 0.0),
        ];
        for (seg, clearance, intersection) in cases {
            for s in [seg, seg.reversed()] {
                assert!(
                    (segment_rect_clearance(&s, &b).unwrap() - clearance).abs() < 0.000001,
                    "{s:?}"
                );
                assert!(
                    (segment_rect_intersection_length(&s, &b).unwrap() - intersection).abs()
                        < 0.000001,
                    "{s:?}"
                );
            }
        }
    }

    #[test]
    fn near_zero_segments_preserve_the_label_clearance_threshold() {
        let q = 100.0 - 3.999899999 / 2.0f64.sqrt();
        let seg = Seg::new([q - 0.0001, q + 0.0001], [q + 0.0001, q - 0.0001]);
        let b = rect(100., 100., 20., 20.);
        assert!(segment_rect_clearance(&seg, &b).unwrap() + 0.0001 >= 4.0);
        assert!(segment_rect_clearance(&seg.reversed(), &b).unwrap() + 0.0001 >= 4.0);
        // A true point measures as a point.
        let p = Seg::new([100., 0.], [100., 0.]);
        assert_eq!(segment_rect_clearance(&p, &b), Some(100.0));
        assert_eq!(segment_rect_intersection_length(&p, &b), Some(0.0));
    }

    #[test]
    fn clearance_within_short_circuits_on_an_axis_gap() {
        let b = rect(10., 10., 10., 10.);
        let far = Seg::new([0., 0.], [5., 0.]);
        assert_eq!(
            segment_rect_clearance_within(&far, &b, 4.0),
            Some(f64::INFINITY)
        );
        assert_eq!(
            segment_rect_clearance_within(&far, &b, 20.0),
            segment_rect_clearance(&far, &b)
        );
        assert_eq!(
            segment_rect_clearance_within(&Seg::new([0., 15.], [30., 15.]), &b, 0.0),
            Some(0.0)
        );
    }

    #[test]
    fn clearance_rejects_bad_input() {
        let s = Seg::new([0., 0.], [5., 0.]);
        assert_eq!(segment_rect_clearance(&s, &rect(0., 0., -1., 5.)), None);
        assert_eq!(
            segment_rect_clearance(&s, &rect(f64::NAN, 0., 1., 5.)),
            None
        );
        assert_eq!(
            segment_rect_intersection_length(&s, &rect(0., 0., 1., -5.)),
            None
        );
    }

    #[test]
    fn proper_intersection_is_strict() {
        let x = proper_segment_intersection([0., 5.], [10., 5.], [5., 0.], [5., 10.]);
        assert_eq!(x, Some([5.0, 5.0]));
        // T-touch and collinear overlap are not proper crossings.
        assert_eq!(
            proper_segment_intersection([0., 5.], [10., 5.], [5., 5.], [5., 10.]),
            None
        );
        assert_eq!(
            proper_segment_intersection([0., 5.], [10., 5.], [5., 5.], [15., 5.]),
            None
        );
    }

    #[test]
    fn collinear_overlap_needs_the_same_axis_line() {
        let o = collinear_axis_overlap([0., 5.], [10., 5.], [4., 5.], [20., 5.]).unwrap();
        assert_eq!((o.length, o.start, o.end), (6.0, [4., 5.], [10., 5.]));
        let v = collinear_axis_overlap([5., 0.], [5., 10.], [5., 6.], [5., 2.]).unwrap();
        assert_eq!((v.length, v.start, v.end), (4.0, [5., 2.], [5., 6.]));
        assert!(collinear_axis_overlap([0., 5.], [10., 5.], [10., 5.], [20., 5.]).is_none());
        assert!(collinear_axis_overlap([0., 5.], [10., 5.], [4., 6.], [20., 6.]).is_none());
    }

    #[test]
    fn frame_border_segments_skip_corners_and_clamp_radius() {
        let f = Frame::Rect {
            rect: rect(0., 0., 100., 40.),
            radius: 10.,
        };
        let s = frame_border_segments(&f);
        assert_eq!(s.len(), 4);
        assert_eq!(
            (s[0].side, s[0].start, s[0].end),
            (BorderSide::Top, [10., 0.], [90., 0.])
        );
        assert_eq!(
            (s[1].side, s[1].start, s[1].end),
            (BorderSide::Right, [100., 10.], [100., 30.])
        );
        // radius 20 = half the height: the vertical sides vanish.
        let pill = Frame::Rect {
            rect: rect(0., 0., 100., 40.),
            radius: 99.,
        };
        assert_eq!(frame_border_segments(&pill).len(), 2);
        assert!(
            frame_border_segments(&Frame::Rect {
                rect: rect(0., 0., 0., 40.),
                radius: 0.
            })
            .is_empty()
        );
        let line = Frame::Line {
            start: [0., 1.],
            end: [5., 1.],
        };
        assert_eq!(frame_border_segments(&line)[0].side, BorderSide::Line);
    }
}
