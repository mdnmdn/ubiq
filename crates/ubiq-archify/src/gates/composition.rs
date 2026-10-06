//! `composition/*` (`geometry.mjs` `clean*Problems` and their `collect*` cores): proper crossings,
//! ambiguous corridors, container border runs, route rhythm, label/route clearance and label canvas
//! containment, with the severity-by-profile of the module doc in [`super`].
//!
//! The collectors are pure and public: [`receipt`] reuses them for the success receipt's
//! `composition` block, the same way the artifact checker reuses `geometry.mjs`.

use serde_json::{Map, Value, json};

use super::{
    Ctx, Gate, LabelRect, Rel, Sink, describe, evidence, format_rect, point1, pt_json, re_plan_hint, rect_json, rel_subject,
    rel_subject_json, round1,
};
use crate::diag::{Diagnostic, Severity, json_num};
use crate::geom::{
    BorderSide, Frame, Pt, Rect, Seg, collinear_axis_overlap, frame_border_segments, is_finite_point, normalize,
    proper_segment_intersection, segment_rect_clearance, segment_rect_intersection_length,
};

/// `NUMERIC` tolerance of the collectors.
const EPS: f64 = 0.0001;

/// A structural frame a route must not run along (a dataflow stage, an architecture boundary).
#[derive(Clone, Debug, PartialEq)]
pub struct FrameInfo {
    pub frame: Frame,
    /// `stage`, `boundary`, ... (`frame.kind`).
    pub kind: String,
    /// `frame.id` as Archify prints it (a number for a dataflow stage).
    pub id: Value,
    pub label: Option<String>,
}

fn id_text(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn severity_word(severity: Severity) -> &'static str {
    if severity == Severity::Error { "showcase" } else { "standard" }
}

fn emit(sink: &mut Sink, d: Diagnostic) {
    if d.severity == Severity::Error {
        sink.error(d);
    } else {
        sink.record(d);
    }
}

// ---- proper crossing ---------------------------------------------------------------------------

/// A proper X between two relations (indices into the slice passed to the collector).
#[derive(Clone, Debug, PartialEq)]
pub struct Crossing {
    pub left: usize,
    pub right: usize,
    pub left_segment: usize,
    pub right_segment: usize,
    pub point: Pt,
}

fn shares_endpoint(a: &Rel, b: &Rel) -> bool {
    [&a.from, &a.to].iter().any(|id| **id == b.from || **id == b.to)
}

fn finite_route(rel: &Rel) -> bool {
    rel.points.len() >= 2 && rel.points.iter().all(|p| is_finite_point(p))
}

/// The proper crossings of unrelated relations: authored segments, relations that share an endpoint
/// are exempt (`includeSharedEndpoints` false).
pub fn collect_crossings(rels: &[Rel]) -> Vec<Crossing> {
    collect_crossings_by(rels, false, false)
}

/// [`collect_crossings`] as workflow v2 calls `cleanCrossingProblems`: `merge_forward` judges routes by
/// their forward-collinear analysis segments (`forwardCollinearAnalysisSegments`, the hit names the
/// authored segment it lands on) and `include_shared` judges pairs that share an endpoint too.
pub fn collect_crossings_by(rels: &[Rel], merge_forward: bool, include_shared: bool) -> Vec<Crossing> {
    let routed: Vec<&Rel> = rels.iter().filter(|r| finite_route(r)).collect();
    let segments: Vec<Vec<crate::route::workflow::AnalysisSegment>> = routed
        .iter()
        .map(|r| {
            if merge_forward {
                crate::route::workflow::forward_collinear_segments(&r.points)
            } else {
                crate::route::workflow::authored_segments(&r.points)
            }
        })
        .collect();
    let mut hits = Vec::new();
    for (li, left) in routed.iter().enumerate() {
        for (ri, right) in routed.iter().enumerate().skip(li + 1) {
            if shares_endpoint(left, right) && !include_shared {
                continue;
            }
            'pair: for a in &segments[li] {
                for b in &segments[ri] {
                    if let Some(point) = proper_segment_intersection(a.start, a.end, b.start, b.end) {
                        hits.push(Crossing {
                            left: li,
                            right: ri,
                            left_segment: a.source_index_at(point),
                            right_segment: b.source_index_at(point),
                            point,
                        });
                        break 'pair;
                    }
                }
            }
        }
    }
    // Indices refer to `routed`; map them back to `rels` positions.
    let positions: Vec<usize> = rels.iter().enumerate().filter(|(_, r)| finite_route(r)).map(|(i, _)| i).collect();
    hits.into_iter()
        .map(|h| Crossing { left: positions[h.left], right: positions[h.right], ..h })
        .collect()
}

/// `cleanCrossingProblems`: error in showcase; a recorded warning only when the renderer opts in
/// (`warn_in_standard`, workflow v2), else nothing.
pub fn proper_crossing(ctx: &Ctx<'_>, rels: &[Rel], gate: Gate, warn_in_standard: bool, route_hint: &str, sink: &mut Sink) {
    proper_crossing_resolved(ctx, rels, gate, warn_in_standard, &|_, _| false, route_hint, sink);
}

/// [`proper_crossing`] with `crossingResolved(left, right)`: a crossing the renderer resolves with
/// halos (two automatic architecture routes) is not reported. Indices are positions in `rels`.
#[allow(clippy::too_many_arguments)]
pub fn proper_crossing_resolved(
    ctx: &Ctx<'_>,
    rels: &[Rel],
    gate: Gate,
    warn_in_standard: bool,
    resolved: &dyn Fn(usize, usize) -> bool,
    route_hint: &str,
    sink: &mut Sink,
) {
    let severity = if gate.showcase() { Severity::Error } else { Severity::Warning };
    if severity == Severity::Warning && !warn_in_standard {
        return;
    }
    emit_crossings(ctx, rels, gate, collect_crossings(rels), resolved, route_hint, sink);
}

/// Workflow v2's `cleanCrossingProblems`: merged analysis segments, shared endpoints judged, a
/// warning below showcase.
pub fn proper_crossing_workflow_v2(ctx: &Ctx<'_>, rels: &[Rel], gate: Gate, route_hint: &str, sink: &mut Sink) {
    emit_crossings(ctx, rels, gate, collect_crossings_by(rels, true, true), &|_, _| false, route_hint, sink);
}

