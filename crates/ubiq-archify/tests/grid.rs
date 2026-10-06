//! `route::grid`: the ported `route-detour.test.mjs` cases (same numbers), hand-checked small
//! searches, and property tests on random rect fields. Parity with the JS on random inputs is
//! `grid_parity.rs`.

use std::collections::HashSet;

use ubiq_archify::geom::{Pt, Rect, Seg, Side, segment_intersects_rect};
use ubiq_archify::route::grid::{
    DetourRelation, DetourThresholds, GridQuery, GridStatus, clean_route_detour_problems,
    shortest_orthogonal_grid_route, shortest_orthogonal_grid_route_with_metrics,
};

// ---------------------------------------------------------------------------------------------
// Ported: route-detour.test.mjs
// ---------------------------------------------------------------------------------------------

fn component(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect::new(x, y, w, h)
}

fn bottom(r: &Rect) -> Pt {
    [r.cx(), r.bottom()]
}

fn relation(id: &str, from: &str, to: &str, via: &[Pt], start: Pt, end: Pt) -> DetourRelation {
    let mut points = vec![start];
    points.extend_from_slice(via);
    points.push(end);
    DetourRelation {
        id: Some(id.to_string()),
        from: from.to_string(),
        to: to.to_string(),
        has_via: !via.is_empty(),
        points,
        from_side: Some(Side::Bottom),
        to_side: Some(Side::Bottom),
    }
}

fn ids(names: &[&str]) -> HashSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn run(
    relations: &[DetourRelation],
    obstacles: &[Rect],
    names: &[&str],
    showcase: bool,
) -> Vec<ubiq_archify::route::grid::DetourFinding> {
    clean_route_detour_problems(
        relations,
        obstacles,
        None,
        &ids(names),
        showcase,
        &DetourThresholds::default(),
    )
}

fn long_detour() -> (Vec<Rect>, Vec<DetourRelation>) {
    let obstacles = vec![
        component(60.0, 300.0, 150.0, 64.0),
        component(410.0, 300.0, 150.0, 64.0),
        component(290.0, 480.0, 150.0, 64.0),
    ];
    let rel = relation(
        "sources-to-worker",
        "sources",
        "worker",
        &[[135.0, 1120.0], [485.0, 1120.0]],
        bottom(&obstacles[0]),
        bottom(&obstacles[1]),
    );
    (obstacles, vec![rel])
}

const NAMES: [&str; 3] = ["sources", "worker", "cache"];

#[test]
fn grid_showcase_rejects_an_empty_space_detour() {
    let (obstacles, relations) = long_detour();
    let found = run(&relations, &obstacles, &NAMES, true);
    assert_eq!(found.len(), 1);
    let f = &found[0];
    assert_eq!(f.id.as_deref(), Some("sources-to-worker"));
    assert_eq!(f.actual_length, 1862.0);
    assert_eq!(f.shortest_length, 358.0);
    assert_eq!(f.detour_ratio, 1862.0 / 358.0);
    assert_eq!((f.detour_ratio * 100.0).round() / 100.0, 5.2);
    assert_eq!(f.excursion.unwrap().bottom, 576.0);
    assert_eq!(f.empty_clearance.unwrap().maximum, 731.0);
    assert_eq!(
        f.shortest_points,
        vec![
            [135.0, 364.0],
            [135.0, 368.0],
            [485.0, 368.0],
            [485.0, 364.0]
        ]
    );
    assert!(f.message("architecture", "connections").starts_with(
        "[composition/excessive-route-detour] architecture connections[0] id \"sources-to-worker\" \"sources\" -> \"worker\" travels 1862px, 5.2x the 358px"
    ));
}

#[test]
fn grid_unnecessary_corridor_inside_a_large_envelope() {
    let obstacles = vec![
        component(60.0, 100.0, 120.0, 60.0),
        component(410.0, 100.0, 120.0, 60.0),
        component(60.0, 700.0, 120.0, 60.0),
    ];
    let rel = relation(
        "internal-detour",
        "source",
        "target",
        &[[120.0, 500.0], [470.0, 500.0]],
        bottom(&obstacles[0]),
        bottom(&obstacles[1]),
    );
    let found = run(
        &[rel],
        &obstacles,
        &["source", "target", "lower-content"],
        true,
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].excursion.unwrap().maximum, 0.0);
    assert_eq!(found[0].empty_clearance.unwrap().maximum, 340.0);
}

