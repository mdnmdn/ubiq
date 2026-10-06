//! The dataflow router (P2.4). `00` §5.6.5, `render-dataflow.mjs:305-373` (`routeVia`, `flowSides`,
//! the spread call, `pathFor`) with the automatic port spread of `geometry.mjs:1585` under the
//! legacy, horizontal-first side rule.
//!
//! No obstacle avoidance: the route is the authored `via`, a preset or the auto elbow, and the
//! gates (P2.5) only lint it. The route is *not* [`crate::geom::normalize`]d: `pathFor` drops
//! consecutive duplicates only, so a collinear middle point survives.

use std::collections::HashMap;

use crate::geom::{
    Pt, Rect, Side, SpreadOptions, SpreadRelation, anchor, automatic_port_spread, chosen_side,
    legacy_default_from_side, legacy_default_to_side,
};
use crate::geom::End;
use crate::model::common::Side as ModelSide;
use crate::model::dataflow::{Flow, FlowRoute};

/// `vertical-channel` runs `44` from the start, toward the end (`render-dataflow.mjs:311`).
pub const VERTICAL_CHANNEL_OFFSET: f64 = 44.0;
/// `bottom-channel` runs `26` below the lower of the two bottoms (`:315`).
pub const BOTTOM_CHANNEL_OFFSET: f64 = 26.0;
/// `top-channel` runs `24` above the higher of the two tops (`:319`).
pub const TOP_CHANNEL_OFFSET: f64 = 24.0;
/// `auto` is straight when the ports are within `4` vertically (`:324`).
pub const AUTO_STRAIGHT_DY: f64 = 4.0;
/// Consecutive route points closer than this on both axes are one point (`:362`).
pub const DUPLICATE_EPSILON: f64 = 0.0001;

/// A routed flow: the canonical un-rounded points (`data-composition-points`) and the sides chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct Routed {
    pub points: Vec<Pt>,
    pub from_side: Side,
    pub to_side: Side,
}

/// The authored schema side as the geometry one.
pub fn side(s: ModelSide) -> Side {
    match s {
        ModelSide::Left => Side::Left,
        ModelSide::Right => Side::Right,
        ModelSide::Top => Side::Top,
        ModelSide::Bottom => Side::Bottom,
    }
}

/// `flowSides`: an authored side, else the legacy horizontal-first default.
pub fn flow_sides(flow: &Flow, from: &Rect, to: &Rect) -> (Side, Side) {
    (
        chosen_side(flow.from_side.map(side), legacy_default_from_side(from, to)),
        chosen_side(flow.to_side.map(side), legacy_default_to_side(from, to)),
    )
}

/// `routeVia(flow, from, to, start, end)`: the intermediate points. A present `via` (even empty)
/// is used verbatim; otherwise the preset of `route`, `auto` by default.
pub fn route_via(flow: &Flow, from: &Rect, to: &Rect, start: Pt, end: Pt) -> Vec<Pt> {
    if let Some(via) = &flow.via {
        return via.clone();
    }
    match flow.route.unwrap_or(FlowRoute::Auto) {
        FlowRoute::Straight => Vec::new(),
        FlowRoute::VerticalChannel => {
            let x = flow
                .channel_x
                .unwrap_or(start[0] + if end[0] > start[0] { VERTICAL_CHANNEL_OFFSET } else { -VERTICAL_CHANNEL_OFFSET });
            vec![[x, start[1]], [x, end[1]]]
        }
        FlowRoute::BottomChannel => {
            let y = flow
                .channel_y
                .unwrap_or((from.y + from.height).max(to.y + to.height) + BOTTOM_CHANNEL_OFFSET);
            vec![[start[0], y], [end[0], y]]
        }
        FlowRoute::TopChannel => {
            let y = flow.channel_y.unwrap_or(from.y.min(to.y) - TOP_CHANNEL_OFFSET);
            vec![[start[0], y], [end[0], y]]
        }
        FlowRoute::Auto => {
            if (start[1] - end[1]).abs() < AUTO_STRAIGHT_DY {
                return Vec::new();
            }
            let mid_x = start[0] + (end[0] - start[0]) / 2.0;
            vec![[mid_x, start[1]], [mid_x, end[1]]]
        }
    }
}

/// Whether the port spread moves this flow's endpoints: no `via` (an empty one counts), `route`
/// absent or `auto`, no `channelX`/`channelY`, no `labelAt`.
pub fn is_automatic(flow: &Flow) -> bool {
    matches!(flow.route, None | Some(FlowRoute::Auto))
        && flow.via.is_none()
        && flow.channel_x.is_none()
        && flow.channel_y.is_none()
        && flow.label_at.is_none()
}