fn emit_crossings(
    ctx: &Ctx<'_>,
    rels: &[Rel],
    gate: Gate,
    hits: Vec<Crossing>,
    resolved: &dyn Fn(usize, usize) -> bool,
    route_hint: &str,
    sink: &mut Sink,
) {
    let severity = if gate.showcase() { Severity::Error } else { Severity::Warning };
    for hit in hits {
        if resolved(hit.left, hit.right) {
            continue;
        }
        let (left, right) = (&rels[hit.left], &rels[hit.right]);
        let hint = re_plan_hint(&[left, right], route_hint);
        let message = format!(
            "[composition/proper-crossing] {} {} {} crosses {} at [{}] (segments {} and {}) — {hint}.",
            severity_word(severity),
            ctx.diagram,
            describe(ctx, left),
            describe(ctx, right),
            point1(hit.point),
            hit.left_segment,
            hit.right_segment,
        );
        let ev = evidence([
            ("otherRelationship", rel_subject_json(ctx, right)),
            ("point", pt_json(hit.point)),
            ("segmentIndex", hit.left_segment.into()),
            ("otherSegmentIndex", hit.right_segment.into()),
        ]);
        emit(
            sink,
            Diagnostic::new("composition/proper-crossing", severity, &message)
                .with_subject(rel_subject(ctx, left))
                .with_evidence(ev)
                .with_fixes([hint]),
        );
    }
}

// ---- ambiguous corridor ------------------------------------------------------------------------

/// Two unrelated relations sharing a collinear stretch.
#[derive(Clone, Debug, PartialEq)]
pub struct Corridor {
    pub left: usize,
    pub right: usize,
    pub left_segment: usize,
    pub right_segment: usize,
    pub length: f64,
    pub start: Pt,
    pub end: Pt,
}

/// `collectAmbiguousCorridors` with the defaults of every caller but workflow v2: a shared endpoint
/// exempts the pair, no short-trunk exemption. The longest overlap per pair, on normalised routes.
pub fn collect_corridors(rels: &[Rel], min_overlap: f64) -> Vec<Corridor> {
    collect_corridors_by(rels, min_overlap, &|_, _| false)
}

/// [`collect_corridors`] with `includeSharedEndpoints(left, right)` (positions in `rels`): a pair
/// that shares an endpoint is judged like any other when it returns `true` (the automatic
/// architecture routes promise separate ports).
pub fn collect_corridors_by(rels: &[Rel], min_overlap: f64, include_shared: &dyn Fn(usize, usize) -> bool) -> Vec<Corridor> {
    collect_corridors_with(rels, min_overlap, include_shared, &|_, _, _| false)
}

/// A bounded terminal trunk exemption: `(overlap, left route, right route)`, both normalised
/// (`shortWorkflowTrunk`, workflow v2 only).
pub type TrunkExempt<'a> = dyn Fn(&Corridor, &[Pt], &[Pt]) -> bool + 'a;

/// [`collect_corridors_by`] that also skips an overlap `exempt` accepts (`allowShortWorkflowTrunks`).
pub fn collect_corridors_with(
    rels: &[Rel],
    min_overlap: f64,
    include_shared: &dyn Fn(usize, usize) -> bool,
    exempt: &TrunkExempt<'_>,
) -> Vec<Corridor> {
    let routed: Vec<(usize, Vec<Pt>)> = rels
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let points = normalize(&r.points);
            (points.len() >= 2).then_some((i, points))
        })
        .collect();
    let mut hits = Vec::new();
    for (li, (left_pos, lp)) in routed.iter().enumerate() {
        for (right_pos, rp) in routed.iter().skip(li + 1) {
            if shares_endpoint(&rels[*left_pos], &rels[*right_pos]) && !include_shared(*left_pos, *right_pos) {
                continue;
            }
            let mut longest: Option<Corridor> = None;
            for ls in 0..lp.len() - 1 {
                for rs in 0..rp.len() - 1 {
                    let Some(o) = collinear_axis_overlap(lp[ls], lp[ls + 1], rp[rs], rp[rs + 1]) else { continue };
                    if o.length + EPS < min_overlap {
                        continue;
                    }
                    let candidate = Corridor {
                        left: *left_pos,
                        right: *right_pos,
                        left_segment: ls,
                        right_segment: rs,
                        length: o.length,
                        start: o.start,
                        end: o.end,
                    };
                    if exempt(&candidate, lp, rp) {
                        continue;
                    }
                    if longest.as_ref().is_none_or(|l| o.length > l.length + EPS) {
                        longest = Some(candidate);
                    }
                }
            }
            hits.extend(longest);
        }
    }
    hits
}

/// `cleanAmbiguousCorridorProblems`.
pub fn ambiguous_corridor(ctx: &Ctx<'_>, rels: &[Rel], gate: Gate, route_hint: &str, sink: &mut Sink) {
    ambiguous_corridor_by(ctx, rels, gate, &|_, _| false, route_hint, sink);
}

/// [`ambiguous_corridor`] with `includeSharedEndpoints` ([`collect_corridors_by`]).
pub fn ambiguous_corridor_by(
    ctx: &Ctx<'_>,
    rels: &[Rel],
    gate: Gate,
    include_shared: &dyn Fn(usize, usize) -> bool,
    route_hint: &str,
    sink: &mut Sink,
) {
    ambiguous_corridor_with(ctx, rels, gate, include_shared, &|_, _, _| false, false, route_hint, sink);
}

/// [`ambiguous_corridor_by`] with the short-trunk exemption and, when `warn_in_standard`, a warning
/// below showcase (`allowShortWorkflowTrunks`: workflow v2).
#[allow(clippy::too_many_arguments)]
pub fn ambiguous_corridor_with(
    ctx: &Ctx<'_>,
    rels: &[Rel],
    gate: Gate,
    include_shared: &dyn Fn(usize, usize) -> bool,
    exempt: &TrunkExempt<'_>,
    warn_in_standard: bool,
    route_hint: &str,
    sink: &mut Sink,
) {
    let severity = if gate.showcase() { Severity::Error } else { Severity::Warning };
    if severity == Severity::Warning && !warn_in_standard {
        return;
    }
    const MIN: f64 = 8.0;
    for hit in collect_corridors_with(rels, MIN, include_shared, exempt) {
        let (left, right) = (&rels[hit.left], &rels[hit.right]);
        let length = round1(hit.length);
        let hint = re_plan_hint(&[left, right], route_hint);
        let message = format!(
            "[composition/ambiguous-corridor] {} {} {} shares a {}px corridor with {} at [{}] -> [{}] (segments {} and {}; minimum {}px) — {hint}.",
            severity_word(severity),
            ctx.diagram,
            describe(ctx, left),
            super::num(length),
            describe(ctx, right),
            point1(hit.start),
            point1(hit.end),
            hit.left_segment,
            hit.right_segment,
            super::num(MIN),
        );
        let ev = evidence([
            ("otherRelationship", rel_subject_json(ctx, right)),
            ("overlapLengthPx", json_num(length)),
            ("minimumPx", json_num(MIN)),
            ("from", pt_json(hit.start)),
            ("to", pt_json(hit.end)),
            ("segmentIndex", hit.left_segment.into()),
            ("otherSegmentIndex", hit.right_segment.into()),
        ]);
        emit(
            sink,
            Diagnostic::new("composition/ambiguous-corridor", severity, &message)
                .with_subject(rel_subject(ctx, left))
                .with_evidence(ev)
                .with_fixes([hint]),
        );
    }
}