#[test]
fn grid_detour_passes_once_the_via_is_removed() {
    let (obstacles, mut relations) = long_detour();
    relations[0].has_via = false;
    relations[0].points = vec![[135.0, 364.0], [485.0, 364.0]];
    assert!(run(&relations, &obstacles, &NAMES, true).is_empty());
}

#[test]
fn grid_standard_profile_keeps_authored_freedom() {
    let (obstacles, relations) = long_detour();
    assert!(run(&relations, &obstacles, &NAMES, false).is_empty());
}

#[test]
fn grid_route_around_an_opaque_obstacle_is_not_a_detour() {
    let obstacles = vec![
        component(60.0, 100.0, 120.0, 60.0),
        component(240.0, 80.0, 100.0, 100.0),
        component(410.0, 100.0, 120.0, 60.0),
    ];
    let mut rel = relation(
        "around-blocker",
        "source",
        "target",
        &[
            [200.0, 130.0],
            [200.0, 190.0],
            [390.0, 190.0],
            [390.0, 130.0],
        ],
        [180.0, 130.0],
        [410.0, 130.0],
    );
    rel.from_side = Some(Side::Right);
    rel.to_side = Some(Side::Left);
    assert!(run(&[rel], &obstacles, &["source", "blocker", "target"], true).is_empty());
}

#[test]
fn grid_related_shared_outer_corridor_is_a_bus() {
    let obstacles = vec![
        component(60.0, 100.0, 120.0, 60.0),
        component(410.0, 100.0, 120.0, 60.0),
        component(410.0, 260.0, 120.0, 60.0),
    ];
    let via = [[120.0, 700.0], [470.0, 700.0]];
    let first = relation(
        "bus-first",
        "source",
        "first",
        &via,
        bottom(&obstacles[0]),
        bottom(&obstacles[1]),
    );
    let second = relation(
        "bus-second",
        "source",
        "second",
        &via,
        bottom(&obstacles[0]),
        bottom(&obstacles[2]),
    );
    let names = ["source", "first", "second"];
    assert!(run(&[first.clone(), second], &obstacles, &names, true).is_empty());
    // On its own the same corridor is a finding: the bus is what excuses it.
    assert_eq!(run(&[first], &obstacles, &names, true).len(), 1);
}

#[test]
fn grid_direct_relationship_is_not_a_detour() {
    let obstacles = vec![
        component(40.0, 120.0, 120.0, 60.0),
        component(340.0, 120.0, 120.0, 60.0),
    ];
    let mut rel = relation(
        "enter",
        "outside",
        "inside",
        &[],
        [160.0, 150.0],
        [340.0, 150.0],
    );
    rel.from_side = Some(Side::Right);
    rel.to_side = Some(Side::Left);
    assert!(run(&[rel], &obstacles, &["outside", "inside"], true).is_empty());
}

// ---------------------------------------------------------------------------------------------
// Hand-checked searches
// ---------------------------------------------------------------------------------------------

fn query<'a>(
    start: Pt,
    end: Pt,
    points: &'a [Pt],
    obstacles: &'a [Rect],
    from: Side,
    to: Side,
) -> GridQuery<'a> {
    GridQuery::new(start, end, points, obstacles, Some(from), Some(to), 2.0, 80)
}

#[test]
fn grid_open_field_is_a_straight_line() {
    let pts = [[0.0, 0.0], [100.0, 0.0]];
    let q = query(pts[0], pts[1], &pts, &[], Side::Right, Side::Left);
    let route = shortest_orthogonal_grid_route(&q).unwrap();
    assert_eq!(route.points, vec![[0.0, 0.0], [100.0, 0.0]]);
    assert_eq!(route.length, Some(100.0));
}

