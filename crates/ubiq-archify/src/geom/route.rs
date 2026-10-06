//! Polylines: `normalize`, the rounded-corner path and its flattening, arclength sampling.
//!
//! Ports `normalizeRoutePoints`, `joinRoutePoints`, `polylinePath` and `roundedPath` of
//! `renderers/shared/geometry.mjs`. Archify never flattens a corner itself: the SVG keeps the `Q`
//! command. A native painter needs points, so [`flatten_cmds`] subdivides each `Q` uniformly in `t`,
//! and [`sample_polyline`] reproduces the one place Archify does sample, the exporter's
//! `getPointAtLength` walk (`viewer/export.js` `samplesFor`: `count = clamp(ceil(L / 12), 12, 72)`,
//! `count + 1` points equally spaced by arclength).

use super::rect::{NUMERIC_CLEARANCE_PX, Pt, collinear_forward, is_finite_point};

/// `normalizeRoutePoints`: drop non-finite points, points within `1e-4` of their predecessor on both
/// axes, and middle points that are *forward-collinear* (a fold-back keeps its vertex).
pub fn normalize(points: &[Pt]) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(points.len());
    for &p in points {
        accept(&mut out, p);
    }
    out
}

/// `joinRoutePoints(start, via, end)`: `normalize([start, ...via, end])` without the temporary.
pub fn join_route_points(start: Pt, via: &[Pt], end: Pt) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(via.len() + 2);
    accept(&mut out, start);
    for &p in via {
        accept(&mut out, p);
    }
    accept(&mut out, end);
    out
}

fn accept(out: &mut Vec<Pt>, p: Pt) {
    if !is_finite_point(&p) {
        return;
    }
    if let Some(prev) = out.last()
        && (p[0] - prev[0]).abs() <= NUMERIC_CLEARANCE_PX
        && (p[1] - prev[1]).abs() <= NUMERIC_CLEARANCE_PX
    {
        return;
    }
    while out.len() >= 2 && collinear_forward(out[out.len() - 2], out[out.len() - 1], p) {
        out.pop();
    }
    out.push(p);
}

/// One SVG path command, as `roundedPath` emits them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCmd {
    M(Pt),
    L(Pt),
    /// Quadratic Bezier: control point (the polyline vertex), then end point.
    Q(Pt, Pt),
}

/// The commands of `polylinePath(points)`: `M` then `L` per point.
pub fn polyline_cmds(points: &[Pt]) -> Vec<PathCmd> {
    points
        .iter()
        .enumerate()
        .map(|(i, &p)| if i == 0 { PathCmd::M(p) } else { PathCmd::L(p) })
        .collect()
}

/// The commands of `roundedPath(points, radius)`. Fewer than 3 points or `radius <= 0` degrade to a
/// plain polyline; per interior vertex `r' = min(radius, prevLen / 2, nextLen / 2)`, and `r' < 1`
/// keeps a sharp `L`, otherwise `L before` + `Q vertex after`.
pub fn rounded_cmds(points: &[Pt], radius: f64) -> Vec<PathCmd> {
    if points.len() < 3 || radius <= 0.0 {
        return polyline_cmds(points);
    }
    let mut cmds = vec![PathCmd::M(points[0])];
    for i in 1..points.len() - 1 {
        let [px, py] = points[i - 1];
        let [cx, cy] = points[i];
        let [nx, ny] = points[i + 1];
        let prev_len = (cx - px).hypot(cy - py);
        let next_len = (nx - cx).hypot(ny - cy);
        let r = radius.min(prev_len / 2.0).min(next_len / 2.0);
        if r < 1.0 {
            cmds.push(PathCmd::L([cx, cy]));
            continue;
        }
        let before = [
            cx - ((cx - px) / prev_len) * r,
            cy - ((cy - py) / prev_len) * r,
        ];
        let after = [
            cx + ((nx - cx) / next_len) * r,
            cy + ((ny - cy) / next_len) * r,
        ];
        cmds.push(PathCmd::L(before));
        cmds.push(PathCmd::Q([cx, cy], after));
    }
    cmds.push(PathCmd::L(points[points.len() - 1]));
    cmds
}

/// JavaScript's number-to-string for the values a path carries: integers without `.0`, otherwise
/// the shortest round-trip form (`f64` `Display` agrees with JS outside `|x| < 1e-6` or `>= 1e21`).
fn num(x: f64) -> String {
    if x == 0.0 {
        "0".to_string()
    } else {
        format!("{x}")
    }
}