/// `pathFor` for every flow: `None` where an endpoint is not in `nodes` (the caller reports it).
/// Ports are spread first (`automaticPortSpread` with the legacy `sideFor`), then each route is
/// `[start, ...routeVia, end]` with consecutive duplicates dropped; a route that collapses to one
/// point keeps its end.
pub fn route_all(flows: &[Flow], nodes: &HashMap<String, Rect>) -> Vec<Option<Routed>> {
    let sides: Vec<Option<(Side, Side)>> = flows
        .iter()
        .map(|f| Some(flow_sides(f, nodes.get(&f.from)?, nodes.get(&f.to)?)))
        .collect();
    let relations: Vec<SpreadRelation> = flows
        .iter()
        .map(|f| SpreadRelation {
            id: f.id.clone().unwrap_or_default(),
            from: f.from.clone(),
            to: f.to.clone(),
            label: f.label.clone(),
            from_side: f.from_side.map(side),
            to_side: f.to_side.map(side),
            auto: is_automatic(f),
        })
        .collect();
    let side_for = |i: usize, end: End| {
        sides[i].map(|(from, to)| if end == End::From { from } else { to })
    };
    let ports = automatic_port_spread(&relations, nodes, SpreadOptions::default(), Some(&side_for), None);

    flows
        .iter()
        .enumerate()
        .map(|(i, flow)| {
            let (from, to) = (nodes.get(&flow.from)?, nodes.get(&flow.to)?);
            let (from_side, to_side) = sides[i]?;
            let start = ports[i].from.unwrap_or_else(|| anchor(from, from_side));
            let end = ports[i].to.unwrap_or_else(|| anchor(to, to_side));
            let raw = std::iter::once(start).chain(route_via(flow, from, to, start, end)).chain(std::iter::once(end));
            let mut points: Vec<Pt> = Vec::new();
            for p in raw {
                let keep = points.last().is_none_or(|prev| {
                    (p[0] - prev[0]).abs() > DUPLICATE_EPSILON || (p[1] - prev[1]).abs() > DUPLICATE_EPSILON
                });
                if keep {
                    points.push(p);
                }
            }
            if points.len() < 2 {
                points.push(end);
            }
            Some(Routed { points, from_side, to_side })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(from: &str, to: &str) -> Flow {
        Flow {
            id: None,
            from: from.into(),
            to: to.into(),
            label: "x".into(),
            classification: None,
            variant: None,
            route: None,
            from_side: None,
            to_side: None,
            channel_x: None,
            channel_y: None,
            label_at: None,
            label_dx: None,
            label_dy: None,
            label_segment: None,
            via: None,
            width: None,
        }
    }

    fn nodes() -> HashMap<String, Rect> {
        [
            ("a", Rect::new(44.0, 128.0, 112.0, 58.0)),
            ("b", Rect::new(259.0, 128.0, 112.0, 58.0)),
            ("c", Rect::new(259.0, 242.0, 112.0, 58.0)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
    }

    #[test]
    fn auto_is_straight_when_level_else_a_midpoint_elbow() {
        let n = nodes();
        let r = route_all(&[flow("a", "b")], &n);
        assert_eq!(r[0].as_ref().unwrap().points, vec![[156.0, 157.0], [259.0, 157.0]]);
        let r = route_all(&[flow("a", "c")], &n);
        let p = &r[0].as_ref().unwrap().points;
        assert_eq!(p, &vec![[156.0, 157.0], [207.5, 157.0], [207.5, 271.0], [259.0, 271.0]]);
    }

    #[test]
    fn presets_follow_the_source() {
        let n = nodes();
        let (a, c) = (&n["a"], &n["c"]);
        let mut f = flow("a", "c");
        f.route = Some(FlowRoute::VerticalChannel);
        assert_eq!(route_via(&f, a, c, [156.0, 157.0], [259.0, 271.0]), vec![[200.0, 157.0], [200.0, 271.0]]);
        f.channel_x = Some(210.0);
        assert_eq!(route_via(&f, a, c, [156.0, 157.0], [259.0, 271.0])[0], [210.0, 157.0]);
        f.route = Some(FlowRoute::BottomChannel);
        f.channel_y = None;
        // max(186, 300) + 26
        assert_eq!(route_via(&f, a, c, [100.0, 186.0], [315.0, 300.0]), vec![[100.0, 326.0], [315.0, 326.0]]);
        f.route = Some(FlowRoute::TopChannel);
        assert_eq!(route_via(&f, a, c, [100.0, 128.0], [315.0, 242.0]), vec![[100.0, 104.0], [315.0, 104.0]]);
        f.route = Some(FlowRoute::Straight);
        assert!(route_via(&f, a, c, [0.0, 0.0], [1.0, 1.0]).is_empty());
        f.via = Some(vec![]);
        f.route = Some(FlowRoute::TopChannel);
        assert!(route_via(&f, a, c, [0.0, 0.0], [1.0, 1.0]).is_empty(), "an empty via wins over the preset");
    }

    #[test]
    fn shared_sides_spread_and_authored_geometry_does_not() {
        let n = nodes();
        // Two flows leave a's right side toward b and c: spread by 14 around the midpoint (157).
        let r = route_all(&[flow("a", "b"), flow("a", "c")], &n);
        let (s0, s1) = (r[0].as_ref().unwrap().points[0], r[1].as_ref().unwrap().points[0]);
        assert_eq!((s0[1], s1[1]), (150.0, 164.0));
        let mut pinned = flow("a", "c");
        pinned.label_at = Some([0.0, 0.0]);
        let r = route_all(&[flow("a", "b"), pinned], &n);
        assert_eq!(r[0].as_ref().unwrap().points[0], [156.0, 157.0]);
        assert_eq!(r[1].as_ref().unwrap().points[0], [156.0, 157.0]);
    }

    #[test]
    fn a_missing_endpoint_is_none_and_duplicates_collapse() {
        let n = nodes();
        assert!(route_all(&[flow("a", "zzz")], &n)[0].is_none());
        let mut f = flow("a", "b");
        f.via = Some(vec![[156.0, 157.0], [259.0, 157.0]]);
        assert_eq!(route_all(&[f], &n)[0].as_ref().unwrap().points.len(), 2);
    }
}