#[test]
fn grid_detours_around_a_blocker_with_two_bends_under_a_bend_penalty() {
    let pts = [[0.0, 0.0], [200.0, 0.0]];
    let blocker = [Rect::new(80.0, -40.0, 40.0, 80.0)];
    let mut q = query(pts[0], pts[1], &pts, &blocker, Side::Right, Side::Left);
    q.bend_penalty_px = 48.0;
    let route = shortest_orthogonal_grid_route(&q).unwrap();
    // Up and over the expanded blocker (top -42, line at -43). Turning up at x = 4 (the stub
    // line) or at x = 77 (the blocker's left line, 80 - 2 - 1) costs the same; the neighbour
    // order (left, right, up, down) and the strict relaxation pick 77, as the JS does.
    assert_eq!(
        route.points,
        vec![
            [0.0, 0.0],
            [77.0, 0.0],
            [77.0, -43.0],
            [196.0, -43.0],
            [196.0, 0.0],
            [200.0, 0.0]
        ]
    );
}

#[test]
fn grid_budgets_and_unsupported_sides_fail_with_a_status() {
    let pts = [[0.0, 0.0], [100.0, 0.0]];
    let many: Vec<Rect> = (0..5)
        .map(|i| Rect::new(10.0 * i as f64, 50.0, 5.0, 5.0))
        .collect();
    let mut q = query(pts[0], pts[1], &pts, &many, Side::Right, Side::Left);
    q.maximum_obstacle_count = 4;
    let out = shortest_orthogonal_grid_route_with_metrics(&q);
    assert_eq!(out.metrics.status, GridStatus::ObstacleBudgetExceeded);
    assert!(out.route.is_none());

    let mut q = query(pts[0], pts[1], &pts, &many, Side::Right, Side::Left);
    q.maximum_grid_nodes = Some(4);
    assert_eq!(
        shortest_orthogonal_grid_route_with_metrics(&q)
            .metrics
            .status,
        GridStatus::NodeBudgetExceeded
    );

    let mut q = query(pts[0], pts[1], &pts, &[], Side::Right, Side::Left);
    q.to_side = None;
    assert_eq!(
        shortest_orthogonal_grid_route_with_metrics(&q)
            .metrics
            .status,
        GridStatus::UnsupportedEndpointSide
    );

    // The start stub sits inside an obstacle.
    let wall = [Rect::new(0.0, -20.0, 30.0, 40.0)];
    let q = query(pts[0], pts[1], &pts, &wall, Side::Right, Side::Left);
    assert_eq!(
        shortest_orthogonal_grid_route_with_metrics(&q)
            .metrics
            .status,
        GridStatus::EndpointBlocked
    );
}

#[test]
fn grid_never_borrows_a_frame_border_as_a_corridor() {
    // A frame border along y = 0 between the endpoints; the route must jog off it.
    let pts = [[0.0, 0.0], [200.0, 0.0]];
    let border = [Seg::new([20.0, 0.0], [180.0, 0.0])];
    let mut q = query(pts[0], pts[1], &pts, &[], Side::Right, Side::Left);
    q.border_segments = &border;
    let route = shortest_orthogonal_grid_route(&q).unwrap();
    for w in route.points.windows(2) {
        let horizontal_on_border = w[0][1] == 0.0
            && w[1][1] == 0.0
            && w[0][0].max(w[1][0]) > 20.0
            && w[0][0].min(w[1][0]) < 180.0;
        assert!(!horizontal_on_border, "{:?}", route.points);
    }
}

#[test]
fn grid_avoided_segment_is_not_shared_beyond_the_overlap() {
    let pts = [[0.0, 0.0], [200.0, 0.0]];
    let avoided = [Seg::new([40.0, 0.0], [160.0, 0.0])];
    let mut q = query(pts[0], pts[1], &pts, &[], Side::Right, Side::Left);
    q.avoided_segments = &avoided;
    q.allow_avoided_crossings = true;
    q.bend_penalty_px = 48.0;
    let route = shortest_orthogonal_grid_route(&q).unwrap();
    assert!(route.points.len() > 2, "{:?}", route.points);
}

// ---------------------------------------------------------------------------------------------
// Properties on random rect fields
// ---------------------------------------------------------------------------------------------

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn int(&mut self, lo: i64, hi: i64) -> f64 {
        (lo + (self.next() % (hi - lo + 1) as u64) as i64) as f64
    }
}