/// The `d` attribute for a command list (`M x y L x y Q cx cy x y`).
pub fn path_d(cmds: &[PathCmd]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(cmds.len());
    for cmd in cmds {
        parts.push(match *cmd {
            PathCmd::M([x, y]) => format!("M {} {}", num(x), num(y)),
            PathCmd::L([x, y]) => format!("L {} {}", num(x), num(y)),
            PathCmd::Q([cx, cy], [x, y]) => {
                format!("Q {} {} {} {}", num(cx), num(cy), num(x), num(y))
            }
        });
    }
    parts.join(" ")
}

/// `polylinePath(points)`.
pub fn polyline_path(points: &[Pt]) -> String {
    path_d(&polyline_cmds(points))
}

/// `roundedPath(points, radius)`.
pub fn rounded_path(points: &[Pt], radius: f64) -> String {
    path_d(&rounded_cmds(points, radius))
}

/// Points per quadratic corner used by [`flatten_rounded`]; a corner of radius 8 or 10 deviates from
/// the true curve by well under 0.01 px at this count.
pub const CORNER_STEPS: usize = 8;

/// Flatten commands to a polyline: `M`/`L` pass through, each `Q` becomes `steps` points at
/// `t = i / steps` (the end point included, the start already present).
pub fn flatten_cmds(cmds: &[PathCmd], steps: usize) -> Vec<Pt> {
    let steps = steps.max(1);
    let mut out: Vec<Pt> = Vec::with_capacity(cmds.len() + steps);
    for cmd in cmds {
        match *cmd {
            PathCmd::M(p) | PathCmd::L(p) => out.push(p),
            PathCmd::Q(c, e) => {
                let s = *out.last().unwrap_or(&c);
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let u = 1.0 - t;
                    out.push([
                        u * u * s[0] + 2.0 * u * t * c[0] + t * t * e[0],
                        u * u * s[1] + 2.0 * u * t * c[1] + t * t * e[1],
                    ]);
                }
            }
        }
    }
    out
}

/// `roundedPath` as points: [`rounded_cmds`] then [`flatten_cmds`] with [`CORNER_STEPS`].
pub fn flatten_rounded(points: &[Pt], radius: f64) -> Vec<Pt> {
    flatten_cmds(&rounded_cmds(points, radius), CORNER_STEPS)
}

/// Total length of a polyline.
pub fn polyline_length(points: &[Pt]) -> f64 {
    points
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum()
}

/// The exporter's sample count for a path of `length` px: `clamp(ceil(length / 12), 12, 72)`.
pub fn sample_count(length: f64) -> usize {
    (length / 12.0).ceil().clamp(12.0, 72.0) as usize
}