// ---- arrowhead collision -----------------------------------------------------------------------

/// Two incoming arrowheads on one side of a destination closer than `3.5 * (sw1 + sw2)`.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrowheadHit {
    /// The earlier sibling, then the later one (positions in `rels`).
    pub left: usize,
    pub right: usize,
    pub distance: f64,
    pub minimum: f64,
    pub left_tip: Pt,
    pub right_tip: Pt,
}

fn sign(x: f64) -> i8 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

/// `collectArrowheadCollisions` over the relations `eligible` selects (positions in `rels`).
pub fn collect_arrowhead_collisions(rels: &[Rel], eligible: &dyn Fn(usize) -> bool) -> Vec<ArrowheadHit> {
    collect_arrowhead_collisions_with(rels, eligible, &|_, _, _, _, _| false)
}

/// The shared stretch of two last segments an arrowhead pair may keep: `(left, right, left route,
/// right route, overlap length)`, routes normalised (`shortWorkflowTrunk`, workflow v2).
pub type ArrowExempt<'a> = dyn Fn(usize, usize, &[Pt], &[Pt], f64) -> bool + 'a;

/// [`collect_arrowhead_collisions`] with `allowShortWorkflowTrunks`.
pub fn collect_arrowhead_collisions_with(rels: &[Rel], eligible: &dyn Fn(usize) -> bool, exempt: &ArrowExempt<'_>) -> Vec<ArrowheadHit> {
    struct Incoming {
        rel: usize,
        tip: Pt,
        half_width: f64,
        points: Vec<Pt>,
    }
    type Key<'a> = (&'a str, usize, i8);
    let mut groups: Vec<(Key<'_>, Vec<Incoming>)> = Vec::new();
    let mut hits = Vec::new();
    for (pos, rel) in rels.iter().enumerate() {
        if !eligible(pos) || rel.to.is_empty() {
            continue;
        }
        let points = normalize(&rel.points);
        if points.len() < 2 {
            continue;
        }
        let tip = points[points.len() - 1];
        let previous = points[points.len() - 2];
        let vertical = (tip[0] - previous[0]).abs() < EPS;
        let axis = usize::from(!vertical);
        let direction = sign(tip[1 - axis] - previous[1 - axis]);
        let half_width = 3.5 * rel.arrow_width;
        let key = (rel.to.as_str(), axis, direction);
        let at = match groups.iter().position(|(k, _)| *k == key) {
            Some(at) => at,
            None => {
                groups.push((key, Vec::new()));
                groups.len() - 1
            }
        };
        for sibling in &groups[at].1 {
            if (tip[1 - axis] - sibling.tip[1 - axis]).abs() > EPS {
                continue;
            }
            let distance = (tip[axis] - sibling.tip[axis]).abs();
            let minimum = half_width + sibling.half_width;
            if distance < minimum - EPS {
                let n = sibling.points.len();
                let overlap = collinear_axis_overlap(sibling.points[n - 2], sibling.points[n - 1], previous, tip);
                if overlap.is_some_and(|o| exempt(sibling.rel, pos, &sibling.points, &points, o.length)) {
                    continue;
                }
                hits.push(ArrowheadHit { left: sibling.rel, right: pos, distance, minimum, left_tip: sibling.tip, right_tip: tip });
            }
        }
        groups[at].1.push(Incoming { rel: pos, tip, half_width, points });
    }
    hits
}

/// The architecture's showcase-only arrowhead check (`render-architecture.mjs:701`): an error
/// with the sentence and subject the renderer records (no `index` in the subject).
pub fn arrowhead_collision(ctx: &Ctx<'_>, rels: &[Rel], eligible: &dyn Fn(usize) -> bool, gate: Gate, sink: &mut Sink) {
    if !gate.showcase() {
        return;
    }
    for hit in collect_arrowhead_collisions(rels, eligible) {
        let (left, right) = (&rels[hit.left], &rels[hit.right]);
        let name = |r: &Rel| r.id.clone().unwrap_or_else(|| r.from.clone());
        let message = format!(
            "[composition/arrowhead-collision] automatic {} \"{}\" and \"{}\" into \"{}\" have arrowheads {}px apart (minimum {}px) \u{2014} enlarge or reposition the destination, or choose separate toSide ports.",
            ctx.collection,
            name(left),
            name(right),
            left.to,
            super::num(hit.distance),
            super::num(hit.minimum),
        );
        let mut subject = crate::diag::Subject::of(ctx.diagram);
        subject.collection = Some(ctx.collection.to_owned());
        subject.id = left.id.clone();
        let subject = subject.with_extra("from", left.from.clone().into()).with_extra("to", left.to.clone().into());
        let mut ev = evidence([
            ("distancePx", json_num(hit.distance)),
            ("minimumPx", json_num(hit.minimum)),
            ("endpoints", Value::Array(vec![pt_json(hit.left_tip), pt_json(hit.right_tip)])),
        ]);
        if let Some(id) = &right.id {
            ev.insert("otherId".to_owned(), id.clone().into());
        }
        sink.detailed(
            Diagnostic::error("composition/arrowhead-collision", &message)
                .with_subject(subject)
                .with_evidence(ev)
                .with_fixes(["enlarge or reposition the destination", "choose separate toSide ports"]),
        );
    }
}

// ---- container border run ----------------------------------------------------------------------

/// A route that follows a frame's straight side.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderRun {
    pub rel: usize,
    pub frame: usize,
    pub side: BorderSide,
    pub segment: usize,
    pub length: f64,
    pub start: Pt,
    pub end: Pt,
}

fn border_side_name(side: BorderSide) -> &'static str {
    match side {
        BorderSide::Top => "top",
        BorderSide::Right => "right",
        BorderSide::Bottom => "bottom",
        BorderSide::Left => "left",
        BorderSide::Line => "line",
    }
}