fn field(rng: &mut Lcg, count: usize) -> Vec<Rect> {
    let mut rects: Vec<Rect> = Vec::new();
    for _ in 0..count * 8 {
        if rects.len() >= count {
            break;
        }
        let r = Rect::new(
            rng.int(0, 40) * 20.0,
            rng.int(0, 26) * 20.0,
            rng.int(3, 9) * 20.0,
            rng.int(2, 5) * 20.0,
        );
        if rects
            .iter()
            .all(|o| !ubiq_archify::geom::rects_overlap(&r, o, 12.0))
        {
            rects.push(r);
        }
    }
    rects
}

fn free_point(rng: &mut Lcg, rects: &[Rect]) -> Pt {
    loop {
        let p = [rng.int(0, 60) * 10.0, rng.int(0, 40) * 10.0];
        if rects
            .iter()
            .all(|r| ubiq_archify::geom::point_rect_distance(p, r) > 20.0)
        {
            return p;
        }
    }
}

fn side(rng: &mut Lcg) -> Side {
    [Side::Left, Side::Right, Side::Top, Side::Bottom][(rng.next() % 4) as usize]
}

fn bends(points: &[Pt]) -> usize {
    points.len().saturating_sub(2)
}

#[test]
fn grid_properties_on_random_fields() {
    let mut rng = Lcg(0x5eed);
    let mut routed = 0;
    for case in 0..300 {
        let rects = field(&mut rng, 3 + (case % 12));
        let (start, end) = (free_point(&mut rng, &rects), free_point(&mut rng, &rects));
        if start == end {
            continue;
        }
        let (from, to) = (side(&mut rng), side(&mut rng));
        let pts = [start, end];
        let mut q = query(start, end, &pts, &rects, from, to);
        q.endpoint_stub_px = Some(8.0);
        let plain = shortest_orthogonal_grid_route(&q);
        q.bend_penalty_px = 48.0;
        let bent = shortest_orthogonal_grid_route(&q);
        assert_eq!(
            plain.is_some(),
            bent.is_some(),
            "case {case}: same graph, same reachability"
        );
        let (Some(plain), Some(bent)) = (plain, bent) else {
            continue;
        };
        routed += 1;
        for route in [&plain, &bent] {
            let p = &route.points;
            assert_eq!(
                (p[0], *p.last().unwrap()),
                (start, end),
                "case {case}: bracketed"
            );
            // Orthogonal.
            assert!(route.length.is_some(), "case {case}: not orthogonal {p:?}");
            // Clear of every (clearance-expanded) obstacle.
            for w in p.windows(2) {
                for r in &rects {
                    assert!(
                        !segment_intersects_rect(&Seg::new(w[0], w[1]), &r.expanded(2.0), 0.0),
                        "case {case}: {w:?} hits {r:?}"
                    );
                }
            }
            // No reversal: a fold-back would be a collinear turn through 180 degrees.
            // (The last turn onto the end stub may fold back: the JS charges it a bend.)
            for w in p.windows(3).take(p.len().saturating_sub(3)) {
                let cross = (w[1][0] - w[0][0]) * (w[2][1] - w[1][1])
                    - (w[1][1] - w[0][1]) * (w[2][0] - w[1][0]);
                let dot = (w[1][0] - w[0][0]) * (w[2][0] - w[1][0])
                    + (w[1][1] - w[0][1]) * (w[2][1] - w[1][1]);
                assert!(
                    !(cross == 0.0 && dot < 0.0),
                    "case {case}: reversal in {p:?}"
                );
            }
        }
        // Minimal length without the penalty; minimal cost with it; fewer or equal bends at
        // equal length.
        let (lp, lb) = (plain.length.unwrap(), bent.length.unwrap());
        assert!(lp <= lb, "case {case}: plain {lp} vs bent {lb}");
        let cost = |r: &ubiq_archify::route::grid::GridRoute| {
            r.length.unwrap() + 48.0 * bends(&r.points) as f64
        };
        assert!(
            cost(&bent) <= cost(&plain),
            "case {case}: penalised search is not cheapest"
        );
        if lp == lb {
            assert!(
                bends(&bent.points) <= bends(&plain.points),
                "case {case}: bends at equal length"
            );
        }
    }
    assert!(routed >= 150, "only {routed} routed");
}