/// `count + 1` points equally spaced by arclength along `points` (`getPointAtLength(L * i / count)`).
/// Empty for fewer than two points or a zero-length path.
pub fn sample_polyline(points: &[Pt], count: usize) -> Vec<Pt> {
    let total = polyline_length(points);
    if points.len() < 2 || total.is_nan() || total <= 0.0 || count == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(count + 1);
    let (mut seg, mut walked) = (0usize, 0.0f64);
    for i in 0..=count {
        let target = total * i as f64 / count as f64;
        while seg + 2 < points.len() {
            let len =
                (points[seg + 1][0] - points[seg][0]).hypot(points[seg + 1][1] - points[seg][1]);
            if walked + len >= target {
                break;
            }
            walked += len;
            seg += 1;
        }
        let (a, b) = (points[seg], points[seg + 1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let t = if len > 0.0 {
            ((target - walked) / len).clamp(0.0, 1.0)
        } else {
            0.0
        };
        out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
    }
    out
}

/// What the exporter samples for an edge: the rounded path, flattened finely, then walked at
/// [`sample_count`] arclength steps.
pub fn sample_rounded(points: &[Pt], radius: f64) -> Vec<Pt> {
    let flat = flatten_cmds(&rounded_cmds(points, radius), 32);
    sample_polyline(&flat, sample_count(polyline_length(&flat)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polyline_path_emits_m_then_l_commands() {
        assert_eq!(
            polyline_path(&[[0., 0.], [10., 0.], [10., 10.]]),
            "M 0 0 L 10 0 L 10 10"
        );
    }

    #[test]
    fn rounded_path_degrades_to_a_polyline_for_short_paths_or_no_radius() {
        assert_eq!(rounded_path(&[[0., 0.], [10., 0.]], 10.0), "M 0 0 L 10 0");
        assert_eq!(
            rounded_path(&[[0., 0.], [10., 0.], [10., 10.]], 0.0),
            "M 0 0 L 10 0 L 10 10"
        );
    }

    #[test]
    fn rounded_path_inserts_a_quadratic_corner_and_never_emits_nan() {
        let d = rounded_path(&[[0., 0.], [100., 0.], [100., 100.]], 10.0);
        assert!(d.contains("Q 100 0"), "{d}");
        assert!(!d.contains("NaN"));
        assert_eq!(d, "M 0 0 L 90 0 Q 100 0 100 10 L 100 100");
    }

    #[test]
    fn rounded_path_clamps_radius_to_half_the_shorter_adjacent_segment() {
        let d = rounded_path(&[[0., 0.], [6., 0.], [6., 6.]], 10.0);
        assert!(!d.contains("NaN"));
        assert!(d.starts_with("M 0 0"));
        assert_eq!(d, "M 0 0 L 3 0 Q 6 0 6 3 L 6 6");
    }

    #[test]
    fn rounded_path_keeps_a_sharp_vertex_below_one_pixel() {
        // r = min(10, 1/2, 50) = 0.5 < 1: plain L.
        assert_eq!(
            rounded_path(&[[0., 0.], [1., 0.], [1., 100.]], 10.0),
            "M 0 0 L 1 0 L 1 100"
        );
    }

    #[test]
    fn normalize_drops_zero_length_and_forward_collinear_points() {
        let n = normalize(&[
            [0., 0.],
            [0., 0.00005],
            [5., 0.],
            [10., 0.],
            [10., 10.],
            [10., 20.],
        ]);
        assert_eq!(n, vec![[0., 0.], [10., 0.], [10., 20.]]);
    }

    #[test]
    fn normalize_keeps_a_fold_back_vertex_and_skips_non_finite() {
        let n = normalize(&[[0., 0.], [10., 0.], [f64::NAN, 1.], [5., 0.]]);
        assert_eq!(n, vec![[0., 0.], [10., 0.], [5., 0.]]);
        assert!(normalize(&[]).is_empty());
    }

    #[test]
    fn join_route_points_is_normalize_of_the_concatenation() {
        let via = [[5., 0.], [10., 0.], [10., 10.]];
        let joined = join_route_points([0., 0.], &via, [10., 20.]);
        assert_eq!(
            joined,
            normalize(&[[0., 0.], [5., 0.], [10., 0.], [10., 10.], [10., 20.]])
        );
        assert_eq!(joined, vec![[0., 0.], [10., 0.], [10., 20.]]);
    }

    #[test]
    fn flatten_keeps_endpoints_and_stays_within_the_corner_box() {
        let pts = flatten_rounded(&[[0., 0.], [100., 0.], [100., 100.]], 10.0);
        assert_eq!(pts.len(), 3 + CORNER_STEPS);
        assert_eq!(pts[0], [0., 0.]);
        assert_eq!(*pts.last().unwrap(), [100., 100.]);
        assert_eq!(pts[1], [90., 0.]);
        assert_eq!(pts[1 + CORNER_STEPS], [100., 10.]);
        // The curve midpoint of a right-angle Q is 0.25 of the way in from the vertex on each axis.
        assert_eq!(pts[1 + CORNER_STEPS / 2], [97.5, 2.5]);
        for p in &pts {
            assert!(p[0] <= 100.0 && p[1] >= 0.0);
        }
    }

    #[test]
    fn flatten_of_a_straight_polyline_is_the_polyline() {
        let p = [[0., 0.], [10., 0.], [10., 10.]];
        assert_eq!(flatten_rounded(&p, 0.0), p.to_vec());
    }

    #[test]
    fn sample_count_follows_the_exporter_clamp() {
        assert_eq!(sample_count(0.0), 12);
        assert_eq!(sample_count(144.0), 12);
        assert_eq!(sample_count(145.0), 13);
        assert_eq!(sample_count(10_000.0), 72);
    }

    #[test]
    fn sample_polyline_is_equal_by_arclength() {
        let s = sample_polyline(&[[0., 0.], [100., 0.], [100., 100.]], 4);
        assert_eq!(
            s,
            vec![[0., 0.], [50., 0.], [100., 0.], [100., 50.], [100., 100.]]
        );
        assert!(sample_polyline(&[[1., 1.]], 4).is_empty());
        assert!(sample_polyline(&[[1., 1.], [1., 1.]], 4).is_empty());
    }

    #[test]
    fn sample_rounded_yields_count_plus_one_points() {
        let s = sample_rounded(&[[0., 0.], [200., 0.], [200., 100.]], 8.0);
        let len = polyline_length(&flatten_cmds(
            &rounded_cmds(&[[0., 0.], [200., 0.], [200., 100.]], 8.0),
            32,
        ));
        assert_eq!(s.len(), sample_count(len) + 1);
        assert_eq!(s[0], [0., 0.]);
        let last = *s.last().unwrap();
        assert!((last[0] - 200.0).abs() < 1e-9 && (last[1] - 100.0).abs() < 1e-9);
    }
}
