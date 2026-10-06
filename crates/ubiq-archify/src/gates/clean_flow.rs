//! `clean-flow/*` (`geometry.mjs` `cleanEndpointSideProblems`, `cleanFlowProblems`): errors in every
//! profile, including none. Port with the messages, evidence and fixes of Archify 3.0.1.

use serde_json::Value;

use super::{Ctx, Obstacle, Rel, SideOrigin, Sink, describe, evidence, point0, point1, pt_json, re_plan_hint, rel_subject};
use crate::diag::Diagnostic;
use crate::geom::{Pt, Seg, Side, is_finite_point, normalize, segment_intersects_rect};

/// Which end of a relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Source,
    Target,
}

/// `ENDPOINT_SIDE_RULES[side]` applied to an end: axis, expected direction word and the sign the
/// segment must move in along that axis.
struct SideRule {
    horizontal: bool,
    sign: f64,
    direction: &'static str,
}

fn side_rule(side: Side, end: End) -> SideRule {
    // (axis, sourceSign, targetSign, sourceDirection, targetDirection)
    let (horizontal, source_sign, target_sign, source_dir, target_dir) = match side {
        Side::Left => (true, -1.0, 1.0, "leftward", "rightward from the left"),
        Side::Right => (true, 1.0, -1.0, "rightward", "leftward from the right"),
        Side::Top => (false, -1.0, 1.0, "upward", "downward from above"),
        Side::Bottom => (false, 1.0, -1.0, "downward", "upward from below"),
    };
    match end {
        End::Source => SideRule { horizontal, sign: source_sign, direction: source_dir },
        End::Target => SideRule { horizontal, sign: target_sign, direction: target_dir },
    }
}

/// A first or last segment that does not honour its side (`endpointSideIssue`).
#[derive(Clone, Debug, PartialEq)]
pub struct SideIssue {
    pub end: End,
    pub side: Side,
    pub segment_index: usize,
    pub start: Pt,
    pub to: Pt,
    pub axis: &'static str,
    pub direction: &'static str,
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
        Side::Top => "top",
        Side::Bottom => "bottom",
    }
}

/// `endpointSideIssue(points, endpoint, side)`.
pub fn endpoint_side_issue(points: &[Pt], end: End, side: Side) -> Option<SideIssue> {
    let rule = side_rule(side, end);
    let normalized = normalize(points);
    if normalized.len() < 2 {
        return None;
    }
    let index = if end == End::Source { 0 } else { normalized.len() - 2 };
    let (start, to) = (normalized[index], normalized[index + 1]);
    let (dx, dy) = (to[0] - start[0], to[1] - start[1]);
    let (along, across) = if rule.horizontal { (dx, dy) } else { (dy, dx) };
    if across.abs() <= 0.0001 && along * rule.sign > 0.0001 {
        return None;
    }
    Some(SideIssue {
        end,
        side,
        segment_index: index,
        start,
        to,
        axis: if rule.horizontal { "horizontal" } else { "vertical" },
        direction: rule.direction,
    })
}

/// `routeHonorsEndpointSides`: for the automatic routers' candidate choice (P3).
pub fn route_honors_endpoint_sides(points: &[Pt], from: Side, to: Side) -> bool {
    endpoint_side_issue(points, End::Source, from).is_none() && endpoint_side_issue(points, End::Target, to).is_none()
}

/// `cleanEndpointSideProblems`: the first and last segment leave and enter their (authored or
/// inferred) side perpendicularly. `rels` carry the sides to check ([`Rel::from_side`]).
pub fn endpoint_side(ctx: &Ctx<'_>, rels: &[Rel], route_hint: &str, sink: &mut Sink) {
    for rel in rels {
        if rel.points.len() < 2 {
            continue;
        }
        for (end, spec) in [(End::Source, rel.from_side), (End::Target, rel.to_side)] {
            let Some((side, origin)) = spec else { continue };
            let Some(issue) = endpoint_side_issue(&rel.points, end, side) else { continue };
            let field = if end == End::Source { "fromSide" } else { "toSide" };
            let side_field = if origin == SideOrigin::Inferred { format!("inferred {field}") } else { field.to_owned() };
            let role = if end == End::Source { "first" } else { "final" };
            let message = format!(
                "[clean-flow/endpoint-side-direction] {} {} {role} segment {} [{}] -> [{}] does not honor {side_field} \"{}\" — it must run {} {}; {route_hint}.",
                ctx.diagram,
                describe(ctx, rel),
                issue.segment_index,
                point1(issue.start),
                point1(issue.to),
                side_name(issue.side),
                issue.axis,
                issue.direction,
            );
            let ev = evidence([
                ("endpoint", Value::from(if end == End::Source { "source" } else { "target" })),
                ("authoredField", field.into()),
                ("sideOrigin", Value::from(if origin == SideOrigin::Inferred { "inferred" } else { "authored" })),
                ("side", side_name(issue.side).into()),
                ("segmentIndex", issue.segment_index.into()),
                ("from", pt_json(issue.start)),
                ("to", pt_json(issue.to)),
                ("expectedAxis", issue.axis.into()),
                ("expectedDirection", issue.direction.into()),
            ]);
            sink.error(
                Diagnostic::error("clean-flow/endpoint-side-direction", &message)
                    .with_subject(rel_subject(ctx, rel))
                    .with_evidence(ev)
                    .with_fixes([route_hint]),
            );
        }
    }
}

/// `cleanFlowProblems`: no relation crosses an unrelated opaque obstacle (`clearance` px).
pub fn edge_through_node(
    ctx: &Ctx<'_>,
    rels: &[Rel],
    obstacles: &[Obstacle],
    obstacle_kind: &str,
    clearance: f64,
    route_hint: &str,
    sink: &mut Sink,
) {
    for rel in rels {
        if rel.points.len() < 2 || !rel.points.iter().all(|p| is_finite_point(p)) {
            continue;
        }
        let known = |id: &str| obstacles.iter().any(|o| o.id == id);
        if !known(&rel.from) || !known(&rel.to) {
            continue;
        }
        for obstacle in obstacles {
            if obstacle.id == rel.from || obstacle.id == rel.to || !obstacle.rect.is_finite() {
                continue;
            }
            let hit = (0..rel.points.len() - 1).find(|&i| {
                segment_intersects_rect(&Seg::new(rel.points[i], rel.points[i + 1]), &obstacle.rect, clearance)
            });
            let Some(hit) = hit else { continue };
            let (a, b) = (rel.points[hit], rel.points[hit + 1]);
            let hint = re_plan_hint(&[rel], route_hint);
            let message = format!(
                "[clean-flow/edge-through-node] {} {} crosses {obstacle_kind} \"{}\" (unrelated to this relationship) on segment {hit} [{}] -> [{}] ({}px clearance) — {hint}.",
                ctx.diagram,
                describe(ctx, rel),
                obstacle.id,
                point0(a),
                point0(b),
                super::num(clearance),
            );
            let ev = evidence([
                ("obstacleKind", obstacle_kind.into()),
                ("obstacleId", obstacle.id.clone().into()),
                ("segmentIndex", hit.into()),
                ("from", pt_json(a)),
                ("to", pt_json(b)),
                ("clearancePx", crate::diag::json_num(clearance)),
            ]);
            sink.error(
                Diagnostic::error("clean-flow/edge-through-node", &message)
                    .with_subject(rel_subject(ctx, rel))
                    .with_evidence(ev)
                    .with_fixes([hint]),
            );
        }
    }
}