/// `collectBorderRuns` over the raw routes: per frame side, the overlaps are merged per axis
/// interval; the hit reports the longest interval and the summed length.
pub fn collect_border_runs(rels: &[Rel], frames: &[FrameInfo]) -> Vec<BorderRun> {
    let routes: Vec<Vec<Seg>> = rels
        .iter()
        .map(|rel| {
            if rel.points.len() < 2 || !rel.points.iter().all(|p| is_finite_point(p)) {
                Vec::new()
            } else {
                rel.points.windows(2).map(|w| Seg::new(w[0], w[1])).collect()
            }
        })
        .collect();
    collect_border_runs_over(&routes, frames)
}

/// `collectBorderRuns` over routes given as segments (`routed.segments`): the artifact checker passes
/// the straight pieces of the rendered path, not the route points. `BorderRun::rel` is the position
/// in `routes`; a route with no segment is skipped.
pub fn collect_border_runs_over(routes: &[Vec<Seg>], frames: &[FrameInfo]) -> Vec<BorderRun> {
    let mut hits = Vec::new();
    for (ri, route) in routes.iter().enumerate() {
        if route.is_empty() || !route.iter().all(|s| is_finite_point(&[s.start[0], s.start[1], s.end[0], s.end[1]])) {
            continue;
        }
        for (fi, info) in frames.iter().enumerate() {
            for border in frame_border_segments(&info.frame) {
                let overlaps: Vec<(usize, Pt, Pt, f64)> = route
                    .iter()
                    .enumerate()
                    .filter_map(|(si, s)| {
                        let o = collinear_axis_overlap(s.start, s.end, border.start, border.end)?;
                        (o.length > EPS).then_some((si, o.start, o.end, o.length))
                    })
                    .collect();
                if overlaps.is_empty() {
                    continue;
                }
                let horizontal = (border.start[1] - border.end[1]).abs() <= EPS;
                let axis = usize::from(!horizontal);
                let fixed = if horizontal { border.start[1] } else { border.start[0] };
                let mut intervals: Vec<(f64, f64)> = overlaps
                    .iter()
                    .map(|(_, s, e, _)| (s[axis].min(e[axis]), s[axis].max(e[axis])))
                    .collect();
                intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
                let mut merged: Vec<(f64, f64)> = Vec::new();
                for (low, high) in intervals {
                    match merged.last_mut() {
                        Some(prev) if low <= prev.1 + EPS => prev.1 = prev.1.max(high),
                        _ => merged.push((low, high)),
                    }
                }
                let total: f64 = merged.iter().map(|(l, h)| h - l).sum();
                let mut by_length = merged.clone();
                by_length.sort_by(|a, b| (b.1 - b.0).total_cmp(&(a.1 - a.0)).then(a.0.total_cmp(&b.0)));
                let (low, high) = by_length[0];
                let point = |v: f64| if horizontal { [v, fixed] } else { [fixed, v] };
                hits.push(BorderRun {
                    rel: ri,
                    frame: fi,
                    side: border.side,
                    segment: overlaps.iter().map(|o| o.0).min().unwrap_or(0),
                    length: total,
                    start: point(low),
                    end: point(high),
                });
            }
        }
    }
    hits
}

/// `cleanBorderRunProblems`: an error under any declared profile, silent with none.
pub fn container_border_run(ctx: &Ctx<'_>, rels: &[Rel], frames: &[FrameInfo], gate: Gate, route_hint: &str, sink: &mut Sink) {
    if !gate.declared() {
        return;
    }
    for hit in collect_border_runs(rels, frames) {
        let rel = &rels[hit.rel];
        let info = &frames[hit.frame];
        let length = round1(hit.length);
        let identity = info.label.clone().filter(|l| !l.is_empty()).unwrap_or_else(|| {
            let id = id_text(&info.id);
            if id.is_empty() { hit.frame.to_string() } else { id }
        });
        let hint = re_plan_hint(&[rel], route_hint);
        let message = format!(
            "[composition/container-border-run] {} {} follows {} \"{identity}\" {} border for {}px on segment {} [{}] -> [{}] — {hint}.",
            ctx.diagram,
            describe(ctx, rel),
            info.kind,
            border_side_name(hit.side),
            super::num(length),
            hit.segment,
            point1(hit.start),
            point1(hit.end),
        );
        let mut ev = Map::new();
        ev.insert("frameKind".into(), info.kind.clone().into());
        ev.insert("frameId".into(), info.id.clone());
        if let Some(label) = &info.label {
            ev.insert("frameLabel".into(), label.clone().into());
        }
        ev.insert("side".into(), border_side_name(hit.side).into());
        ev.insert("segmentIndex".into(), hit.segment.into());
        ev.insert("overlapLengthPx".into(), json_num(length));
        ev.insert("from".into(), pt_json(hit.start));
        ev.insert("to".into(), pt_json(hit.end));
        sink.error(
            Diagnostic::error("composition/container-border-run", &message)
                .with_subject(rel_subject(ctx, rel))
                .with_evidence(ev)
                .with_fixes([hint]),
        );
    }
}

// ---- route rhythm ------------------------------------------------------------------------------

/// Where a segment sits in its route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    SourceStub,
    Interior,
    TargetStub,
}

impl Position {
    pub fn name(self) -> &'static str {
        match self {
            Position::SourceStub => "source-stub",
            Position::Interior => "interior",
            Position::TargetStub => "target-stub",
        }
    }
}

fn segment_position(index: usize, count: usize) -> Position {
    if index == 0 {
        Position::SourceStub
    } else if index + 1 == count {
        Position::TargetStub
    } else {
        Position::Interior
    }
}

/// A segment below the micro (8) or interior (16) floor.
#[derive(Clone, Debug, PartialEq)]
pub struct RhythmIssue {
    pub micro: bool,
    pub rel: usize,
    pub segment: usize,
    pub position: Position,
    pub length: f64,
    pub start: Pt,
    pub end: Pt,
}

pub const MICRO_SEGMENT_PX: f64 = 8.0;
pub const INTERIOR_SEGMENT_PX: f64 = 16.0;

/// `collectRouteRhythmIssues` on normalised routes (Manhattan length).
pub fn collect_rhythm(rels: &[Rel]) -> Vec<RhythmIssue> {
    let mut issues = Vec::new();
    for (ri, rel) in rels.iter().enumerate() {
        let points = normalize(&rel.points);
        if points.len() < 2 {
            continue;
        }
        let count = points.len() - 1;
        for (si, w) in points.windows(2).enumerate() {
            let length = (w[1][0] - w[0][0]).abs() + (w[1][1] - w[0][1]).abs();
            if length <= EPS {
                continue;
            }
            let position = segment_position(si, count);
            let micro = length < MICRO_SEGMENT_PX - EPS;
            if micro || (position == Position::Interior && length < INTERIOR_SEGMENT_PX - EPS) {
                issues.push(RhythmIssue { micro, rel: ri, segment: si, position, length, start: w[0], end: w[1] });
            }
        }
    }
    issues
}

