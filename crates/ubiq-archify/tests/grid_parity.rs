//! `route::grid` against Archify's own `shortestOrthogonalGridRoute` / `cleanRouteDetourProblems`.
//!
//! `grid_cases.json` is written by `gen-grid-cases.mjs`: random rect fields with the
//! architecture router's options, the oracle's options, and authored `via` corridors. Every case
//! must agree exactly: status, points, length, the node/edge/visited counters (the visited count
//! pins the heap tie-breaking) and the detour messages.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use ubiq_archify::geom::{Pt, Rect, Seg, Side};
use ubiq_archify::route::grid::{
    DetourRelation, DetourThresholds, GridQuery, clean_route_detour_problems,
    shortest_orthogonal_grid_route_with_metrics,
};
use serde_json::Value;

fn pt(v: &Value) -> Pt {
    [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()]
}

fn pts(v: &Value) -> Vec<Pt> {
    v.as_array().unwrap().iter().map(pt).collect()
}

fn rects(v: &Value) -> Vec<Rect> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            Rect::new(
                r["x"].as_f64().unwrap(),
                r["y"].as_f64().unwrap(),
                r["width"].as_f64().unwrap(),
                r["height"].as_f64().unwrap(),
            )
        })
        .collect()
}

fn segs(v: &Value) -> Vec<Seg> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|s| Seg::new(pt(&s["start"]), pt(&s["end"])))
                .collect()
        })
        .unwrap_or_default()
}

fn side(v: &Value) -> Option<Side> {
    v.as_str().and_then(Side::parse)
}

fn corpus() -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/grid_cases.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn grid_routes_match_archify() {
    let mut checked = 0;
    let mut routed = 0;
    for case in corpus().iter().filter(|c| c["kind"] == "route") {
        let name = case["name"].as_str().unwrap();
        let i = &case["input"];
        let (points, obstacles) = (pts(&i["points"]), rects(&i["obstacles"]));
        let (avoided, borders) = (segs(&i["avoidedSegments"]), segs(&i["borderSegments"]));
        let mut q = GridQuery::new(
            pt(&i["start"]),
            pt(&i["end"]),
            &points,
            &obstacles,
            side(&i["fromSide"]),
            side(&i["toSide"]),
            i["clearance"].as_f64().unwrap(),
            i["maximumObstacleCount"].as_u64().unwrap() as usize,
        );
        q.endpoint_stub_px = i["endpointStubPx"].as_f64();
        q.maximum_grid_nodes = i["maximumGridNodes"].as_u64().map(|n| n as usize);
        q.avoided_segments = &avoided;
        q.border_segments = &borders;
        q.allow_avoided_crossings = i["allowAvoidedCrossings"].as_bool().unwrap_or(false);
        if let Some(v) = i["minimumAvoidedOverlapPx"].as_f64() {
            q.minimum_avoided_overlap_px = v;
        }
        if let Some(v) = i["routeSeparationPx"].as_f64() {
            q.route_separation_px = v;
        }
        if let Some(v) = i["minimumSegmentPx"].as_f64() {
            q.minimum_segment_px = v;
        }
        q.bend_penalty_px = i["bendPenaltyPx"].as_f64().unwrap_or(0.0);

        let out = shortest_orthogonal_grid_route_with_metrics(&q);
        let e = &case["expected"];
        assert_eq!(
            out.metrics.status.as_str(),
            e["status"].as_str().unwrap(),
            "{name}: status"
        );
        match &out.route {
            Some(route) => {
                routed += 1;
                assert_eq!(route.points, pts(&e["points"]), "{name}: points");
                assert_eq!(route.length, e["length"].as_f64(), "{name}: length");
                assert_eq!(
                    route.obstacle_count as u64,
                    e["obstacleCount"].as_u64().unwrap(),
                    "{name}: obstacle count"
                );
            }
            None => assert!(e["points"].is_null(), "{name}: expected a route"),
        }
        for (got, key) in [
            (out.metrics.candidate_node_count, "candidateNodeCount"),
            (out.metrics.usable_node_count, "usableNodeCount"),
            (out.metrics.graph_edge_count, "graphEdgeCount"),
            (out.metrics.visited_node_count, "visitedNodeCount"),
        ] {
            // Counters the JS had not yet written when it bailed out are 0 there too.
            assert_eq!(got as u64, e[key].as_u64().unwrap(), "{name}: {key}");
        }
        checked += 1;
    }
    assert!(
        checked >= 300 && routed >= 250,
        "corpus too thin: {checked} / {routed}"
    );
}

#[test]
fn grid_detour_oracle_matches_archify() {
    let (mut checked, mut found) = (0, 0);
    for case in corpus().iter().filter(|c| c["kind"] == "detour") {
        let name = case["name"].as_str().unwrap();
        let i = &case["input"];
        let obstacles = rects(&i["obstacles"]);
        let content = (!i["contentRects"].is_null()).then(|| rects(&i["contentRects"]));
        let ids: HashSet<String> = i["endpointIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let relations: Vec<DetourRelation> = i["relations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| DetourRelation {
                id: r["id"].as_str().map(str::to_string),
                from: r["from"].as_str().unwrap().to_string(),
                to: r["to"].as_str().unwrap().to_string(),
                has_via: r["hasVia"].as_bool().unwrap(),
                points: pts(&r["points"]),
                from_side: side(&r["fromSide"]),
                to_side: side(&r["toSide"]),
            })
            .collect();
        let got: Vec<String> = clean_route_detour_problems(
            &relations,
            &obstacles,
            content.as_deref(),
            &ids,
            i["showcase"].as_bool().unwrap(),
            &DetourThresholds::default(),
        )
        .iter()
        .map(|f| f.message("architecture", "connections"))
        .collect();
        let want: Vec<String> = case["expected"]["problems"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap().to_string())
            .collect();
        assert_eq!(got, want, "{name}");
        checked += 1;
        found += want.len();
    }
    assert!(
        checked >= 100 && found >= 20,
        "corpus too thin: {checked} / {found}"
    );
}