/// `cleanRouteRhythmProblems`: showcase only.
pub fn route_rhythm(ctx: &Ctx<'_>, rels: &[Rel], gate: Gate, route_hint: &str, sink: &mut Sink) {
    if !gate.showcase() {
        return;
    }
    for hit in collect_rhythm(rels) {
        let rel = &rels[hit.rel];
        let (code, rule, minimum) = if hit.micro {
            ("composition/micro-segment", format!("is below the {}px micro-segment floor", super::num(MICRO_SEGMENT_PX)), MICRO_SEGMENT_PX)
        } else {
            (
                "composition/short-interior-segment",
                format!("is below the {}px interior-segment floor", super::num(INTERIOR_SEGMENT_PX)),
                INTERIOR_SEGMENT_PX,
            )
        };
        let length = round1(hit.length);
        let hint = re_plan_hint(&[rel], route_hint);
        let message = format!(
            "[{code}] showcase {} {} has a {}px {} segment {} [{}] -> [{}] that {rule} — {hint}.",
            ctx.diagram,
            describe(ctx, rel),
            super::num(length),
            hit.position.name(),
            hit.segment,
            point1(hit.start),
            point1(hit.end),
        );
        let ev = evidence([
            ("segmentIndex", hit.segment.into()),
            ("position", hit.position.name().into()),
            ("lengthPx", json_num(length)),
            ("minimumPx", json_num(minimum)),
            ("from", pt_json(hit.start)),
            ("to", pt_json(hit.end)),
        ]);
        sink.error(
            Diagnostic::error(code, &message).with_subject(rel_subject(ctx, rel)).with_evidence(ev).with_fixes([hint]),
        );
    }
}

// ---- label / route clearance -------------------------------------------------------------------

/// A label rect closer than `threshold` to another relation's route.
#[derive(Clone, Debug, PartialEq)]
pub struct ClearanceHit {
    /// Index into the labels slice.
    pub label: usize,
    /// Index into the rels slice of the other relation.
    pub other: usize,
    pub clearance: f64,
    pub intersection: f64,
    pub segment: usize,
    pub start: Pt,
    pub end: Pt,
    pub threshold: f64,
}

/// `collectLabelRouteClearance` (normalised routes; one result per label and foreign route).
pub fn collect_label_clearance(labels: &[LabelRect], rels: &[Rel], threshold: f64) -> Vec<ClearanceHit> {
    if !threshold.is_finite() || threshold <= 0.0 {
        return Vec::new();
    }
    let routes: Vec<(usize, Vec<Pt>)> = rels
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let points = normalize(&r.points);
            (points.len() >= 2).then_some((i, points))
        })
        .collect();
    let mut hits = Vec::new();
    for (li, label) in labels.iter().enumerate() {
        let rect = label.rect;
        if !rect.is_finite() || rect.width < 0.0 || rect.height < 0.0 {
            continue;
        }
        for (ri, points) in &routes {
            if rels[*ri].index == label.rel {
                continue;
            }
            let mut nearest: Option<ClearanceHit> = None;
            for (si, w) in points.windows(2).enumerate() {
                let seg = Seg::new(w[0], w[1]);
                let Some(clearance) = segment_rect_clearance(&seg, &rect) else { continue };
                if nearest.as_ref().is_none_or(|n| clearance < n.clearance) {
                    nearest = Some(ClearanceHit {
                        label: li,
                        other: *ri,
                        clearance,
                        intersection: segment_rect_intersection_length(&seg, &rect).unwrap_or(0.0),
                        segment: si,
                        start: w[0],
                        end: w[1],
                        threshold,
                    });
                }
            }
            if let Some(n) = nearest
                && n.clearance + EPS < threshold
            {
                hits.push(n);
            }
        }
    }
    hits
}

fn rel_of(rels: &[Rel], index: usize) -> Option<&Rel> {
    rels.iter().find(|r| r.index == index)
}

/// The minimum label/route clearance over every pair, rounded to 0.1 (`minimumLabelRouteClearance`).
pub fn min_label_clearance(labels: &[LabelRect], rels: &[Rel]) -> Option<f64> {
    collect_label_clearance(labels, rels, f64::MAX)
        .iter()
        .map(|h| h.clearance)
        .reduce(f64::min)
        .map(round1)
}

/// `cleanLabelRouteClearanceProblems`: showcase only, 4 px.
pub fn label_route_clearance(ctx: &Ctx<'_>, rels: &[Rel], labels: &[LabelRect], gate: Gate, route_hint: &str, sink: &mut Sink) {
    if !gate.showcase() {
        return;
    }
    const THRESHOLD: f64 = 4.0;
    for hit in collect_label_clearance(labels, rels, THRESHOLD) {
        let label = &labels[hit.label];
        let (Some(owner), other) = (rel_of(rels, label.rel), &rels[hit.other]) else { continue };
        let name = |r: &Rel| {
            let text = if r.label.is_empty() { String::new() } else { format!(" label \"{}\"", r.label) };
            format!("{}{text}", describe(ctx, r))
        };
        let clearance = round1(hit.clearance);
        let hint = re_plan_hint(&[owner, other], route_hint);
        let message = format!(
            "[composition/label-route-clearance] showcase {} label \"{}\" on {} is {}px from {} segment {} [{}] -> [{}] (label rect {}; minimum {}px) — {hint}.",
            ctx.diagram,
            label.text,
            name(owner),
            super::num(clearance),
            name(other),
            hit.segment,
            point1(hit.start),
            point1(hit.end),
            format_rect(&label.rect),
            super::num(THRESHOLD),
        );
        let ev = evidence([
            ("label", label.text.clone().into()),
            ("otherRelationship", rel_subject_json(ctx, other)),
            ("segmentIndex", hit.segment.into()),
            ("clearancePx", json_num(clearance)),
            ("minimumPx", json_num(THRESHOLD)),
            ("labelRect", rect_json(&label.rect)),
            ("from", pt_json(hit.start)),
            ("to", pt_json(hit.end)),
        ]);
        sink.error(
            Diagnostic::error("composition/label-route-clearance", &message)
                .with_subject(rel_subject(ctx, owner))
                .with_evidence(ev)
                .with_fixes([hint]),
        );
    }
}

// ---- label canvas containment ------------------------------------------------------------------

/// A label rect past the canvas by more than the tolerance.
#[derive(Clone, Debug, PartialEq)]
pub struct Overflow {
    pub label: usize,
    /// `(side, px)` in `left, right, top, bottom` order, px rounded to 0.1.
    pub sides: Vec<(&'static str, f64)>,
}

pub const LABEL_TOLERANCE_PX: f64 = 0.5;

/// `collectLabelCanvasOverflow` for an origin-zero canvas.
pub fn collect_label_overflow(labels: &[LabelRect], view_box: [f64; 2], tolerance: f64) -> Vec<Overflow> {
    let mut hits = Vec::new();
    for (li, label) in labels.iter().enumerate() {
        let r: Rect = label.rect;
        if !r.is_finite() || r.width < 0.0 || r.height < 0.0 {
            continue;
        }
        let overflow = [
            ("left", -r.x),
            ("right", r.x + r.width - view_box[0]),
            ("top", -r.y),
            ("bottom", r.y + r.height - view_box[1]),
        ];
        let sides: Vec<(&'static str, f64)> =
            overflow.into_iter().filter(|(_, px)| *px > tolerance).map(|(s, px)| (s, round1(px))).collect();
        if !sides.is_empty() {
            hits.push(Overflow { label: li, sides });
        }
    }
    hits
}

/// `describeLabelCanvasOverflow`.
pub fn describe_overflow(hit: &Overflow) -> String {
    hit.sides.iter().map(|(s, px)| format!("{s} edge by {}px", super::num(*px))).collect::<Vec<_>>().join(" and ")
}

/// `cleanLabelCanvasContainmentProblems`: showcase only.
pub fn label_canvas_containment(
    ctx: &Ctx<'_>,
    rels: &[Rel],
    labels: &[LabelRect],
    view_box: [f64; 2],
    gate: Gate,
    sink: &mut Sink,
) {
    if !gate.showcase() {
        return;
    }
    const HINT: &str = "adjust labelAt, labelDx, labelDy, or labelSegment; otherwise enlarge meta.viewBox";
    for hit in collect_label_overflow(labels, view_box, LABEL_TOLERANCE_PX) {
        let label = &labels[hit.label];
        let Some(rel) = rel_of(rels, label.rel) else { continue };
        let hint = re_plan_hint(&[rel], HINT);
        let message = format!(
            "[composition/label-canvas-containment] showcase {} label \"{}\" on {} extends past the {} (label rect {}; viewBox {}x{}) — {hint}.",
            ctx.diagram,
            label.text,
            describe(ctx, rel),
            describe_overflow(&hit),
            format_rect(&label.rect),
            super::num(view_box[0]),
            super::num(view_box[1]),
        );
        let overflow: Map<String, Value> = hit.sides.iter().map(|(s, px)| ((*s).to_owned(), json_num(*px))).collect();
        let ev = evidence([
            ("label", label.text.clone().into()),
            ("labelRect", rect_json(&label.rect)),
            ("viewBox", json!([json_num(view_box[0]), json_num(view_box[1])])),
            ("overflowPx", Value::Object(overflow)),
            ("tolerancePx", json_num(LABEL_TOLERANCE_PX)),
        ]);
        sink.error(
            Diagnostic::error("composition/label-canvas-containment", &message)
                .with_subject(rel_subject(ctx, rel))
                .with_evidence(ev)
                .with_fixes([hint]),
        );
    }
}

// ---- route budget metrics and the receipt block ------------------------------------------------

/// `routeBudgetMetrics` with the suggested limits (2 bends, 1.35 stretch, 16 and 8 px).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RouteMetrics {
    pub max_bends: usize,
    pub routes_over_bends: usize,
    pub max_stretch: Option<f64>,
    pub routes_over_stretch: usize,
    pub min_segment: Option<f64>,
    pub min_interior_segment: Option<f64>,
    pub short_segments: usize,
    pub short_endpoint_segments: usize,
    pub short_interior_segments: usize,
    pub micro_segments: usize,
}

/// `routeBudgetMetrics({ routedRelations })`.
pub fn route_metrics(rels: &[Rel]) -> RouteMetrics {
    let mut m = RouteMetrics::default();
    for rel in rels {
        let points = normalize(&rel.points);
        if points.len() < 2 {
            continue;
        }
        let bends = points.len() - 2;
        m.max_bends = m.max_bends.max(bends);
        if bends > 2 {
            m.routes_over_bends += 1;
        }
        let mut route_length = 0.0;
        for (si, w) in points.windows(2).enumerate() {
            let length = (w[1][0] - w[0][0]).abs() + (w[1][1] - w[0][1]).abs();
            if length <= EPS {
                continue;
            }
            let position = segment_position(si, points.len() - 1);
            route_length += length;
            m.min_segment = Some(m.min_segment.map_or(length, |v| v.min(length)));
            if position == Position::Interior {
                m.min_interior_segment = Some(m.min_interior_segment.map_or(length, |v| v.min(length)));
            }
            if length < INTERIOR_SEGMENT_PX {
                m.short_segments += 1;
                if position == Position::Interior {
                    m.short_interior_segments += 1;
                } else {
                    m.short_endpoint_segments += 1;
                }
            }
            if length < MICRO_SEGMENT_PX {
                m.micro_segments += 1;
            }
        }
        let last = points[points.len() - 1];
        let direct = (last[0] - points[0][0]).abs() + (last[1] - points[0][1]).abs();
        if direct > EPS {
            let stretch = route_length / direct;
            m.max_stretch = Some(m.max_stretch.map_or(stretch, |v| v.max(stretch)));
            if stretch > 1.35 + EPS {
                m.routes_over_stretch += 1;
            }
        }
    }
    m
}

fn opt_num(v: Option<f64>) -> Value {
    v.map_or(Value::Null, json_num)
}

fn rel_record(rel: &Rel) -> Value {
    json!({"id": rel.id, "from": rel.from, "to": rel.to, "label": rel.label,
        "collectionIndex": rel.index, "artifactIndex": rel.index + 1})
}

/// The success receipt's `composition` block, from the gate collectors (the artifact checker's
/// `composition`, minus the two rules that need the reader's viewport: `desktop-readability` and
/// `viewport-height`, listed under `notMeasured`).
///
/// `reported` is the profile the receipt prints (`standard` by default); `gate` the one the renderer
/// saw. Severities are the artifact checker's: everything but the border run is an error only in
/// showcase, a border run is an error exactly when a profile is declared.
pub fn receipt(
    rels: &[Rel],
    labels: &[LabelRect],
    frames: &[FrameInfo],
    view_box: [f64; 2],
    reported: &str,
    gate: Gate,
) -> Value {
    let showcase = reported == "showcase";
    let enforced = gate.declared();
    let crossings = collect_crossings(rels);
    let corridors = collect_corridors(rels, 8.0);
    let borders = collect_border_runs(rels, frames);
    let rhythm = collect_rhythm(rels);
    let threshold = if showcase { 4.0 } else { 2.0 };
    let clearance = collect_label_clearance(labels, rels, threshold);
    let overflow = collect_label_overflow(labels, view_box, LABEL_TOLERANCE_PX);

    let sev = |error: bool| if error { "error" } else { "warning" };
    let mut issues: Vec<Value> = Vec::new();
    for h in &borders {
        let info = &frames[h.frame];
        issues.push(json!({"severity": sev(enforced), "code": "composition/container-border-run",
            "relationship": rel_record(&rels[h.rel]),
            "frame": {"kind": info.kind, "id": id_text(&info.id)},
            "side": border_side_name(h.side), "segmentIndex": h.segment,
            "overlapLength": round1(h.length), "from": pt_json(h.start), "to": pt_json(h.end)}));
    }
    for h in &clearance {
        let label = &labels[h.label];
        issues.push(json!({"severity": sev(showcase), "code": "composition/label-route-clearance",
            "label": label.text,
            "labelRelationship": rel_of(rels, label.rel).map(rel_record),
            "otherRelationship": rel_record(&rels[h.other]),
            "segmentIndex": h.segment, "labelRect": rect_json(&label.rect),
            "clearance": round1(h.clearance), "intersectionLength": round1(h.intersection),
            "threshold": h.threshold, "from": pt_json(h.start), "to": pt_json(h.end)}));
    }
    for h in &overflow {
        let label = &labels[h.label];
        issues.push(json!({"severity": sev(showcase), "code": "composition/label-canvas-containment",
            "label": label.text, "relationship": rel_of(rels, label.rel).map(rel_record),
            "labelRect": rect_json(&label.rect), "viewBox": [view_box[0], view_box[1]],
            "overflowPx": h.sides.iter().map(|(s, px)| ((*s).to_owned(), json_num(*px))).collect::<Map<_, _>>()}));
    }
    for h in &crossings {
        issues.push(json!({"severity": sev(showcase), "code": "composition/proper-crossing",
            "relationship": rel_record(&rels[h.left]), "otherRelationship": rel_record(&rels[h.right]),
            "point": pt_json([round1(h.point[0]), round1(h.point[1])])}));
    }
    for h in &corridors {
        issues.push(json!({"severity": sev(showcase), "code": "composition/ambiguous-corridor",
            "relationship": rel_record(&rels[h.left]), "otherRelationship": rel_record(&rels[h.right]),
            "segmentIndex": h.left_segment, "otherSegmentIndex": h.right_segment,
            "overlapLength": round1(h.length), "from": pt_json(h.start), "to": pt_json(h.end)}));
    }
    for h in &rhythm {
        issues.push(json!({"severity": sev(showcase),
            "code": if h.micro { "composition/micro-segment" } else { "composition/short-interior-segment" },
            "relationship": rel_record(&rels[h.rel]), "segmentIndex": h.segment,
            "position": h.position.name(), "length": round1(h.length),
            "from": pt_json(h.start), "to": pt_json(h.end)}));
    }
    let count = |severity: &str| issues.iter().filter(|i| i["severity"] == severity).count();
    let (errors, warnings) = (count("error"), count("warning"));

    let m = route_metrics(rels);
    let detours: Vec<Value> = rels
        .iter()
        .filter_map(|rel| {
            let one = route_metrics(std::slice::from_ref(rel));
            (one.routes_over_bends > 0 || one.routes_over_stretch > 0).then(|| {
                json!({"relationship": rel_record(rel), "bends": one.max_bends,
                    "stretch": one.max_stretch.map(|s| (s * 1000.0).round() / 1000.0)})
            })
        })
        .collect();

    json!({
        "schemaVersion": 1,
        "profile": reported,
        "status": if errors > 0 { "fail" } else { "pass" },
        "summary": {"errors": errors, "warnings": warnings},
        "metrics": {
            "properCrossings": crossings.len(),
            "resolvedCrossovers": 0,
            "ambiguousCorridors": corridors.len(),
            "arrowheadCollisions": 0,
            "containerBorderRuns": borders.len(),
            "labelRouteClearanceIssues": clearance.len(),
            "labelCanvasOverflowIssues": overflow.len(),
            "minLabelRouteClearance": opt_num(min_label_clearance(labels, rels)),
            "maxBends": m.max_bends,
            "routesOverSuggestedBends": m.routes_over_bends,
            "maxStretch": opt_num(m.max_stretch.map(|s| (s * 1000.0).round() / 1000.0)),
            "routesOverSuggestedStretch": m.routes_over_stretch,
            "minSegmentPx": opt_num(m.min_segment.map(round1)),
            "minInteriorSegmentPx": opt_num(m.min_interior_segment.map(round1)),
            "shortSegmentCount": m.short_segments,
            "shortEndpointSegmentCount": m.short_endpoint_segments,
            "shortInteriorSegmentCount": m.short_interior_segments,
            "microSegmentCount": m.micro_segments,
        },
        "suggestedLimits": {"bendsPerRelationship": 2, "stretch": 1.35, "segmentPx": 16, "microSegmentPx": 8},
        "routeReview": {"crossings": [], "detours": detours},
        "issues": issues,
        "notMeasured": ["composition/desktop-readability", "composition/viewport-height"],
    })
}

/// [`receipt`] with every proper crossing of two `halo` relations (positions in `rels`: the
/// grid-routed transitions, the automatic architecture routes) moved to `resolvedCrossovers`, as
/// the artifact checker counts them.
pub fn receipt_resolving(
    rels: &[Rel],
    labels: &[LabelRect],
    frames: &[FrameInfo],
    view_box: [f64; 2],
    reported: &str,
    gate: Gate,
    halo: &[bool],
) -> Value {
    let mut block = receipt(rels, labels, frames, view_box, reported, gate);
    let resolved = collect_crossings(rels).iter().filter(|h| halo[h.left] && halo[h.right]).count();
    if resolved == 0 {
        return block;
    }
    let both_halo = |issue: &Value| {
        let index = |key: &str| issue[key]["collectionIndex"].as_u64().map(|i| i as usize);
        let has_halo = |i: Option<usize>| i.and_then(|i| rels.iter().position(|r| r.index == i)).is_some_and(|p| halo[p]);
        issue["code"] == "composition/proper-crossing" && has_halo(index("relationship")) && has_halo(index("otherRelationship"))
    };
    if let Some(issues) = block["issues"].as_array_mut() {
        issues.retain(|i| !both_halo(i));
    }
    let count = |severity: Severity| {
        let word = if severity == Severity::Error { "error" } else { "warning" };
        block["issues"].as_array().map_or(0, |a| a.iter().filter(|i| i["severity"] == word).count())
    };
    let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
    let proper = block["metrics"]["properCrossings"].as_u64().unwrap_or(0) as usize - resolved;
    block["metrics"]["properCrossings"] = proper.into();
    block["metrics"]["resolvedCrossovers"] = resolved.into();
    block["summary"] = json!({"errors": errors, "warnings": warnings});
    block["status"] = (if errors > 0 { "fail" } else { "pass" }).into();
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(index: usize, from: &str, to: &str, points: &[Pt]) -> Rel {
        Rel {
            index,
            id: None,
            from: from.into(),
            to: to.into(),
            label: format!("l{index}"),
            points: points.to_vec(),
            controls: vec![],
            arrow_width: 1.5,
            from_side: None,
            to_side: None,
        }
    }

    const CTX: Ctx<'static> = Ctx { diagram: "dataflow", collection: "flows" };

    fn run(f: impl Fn(&mut Sink)) -> Vec<Diagnostic> {
        let mut sink = Sink::default();
        f(&mut sink);
        sink.recorded
    }

    #[test]
    fn a_proper_x_is_an_error_in_showcase_and_silent_otherwise() {
        let rels = [
            rel(0, "a", "b", &[[0.0, 50.0], [100.0, 50.0]]),
            rel(1, "c", "d", &[[50.0, 0.0], [50.0, 100.0]]),
        ];
        let go = |gate| run(|s| proper_crossing(&CTX, &rels, gate, false, "hint", s));
        assert!(go(Gate(None)).is_empty() && go(Gate(Some("standard"))).is_empty());
        let found = go(Gate(Some("showcase")));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].code, "composition/proper-crossing");
        assert!(found[0].message.contains("crosses flows[1] \"c\" -> \"d\" at [50, 50] (segments 0 and 0)"));
        // Workflow v2's opt-in turns the standard silence into a recorded warning.
        let warned = run(|s| proper_crossing(&CTX, &rels, Gate(Some("standard")), true, "hint", s));
        assert_eq!(warned[0].severity, Severity::Warning);
        // A shared endpoint is a junction, not a crossing.
        let shared = [rels[0].clone(), rel(1, "a", "d", &[[50.0, 0.0], [50.0, 100.0]])];
        assert!(run(|s| proper_crossing(&CTX, &shared, Gate(Some("showcase")), false, "hint", s)).is_empty());
    }

    #[test]
    fn a_shared_corridor_needs_eight_px_and_no_common_endpoint() {
        let rels = [
            rel(0, "a", "b", &[[0.0, 50.0], [100.0, 50.0]]),
            rel(1, "c", "d", &[[60.0, 50.0], [200.0, 50.0]]),
        ];
        let found = run(|s| ambiguous_corridor(&CTX, &rels, Gate(Some("showcase")), "hint", s));
        assert_eq!(found.len(), 1);
        assert!(found[0].message.contains("shares a 40px corridor"), "{}", found[0].message);
        assert!(run(|s| ambiguous_corridor(&CTX, &rels, Gate(Some("standard")), "hint", s)).is_empty());
        let touching = [rels[0].clone(), rel(1, "c", "d", &[[96.0, 50.0], [200.0, 50.0]])];
        assert!(run(|s| ambiguous_corridor(&CTX, &touching, Gate(Some("showcase")), "hint", s)).is_empty());
    }

    #[test]
    fn a_border_run_is_an_error_under_any_declared_profile() {
        let frames = [FrameInfo {
            frame: Frame::Rect { rect: Rect::new(0.0, 0.0, 100.0, 100.0), radius: 10.0 },
            kind: "stage".into(),
            id: 0.into(),
            label: Some("Sources".into()),
        }];
        let rels = [rel(0, "a", "b", &[[100.0, 20.0], [100.0, 80.0]])];
        assert!(run(|s| container_border_run(&CTX, &rels, &frames, Gate(None), "hint", s)).is_empty());
        let found = run(|s| container_border_run(&CTX, &rels, &frames, Gate(Some("standard")), "hint", s));
        assert_eq!(found.len(), 1);
        assert!(found[0].message.contains("follows stage \"Sources\" right border for 60px"), "{}", found[0].message);
    }

    #[test]
    fn route_rhythm_and_label_rules_are_showcase_only() {
        let rels = [rel(0, "a", "b", &[[0.0, 0.0], [50.0, 0.0], [50.0, 7.0], [100.0, 7.0]])];
        assert!(run(|s| route_rhythm(&CTX, &rels, Gate(Some("standard")), "hint", s)).is_empty());
        let found = run(|s| route_rhythm(&CTX, &rels, Gate(Some("showcase")), "hint", s));
        assert_eq!(found[0].code, "composition/micro-segment");
        assert!(found[0].message.contains("has a 7px interior segment 1"), "{}", found[0].message);

        let labels = [LabelRect {
            rel: 0,
            text: "l0".into(),
            rect: Rect::new(90.0, 0.0, 30.0, 16.0),
            anchor: [105.0, 11.0],
            authored_at: false,
            authored_dx: 0.0,
            authored_dy: 0.0,
        }];
        assert!(run(|s| label_canvas_containment(&CTX, &rels, &labels, [100.0, 100.0], Gate(Some("standard")), s)).is_empty());
        let found = run(|s| label_canvas_containment(&CTX, &rels, &labels, [100.0, 100.0], Gate(Some("showcase")), s));
        assert!(found[0].message.contains("extends past the right edge by 20px"), "{}", found[0].message);
    }

    #[test]
    fn the_receipt_block_counts_what_the_collectors_find() {
        let rels = [
            rel(0, "a", "b", &[[0.0, 50.0], [100.0, 50.0]]),
            rel(1, "c", "d", &[[50.0, 0.0], [50.0, 100.0]]),
        ];
        let block = receipt(&rels, &[], &[], [200.0, 200.0], "showcase", Gate(Some("showcase")));
        assert_eq!(block["status"], "fail");
        assert_eq!(block["metrics"]["properCrossings"], 1);
        let block = receipt(&rels, &[], &[], [200.0, 200.0], "standard", Gate(None));
        assert_eq!(block["status"], "pass");
        assert_eq!(block["summary"]["warnings"], 1);
    }
}
