//! `validateArchitecture` (`render-architecture.mjs:430-770`): every problem of one architecture
//! document, in Archify's order, graph rules and geometry rules together, measured on the geometry
//! [`crate::layout::architecture_build::build`] produces (Archify validates after its module scope
//! has routed and placed every label, so the relocated labels are what the label rules see).
//!
//! In order: the title-convergence problem, unique ids, grid (or free) placement, per component the
//! finite geometry, bounds, label width, brand rail, sublabel/tag fit; components 8 apart; wraps
//! that name no component; the boundary-title composition (a profile only) and, under
//! `deployment-ownership`, boundary nesting; boundary bounds (a recorded diagnostic); per
//! connection the endpoints, the showcase route bounds, the self-loop ports and the 24 px length;
//! then the shared gates (`clean-flow/*`, crossings (two automatic routes are resolved by their
//! halos), corridors (automatic routes without `labelAt` also across a shared endpoint), the
//! showcase arrowhead check, border runs against the final frames, route rhythm, the detour oracle),
//! the label/component and label-gap rules, label vs boundary titles, label/route clearance and
//! label canvas containment.
//!
//! Profiles (`00` section 3.3): the shared gates take [`Gate`] (the `--quality` override, else the
//! authored profile). The title contract, the route bounds, the label gap and the detour oracle read
//! the authored `meta.quality_profile` only, exactly as the renderer does.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::clean_flow::{edge_through_node, endpoint_side};
use super::composition::{
    FrameInfo, ambiguous_corridor_by, arrowhead_collision, container_border_run, label_canvas_containment,
    label_route_clearance, proper_crossing_resolved, receipt_resolving, route_rhythm,
};
use super::suggest::{component_separation, label_obstacle_fix};
use super::{
    CheckedSide, Ctx, Gate, LabelRect, Obstacle, Outcome, Rel, SideOrigin, Sink, evidence, num, pt_json, rect_json,
};
use crate::diag::{Diagnostic, Subject, js_round, json_num};
use crate::geom::{Frame, Rect, is_finite_point, rects_overlap};
use crate::layout::architecture::{PlacedBoundary, grid_of, grid_problems};
use crate::layout::architecture_build::{ArchitectureGeometry, ConnectionGeom, side as model_side};
use crate::layout_json::component_box;
use crate::model::architecture::{Architecture, BoundaryKind, Connection, ConnectionRoute, EngineeringProfile};
use crate::model::common::QualityProfile;
use crate::route::grid::{DetourRelation, DetourThresholds, clean_route_detour_problems};
use crate::text::{self, DiagramType};

const CTX: Ctx<'static> = Ctx { diagram: "architecture", collection: "connections" };

/// Components closer than this overlap (`rectsOverlap(a, b, 8)`).
const COMPONENT_GAP: f64 = 8.0;
/// A connection shorter than this between its end points is an error.
const MIN_CONNECTION: f64 = 24.0;
/// `componentTextFit.sublabelMinimum` / `tagMinimum`, and the brand rail check's minimum font.
const TEXT_MINIMUM: f64 = 6.0;
const BRAND_MINIMUM: f64 = 8.0;
/// `rectsOverlap(label, component, -2)`: a label may touch a component by 2 px.
const LABEL_GAP: f64 = -2.0;

/// What the gates measured, for the success receipt and for callers that want the numbers.
#[derive(Debug, Default, Clone)]
pub struct Review {
    pub rels: Vec<Rel>,
    pub labels: Vec<LabelRect>,
    pub frames: Vec<FrameInfo>,
    pub view_box: [f64; 2],
    /// Per entry of `rels`: an automatic route (drawn with the crossover halo).
    pub halo: Vec<bool>,
}

/// The rule id (D7) a plain sentence of `validateArchitecture` carries in `subject.rule`.
fn rule_of(message: &str) -> &'static str {
    let has = |s: &str| message.contains(s);
    if message == "Component ids must be unique." {
        "graph/duplicate-node-id"
    } else if has("needs pos [x,y] or grid") || has("must include pos [x, y] when layout.mode") {
        "architecture/placement-missing"
    } else if has("row/col must be non-negative") || has("exceeds layout.cols") || has("share grid cell") {
        "architecture/grid-cell"
    } else if has("non-finite pos/size") || has("has invalid size") {
        "geom/non-finite"
    } else if has("falls outside the viewBox") {
        "geom/node-out-of-bounds"
    } else if has("is wider than component") {
        "geom/label-too-wide"
    } else if has("brand top rail") {
        "geom/brand-rail"
    } else if has("legible minimum") {
        "geom/text-min-fit"
    } else if has("apart \u{2014} move one") {
        "geom/node-overlap"
    } else if has("wraps unknown component") {
        "graph/unknown-container"
    } else if has("references unknown") {
        "graph/unknown-endpoint"
    } else if has("is too short") {
        "geom/edge-too-short"
    } else if message.starts_with("Label \"") && has("overlaps component") {
        "geom/label-node-overlap"
    } else if message.starts_with("Boundary") || message.starts_with("[composition/desktop-readability]") {
        "architecture/boundary-title"
    } else {
        "layout/constraint"
    }
}

fn controls(c: &Connection) -> Vec<&'static str> {
    [
        ("route", c.route.is_some()),
        ("via", c.via.is_some()),
        ("fromSide", c.from_side.is_some()),
        ("toSide", c.to_side.is_some()),
        ("labelAt", c.label_at.is_some()),
        ("labelDx", c.label_dx.is_some()),
        ("labelDy", c.label_dy.is_some()),
        ("labelSegment", c.label_segment.is_some()),
    ]
    .into_iter()
    .filter_map(|(k, present)| present.then_some(k))
    .collect()
}

/// The sides `cleanEndpointSideProblems` checks: the authored one, else (no `route`/`via`) the one
/// the router chose.
fn sides_to_check(c: &Connection, g: &ConnectionGeom) -> (CheckedSide, CheckedSide) {
    let geometry = c.route.is_some_and(|r| r != ConnectionRoute::Auto) || c.via.is_some();
    let pick = |authored: Option<crate::model::common::Side>, chosen: crate::geom::Side| {
        authored
            .map(|s| (model_side(s), SideOrigin::Authored))
            .or_else(|| (!geometry).then_some((chosen, SideOrigin::Inferred)))
    };
    (pick(c.from_side, g.sides.from), pick(c.to_side, g.sides.to))
}

fn rect_contains(outer: &Rect, inner: &Rect) -> bool {
    const EPS: f64 = 1e-9;
    outer.x <= inner.x + EPS
        && outer.y <= inner.y + EPS
        && outer.x + outer.width + EPS >= inner.x + inner.width
        && outer.y + outer.height + EPS >= inner.y + inner.height
}

/// `Number(x.toFixed(2))` printed as JS prints it.
fn fixed2(x: f64) -> String {
    num(format!("{x:.2}").parse().unwrap_or(x))
}

fn connection_subject(index: usize, c: &Connection) -> Subject {
    let mut s = Subject::of("architecture");
    s.collection = Some("connections".to_owned());
    s.index = Some(index as i64);
    s.id = c.id.clone();
    s.with_extra("from", c.from.clone().into()).with_extra("to", c.to.clone().into())
}

/// The subject of a record that names the connection without its index (`label-gap`).
fn connection_subject_unindexed(c: &Connection) -> Subject {
    let mut s = connection_subject(0, c);
    s.index = None;
    s
}

fn side_name(s: crate::geom::Side) -> &'static str {
    match s {
        crate::geom::Side::Left => "left",
        crate::geom::Side::Right => "right",
        crate::geom::Side::Top => "top",
        crate::geom::Side::Bottom => "bottom",
    }
}

/// Every problem of `doc` under `gate`, plus what was measured. `g` is the geometry of
/// [`crate::layout::architecture_build::build`], or [`ArchitectureGeometry::unrouted`] when that
/// refused (a component with no finite position).
pub fn validate(doc: &Architecture, g: &ArchitectureGeometry, gate: Gate) -> (Outcome, Review) {
    let p = &g.placement;
    let view_box = p.view_box;
    let meta_showcase = doc.meta.quality_profile == Some(QualityProfile::Showcase);
    let enforces = p.enforces_titles;
    let nested_membership = doc.meta.engineering_profile == Some(EngineeringProfile::DeploymentOwnership);
    let connections: &[Connection] = doc.connections.as_deref().unwrap_or(&[]);
    let mut sink = Sink::default();

    if let Some(problem) = &p.readability_problem {
        sink.problem(problem.clone());
    }
    if p.components.len() != doc.components.len() {
        sink.problem("Component ids must be unique.".to_owned());
    }
    if grid_of(doc).is_some() {
        for problem in grid_problems(doc) {
            sink.problem(problem);
        }
    } else {
        for c in &doc.components {
            if c.pos.is_none() {
                sink.problem(format!(
                    "Component \"{}\" must include pos [x, y] when layout.mode is omitted (free placement).",
                    c.id
                ));
            }
        }
    }

    for pc in &p.components {
        let c = &doc.components[pc.index];
        let r = pc.rect;
        if !is_finite_point(&[r.x, r.y, r.width, r.height]) {
            sink.problem(format!("Component \"{}\" has non-finite pos/size \u{2014} pos and size must be [number, number].", c.id));
            continue;
        }
        if r.width <= 0.0 || r.height <= 0.0 {
            sink.problem(format!(
                "Component \"{}\" has invalid size {}x{} \u{2014} width and height must be greater than 0.",
                c.id,
                num(r.width),
                num(r.height)
            ));
            continue;
        }
        if r.x < 0.0 || r.y < 0.0 || r.x + r.width > view_box[0] || r.y + r.height > view_box[1] {
            sink.problem(format!(
                "Component \"{}\" falls outside the viewBox {}x{} \u{2014} adjust pos/size or set a larger meta.viewBox.",
                c.id,
                num(view_box[0]),
                num(view_box[1])
            ));
        }
        if text::label_too_wide(DiagramType::Architecture, &c.label, r.width) {
            let estimate = text::units(&c.label) as f64 * 6.6;
            sink.problem(format!(
                "Label \"{}\" (~{}px) is wider than component \"{}\" ({}px) \u{2014} shorten the label or widen size.",
                c.label,
                num(js_round(estimate)),
                c.id,
                num(r.width)
            ));
        }
        if c.brand.is_some()
            && let Some((available, required)) = text::brand_rail_problem(&c.label, r.width, BRAND_MINIMUM)
        {
            sink.problem(format!(
                "Component \"{}\" brand top rail leaves {}px for its label, but \"{}\" needs ~{}px at the {}px legible minimum \u{2014} widen the node or shorten the label.",
                c.id,
                num(available.max(0.0)),
                c.label,
                num(required.ceil()),
                num(BRAND_MINIMUM)
            ));
        }
        let available = text::available_width(r.width);
        for (field, value) in [("Sublabel", &c.sublabel), ("Tag", &c.tag)] {
            let Some(value) = value.as_deref().filter(|v| !v.is_empty()) else { continue };
            let needed = text::min_text_width(value, TEXT_MINIMUM);
            if needed > available {
                sink.problem(format!(
                    "{field} \"{value}\" needs ~{}px at the {}px legible minimum, but component \"{}\" provides {}px \u{2014} shorten the {} or widen size.",
                    num(needed.ceil()),
                    num(TEXT_MINIMUM),
                    c.id,
                    num(available),
                    field.to_lowercase()
                ));
            }
        }
    }

    // Component overlap, the highest-traffic hand-placement failure.
    let obstacles: Vec<Obstacle> = p.components.iter().map(|c| Obstacle { id: c.id.clone(), rect: c.rect }).collect();
    for (i, a) in obstacles.iter().enumerate() {
        for b in &obstacles[i + 1..] {
            if rects_overlap(&a.rect, &b.rect, COMPONENT_GAP) {
                sink.problem(format!(
                    "Components \"{}\" and \"{}\" are less than 8px apart \u{2014} move one or shrink its size.\n{}",
                    a.id,
                    b.id,
                    component_separation(a, b, COMPONENT_GAP)
                ));
            }
        }
    }

    // Boundaries: every wrapped id must exist; the computed box must stay in view.
    for boundary in doc.boundaries.iter().flatten() {
        for id in &boundary.wraps {
            if p.component(id).is_none() {
                sink.problem(format!("Boundary \"{}\" wraps unknown component \"{id}\".", boundary.label));
            }
        }
    }
    let view_rect = Rect::new(0.0, 0.0, view_box[0], view_box[1]);
    for b in &p.boundaries {
        if !enforces {
            continue;
        }
        let t = &b.title;
        if t.minimum_width > t.available_width {
            sink.problem(format!(
                "Boundary label \"{}\" needs ~{}px to fit at the {}px desktop-readable source minimum, but its frame provides {}px \u{2014} shorten the boundary label, increase pad, or widen the wrapped component layout.",
                b.raw.label,
                num(t.minimum_width.ceil()),
                fixed2(t.minimum_font_size),
                num(t.available_width.floor())
            ));
        }
        if !rect_contains(&b.rect, &t.rect) {
            sink.problem(format!(
                "Boundary label \"{}\" extends outside its final frame \u{2014} shorten the label or increase boundary pad.",
                b.raw.label
            ));
        }
        if !rect_contains(&view_rect, &t.rect) {
            sink.problem(format!(
                "Boundary label \"{}\" extends outside the viewBox \u{2014} move wrapped components away from the canvas edge, shorten the label, or increase the viewBox.",
                b.raw.label
            ));
        }
        for component in &p.components {
            if rects_overlap(&t.rect, &component.rect, 0.0) {
                sink.problem(format!(
                    "Boundary label \"{}\" overlaps component \"{}\" \u{2014} move the component, increase boundary title space, or shorten the label.",
                    b.raw.label, component.id
                ));
            }
        }
    }
    for (li, left) in p.boundaries.iter().enumerate() {
        for right in &p.boundaries[li + 1..] {
            if enforces && rects_overlap(&left.title.rect, &right.title.rect, 0.0) {
                sink.problem(format!(
                    "Boundary labels \"{}\" and \"{}\" overlap \u{2014} shorten a label or increase boundary title space.",
                    left.raw.label, right.raw.label
                ));
            }
            if nested_membership {
                boundary_nesting(left, right, &mut sink);
            }
        }
    }
    for b in &p.boundaries {
        let r = b.rect;
        if r.x < 0.0 || r.y < 0.0 || r.x + r.width > view_box[0] || r.y + r.height > view_box[1] {
            boundary_out_of_bounds(doc, g, b, &mut sink);
        }
    }

    // Connections: endpoints, route bounds, self-loops and length.
    let by_index: Vec<Option<&ConnectionGeom>> =
        (0..connections.len()).map(|i| g.connections.iter().find(|c| c.index == i)).collect();
    for (index, conn) in connections.iter().enumerate() {
        let name = |end: &str| conn.label.as_deref().filter(|l| !l.is_empty()).unwrap_or(end).to_owned();
        if p.component(&conn.from).is_none() {
            sink.problem(format!("Connection \"{}\" references unknown source \"{}\".", name(&conn.from), conn.from));
        }
        if p.component(&conn.to).is_none() {
            sink.problem(format!("Connection \"{}\" references unknown target \"{}\".", name(&conn.to), conn.to));
        }
        let Some(geom) = by_index[index] else { continue };
        let outside: Vec<Value> = geom
            .points
            .iter()
            .filter(|q| q[0] < 0.0 || q[1] < 0.0 || q[0] > view_box[0] || q[1] > view_box[1])
            .map(|q| pt_json(*q))
            .collect();
        let label_or_ends = || conn.id.clone().unwrap_or_else(|| format!("{}->{}", conn.from, conn.to));
        if meta_showcase && !outside.is_empty() {
            let message = format!(
                "Connection \"{}\" extends outside the viewBox \u{2014} move the measured outside route points inward or enlarge an authored viewBox for right/bottom overflow.",
                label_or_ends()
            );
            let ev = evidence([
                ("viewBox", json!([json_num(view_box[0]), json_num(view_box[1])])),
                ("outsidePoints", Value::Array(outside)),
                ("points", Value::Array(geom.points.iter().map(|q| pt_json(*q)).collect())),
            ]);
            sink.detailed(
                Diagnostic::error("layout/route-out-of-bounds", &message)
                    .with_subject(connection_subject(index, conn))
                    .with_evidence(ev)
                    .with_fixes(["move negative route coordinates inside the canvas; for right/bottom overflow, enlarge meta.viewBox or reroute inward; preserve endpoints, direction and labels, then revalidate"]),
            );
        }
        let (start, end) = (geom.points[0], geom.points[geom.points.len() - 1]);
        let distance = (end[0] - start[0]).hypot(end[1] - start[1]);
        if distance < MIN_CONNECTION && conn.from == conn.to {
            let id = conn.id.clone().or_else(|| conn.label.clone().filter(|l| !l.is_empty())).unwrap_or_else(|| conn.from.clone());
            let message = format!(
                "Self-loop \"{id}\" on component \"{}\" has its two ports only {}px apart (minimum 24px) \u{2014} remove fromSide/toSide so the renderer can choose the loop's sides, or set fromSide and toSide to different sides.",
                conn.from,
                num(js_round(distance))
            );
            let authored_or = |authored: Option<crate::model::common::Side>, chosen: crate::geom::Side| {
                authored.map_or(side_name(chosen), |s| side_name(model_side(s)))
            };
            let ev = evidence([
                ("distancePx", json_num(js_round(distance))),
                ("minimumPx", json_num(MIN_CONNECTION)),
                ("fromSide", authored_or(conn.from_side, geom.sides.from).into()),
                ("toSide", authored_or(conn.to_side, geom.sides.to).into()),
                ("points", Value::Array(geom.points.iter().map(|q| pt_json(*q)).collect())),
            ]);
            sink.detailed(
                Diagnostic::error("layout/self-loop-ports", &message)
                    .with_subject(connection_subject(index, conn))
                    .with_evidence(ev)
                    .with_fixes(["remove fromSide/toSide from the self-loop", "set fromSide and toSide to different sides of the component"]),
            );
        } else if distance < MIN_CONNECTION {
            sink.problem(format!(
                "Connection \"{}\" is too short ({}px; minimum 24px) \u{2014} place its components farther apart.",
                conn.label.as_deref().filter(|l| !l.is_empty()).map(str::to_owned).unwrap_or_else(|| format!("{}->{}", conn.from, conn.to)),
                num(js_round(distance))
            ));
        }
    }

    // The relations and labels the shared gates see.
    let routed: Vec<(&Connection, &ConnectionGeom)> = g.connections.iter().map(|geom| (&connections[geom.index], geom)).collect();
    let rels: Vec<Rel> = routed
        .iter()
        .map(|(c, geom)| {
            let (from_side, to_side) = sides_to_check(c, geom);
            Rel {
                index: geom.index,
                id: c.id.clone(),
                from: c.from.clone(),
                to: c.to.clone(),
                label: c.label.clone().unwrap_or_default(),
                points: geom.points.clone(),
                controls: controls(c),
                arrow_width: geom.stroke,
                from_side,
                to_side,
            }
        })
        .collect();
    let automatic: Vec<bool> = routed.iter().map(|(c, _)| !crate::layout::architecture_build::router_rel(c).authored_geometry()).collect();
    let unlabelled_automatic: Vec<bool> = routed.iter().zip(&automatic).map(|((c, _), auto)| *auto && c.label_at.is_none()).collect();

    endpoint_side(
        &CTX,
        &rels,
        "keep automatic routing so the renderer can use a side-aware bridge, or set truthful fromSide/toSide with perpendicular via segments",
        &mut sink,
    );
    edge_through_node(&CTX, &rels, &obstacles, "component", 2.0, "adjust fromSide/toSide, set route/via, or move the component", &mut sink);
    proper_crossing_resolved(
        &CTX,
        &rels,
        gate,
        false,
        &|l, r| automatic[l] && automatic[r],
        "adjust route/via or fromSide/toSide so the connections use separate corridors",
        &mut sink,
    );
    ambiguous_corridor_by(
        &CTX,
        &rels,
        gate,
        &|l, r| unlabelled_automatic[l] && unlabelled_automatic[r],
        "adjust route/via or fromSide/toSide so distinct connections do not visually merge",
        &mut sink,
    );
    arrowhead_collision(&CTX, &rels, &|i| unlabelled_automatic[i], gate, &mut sink);
    let frames: Vec<FrameInfo> = p
        .boundaries
        .iter()
        .enumerate()
        .map(|(i, b)| FrameInfo {
            frame: Frame::Rect { rect: b.rect, radius: b.raw.radius() },
            kind: kind_name(b.raw.kind).to_owned(),
            id: i.into(),
            label: Some(b.raw.label.clone()),
        })
        .collect();
    container_border_run(
        &CTX,
        &rels,
        &frames,
        gate,
        "adjust route/via or fromSide/toSide so the connection crosses the boundary perpendicularly instead of following its border",
        &mut sink,
    );
    route_rhythm(
        &CTX,
        &rels,
        gate,
        "move route/via points into a wider corridor or move the component so every turn has room to read",
        &mut sink,
    );
    detour(g, connections, meta_showcase, &by_index, &mut sink);

    // Connection labels: must not land on components (or, with a title contract, on titles).
    let labels: Vec<LabelRect> = routed
        .iter()
        .filter_map(|(c, geom)| {
            let l = geom.label.as_ref()?;
            Some(LabelRect {
                rel: geom.index,
                text: l.text.clone(),
                rect: l.rect,
                anchor: l.at,
                authored_at: c.label_at.is_some(),
                authored_dx: c.label_dx.filter(|v| v.is_finite()).unwrap_or(0.0),
                authored_dy: c.label_dy.filter(|v| v.is_finite()).unwrap_or(0.0),
            })
        })
        .collect();
    for (label, (conn, geom)) in labels.iter().zip(routed.iter().filter(|(_, g)| g.label.is_some())) {
        let blocked: Vec<&Obstacle> = obstacles.iter().filter(|o| rects_overlap(&label.rect, &o.rect, LABEL_GAP)).collect();
        let pinned = router_pinned(conn);
        let gap = (geom.points.len() == 2 && (geom.points[0][1] - geom.points[1][1]).abs() < 0.0001)
            .then(|| (geom.points[1][0] - geom.points[0][0]).abs());
        let required = (label.rect.width + 16.0).ceil();
        match gap {
            Some(gap) if meta_showcase && !pinned && !blocked.is_empty() && gap < required => {
                let message = format!(
                    "Label \"{}\" has only {}px between \"{}\" and \"{}\"; it needs at least {}px to stay beside its route \u{2014} increase that clear gap or place the connected nodes on another readable row, preserving the label.",
                    label.text,
                    num(js_round(gap)),
                    conn.from,
                    conn.to,
                    num(required)
                );
                let ev = evidence([
                    ("clearGapPx", json_num(gap)),
                    ("minimumGapPx", json_num(required)),
                    ("labelWidthPx", json_num(label.rect.width)),
                    ("obstacles", Value::Array(blocked.iter().map(|o| o.id.clone().into()).collect())),
                ]);
                sink.detailed(
                    Diagnostic::error("composition/label-gap", &message)
                        .with_subject(connection_subject_unindexed(conn))
                        .with_evidence(ev)
                        .with_fixes([
                            format!("increase the clear gap between the connected nodes to at least {}px", num(required)),
                            "reposition the connected nodes together while preserving the full relationship label".to_owned(),
                        ]),
                );
            }
            _ => {
                for o in &blocked {
                    sink.problem(format!(
                        "Label \"{}\" overlaps component \"{}\" \u{2014} adjust labelDx/labelDy/labelSegment or set labelAt.\n{}",
                        label.text,
                        o.id,
                        label_obstacle_fix(label, o, "component", view_box, &obstacles)
                    ));
                }
            }
        }
        if enforces {
            for b in &p.boundaries {
                if rects_overlap(&b.title.rect, &label.rect, 0.0) {
                    sink.problem(format!(
                        "Boundary label \"{}\" overlaps connection label \"{}\" \u{2014} move the boundary title rail by adjusting wrapped component positions, or move the connection label with labelAt/labelDx/labelDy/labelSegment.",
                        b.raw.label, label.text
                    ));
                }
            }
        }
    }
    let before = sink.recorded.len();
    label_route_clearance(
        &CTX,
        &rels,
        &labels,
        gate,
        "adjust labelAt, labelDx, labelDy, or labelSegment; otherwise adjust the other relationship route/via/channel",
        &mut sink,
    );
    // The renderer's label rects are objects that carry their relation (`{relation, relationIndex,
    // label, lx, ly, ...}`) and the evidence prints the whole object as `labelRect`.
    for d in &mut sink.recorded[before..] {
        let Some(label) = d.subject.index.and_then(|i| labels.iter().find(|l| l.rel as i64 == i)) else { continue };
        if let Some(Value::Object(rect)) = d.evidence.get_mut("labelRect") {
            rect.insert("relationIndex".to_owned(), label.rel.into());
            rect.insert("label".to_owned(), label.text.clone().into());
            rect.insert("lx".to_owned(), json_num(label.anchor[0]));
            rect.insert("ly".to_owned(), json_num(label.anchor[1]));
            let relation = serde_json::to_value(&connections[label.rel]).unwrap_or(Value::Null);
            rect.insert("relation".to_owned(), js_numbers(relation));
        }
    }
    label_canvas_containment(&CTX, &rels, &labels, view_box, gate, &mut sink);

    let outcome = sink.finish(|message| Subject::of("architecture").with_rule(rule_of(message)));
    (outcome, Review { rels, labels, frames, view_box, halo: automatic })
}

/// A serialised document fragment with its numbers as JS prints them (`480`, not `480.0`).
fn js_numbers(v: Value) -> Value {
    match v {
        Value::Number(n) => n.as_f64().map_or(Value::Number(n), json_num),
        Value::Array(a) => Value::Array(a.into_iter().map(js_numbers).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, js_numbers(v))).collect()),
        other => other,
    }
}

fn router_pinned(c: &Connection) -> bool {
    c.label_at.is_some() || c.label_dx.is_some() || c.label_dy.is_some() || c.label_segment.is_some()
}

fn kind_name(k: BoundaryKind) -> &'static str {
    match k {
        BoundaryKind::Region => "region",
        BoundaryKind::SecurityGroup => "security-group",
    }
}

/// The success receipt's `composition` block: the shared one, with every crossing of two automatic
/// (both-halo) routes counted as a resolved crossover.
pub fn receipt(review: &Review, reported: &str, gate: Gate) -> Value {
    receipt_resolving(&review.rels, &review.labels, &review.frames, review.view_box, reported, gate, &review.halo)
}

/// `requiresNestedBoundaryMembership`: two boundaries sharing some members must nest, and their
/// final frames must agree with the membership.
fn boundary_nesting(left: &PlacedBoundary, right: &PlacedBoundary, sink: &mut Sink) {
    let (lm, rm) = (&left.raw.wraps, &right.raw.wraps);
    fn set(v: &[String]) -> HashSet<&str> {
        v.iter().map(String::as_str).collect()
    }
    let (ls, rs) = (set(lm), set(rm));
    let quoted = |ids: Vec<&str>| ids.iter().map(|id| format!("\"{id}\"")).collect::<Vec<_>>().join(", ");
    // A JS `Set` iterates in insertion order: keep the authored order, once per id.
    let shared_ordered: Vec<&str> = {
        let mut seen = HashSet::new();
        lm.iter().map(String::as_str).filter(|id| rs.contains(id) && seen.insert(*id)).collect()
    };
    let left_nested = ls.iter().all(|id| rs.contains(id));
    let right_nested = rs.iter().all(|id| ls.contains(id));
    let only = |members: &Vec<String>, other: &HashSet<&str>| -> Vec<String> {
        let mut seen = HashSet::new();
        members.iter().filter(|id| !other.contains(id.as_str()) && seen.insert(id.as_str())).cloned().collect()
    };
    let (ll, rl) = (&left.raw.label, &right.raw.label);
    if !shared_ordered.is_empty() && !left_nested && !right_nested {
        let left_only = only(lm, &rs);
        let right_only = only(rm, &ls);
        sink.problem(format!(
            "Boundary \"{ll}\" crosses boundary \"{rl}\" because their memberships partially overlap (shared: {}; only in \"{ll}\": {}; only in \"{rl}\": {}) \u{2014} keep one boundary fully nested by removing outside members, or split the boundary.",
            quoted(shared_ordered),
            quoted(left_only.iter().map(String::as_str).collect()),
            quoted(right_only.iter().map(String::as_str).collect()),
        ));
        return;
    }
    if !rects_overlap(&left.rect, &right.rect, 0.0) {
        return;
    }
    let left_contains_right = rect_contains(&left.rect, &right.rect);
    let right_contains_left = rect_contains(&right.rect, &left.rect);
    if !left_contains_right && !right_contains_left {
        sink.problem(format!(
            "Boundary \"{ll}\" and boundary \"{rl}\" final frames partially overlap \u{2014} adjust wraps, pad, or component positions so the frames are disjoint or one fully contains the other."
        ));
        return;
    }
    if shared_ordered.is_empty() {
        sink.problem(format!(
            "Boundary \"{ll}\" and boundary \"{rl}\" final frames overlap even though their memberships are disjoint \u{2014} adjust pad or component positions so the frames are disjoint, or make wraps express the intended nesting."
        ));
        return;
    }
    if !((left_nested && right_contains_left) || (right_nested && left_contains_right)) {
        sink.problem(format!(
            "Boundary \"{ll}\" and boundary \"{rl}\" final frame containment contradicts their wraps membership \u{2014} reduce the inner boundary pad, move its components, or correct wraps so geometry and nesting agree."
        ));
    }
}

/// `layout/boundary-out-of-bounds`: the final frame leaves the canvas.
fn boundary_out_of_bounds(doc: &Architecture, g: &ArchitectureGeometry, b: &PlacedBoundary, sink: &mut Sink) {
    let view_box = g.placement.view_box;
    let r = b.rect;
    let overflow = [
        ("left", (-r.x).max(0.0)),
        ("top", (-r.y).max(0.0)),
        ("right", (r.x + r.width - view_box[0]).max(0.0)),
        ("bottom", (r.y + r.height - view_box[1]).max(0.0)),
    ];
    let get = |name: &str| overflow.iter().find(|(n, _)| *n == name).map_or(0.0, |(_, v)| *v);
    let sides = overflow
        .iter()
        .filter(|(_, px)| *px > 0.0)
        .map(|(side, px)| format!("{side} by {}px", num(px.ceil())))
        .collect::<Vec<_>>()
        .join(", ");
    let mut fixes = Vec::new();
    if get("left") > 0.0 || get("top") > 0.0 {
        fixes.push(format!(
            "move the wrapped components right by at least {}px and down by at least {}px, then revalidate connected routes and the opposite canvas sides; enlarging meta.viewBox cannot fix left/top overflow",
            num(get("left").ceil()),
            num(get("top").ceil())
        ));
    }
    if get("right") > 0.0 || get("bottom") > 0.0 {
        fixes.push(format!(
            "increase meta.viewBox to at least [{}, {}] for right/bottom overflow, or move the wrapped components inward; revalidate desktop readability",
            num(view_box[0].max(r.x + r.width).ceil()),
            num(view_box[1].max(r.y + r.height).ceil())
        ));
    }
    let message = format!(
        "Boundary \"{}\" extends outside the viewBox ({sides}) \u{2014} preserve wraps membership and repair the measured canvas side.",
        b.raw.label
    );
    let mut boundary = Map::new();
    boundary.insert("kind".to_owned(), kind_name(b.raw.kind).into());
    boundary.insert("label".to_owned(), b.raw.label.clone().into());
    boundary.insert("wraps".to_owned(), b.raw.wraps.clone().into());
    let subject = Subject::of("architecture").with_extra("boundary", Value::Object(boundary));
    let members: Vec<Value> = b
        .raw
        .wraps
        .iter()
        .filter_map(|id| g.placement.component(id))
        .map(|pc| component_box(doc, pc))
        .collect();
    let overflow_json: Map<String, Value> = overflow.iter().map(|(n, v)| ((*n).to_owned(), json_num(*v))).collect();
    let ev = evidence([
        ("bounds", rect_json(&r)),
        ("viewBox", json!([json_num(view_box[0]), json_num(view_box[1])])),
        ("overflow", Value::Object(overflow_json)),
        ("members", Value::Array(members)),
    ]);
    sink.detailed(
        Diagnostic::error("layout/boundary-out-of-bounds", &message).with_subject(subject).with_evidence(ev).with_fixes(fixes),
    );
}

fn bounds_json(b: &crate::route::grid::Bounds) -> Value {
    json!({"left": json_num(b.left), "top": json_num(b.top), "right": json_num(b.right), "bottom": json_num(b.bottom),
        "width": json_num(b.width), "height": json_num(b.height)})
}

/// `cleanRouteDetourProblems` through the grid oracle (`route::grid`): showcase (authored) only,
/// authored `via` corridors against the components, the content bounds taken from components and
/// final frames.
fn detour(g: &ArchitectureGeometry, connections: &[Connection], meta_showcase: bool, by_index: &[Option<&ConnectionGeom>], sink: &mut Sink) {
    if !meta_showcase {
        return;
    }
    let p = &g.placement;
    let relations: Vec<DetourRelation> = connections
        .iter()
        .enumerate()
        .map(|(i, c)| DetourRelation {
            id: c.id.clone(),
            from: c.from.clone(),
            to: c.to.clone(),
            has_via: c.via.as_ref().is_some_and(|v| !v.is_empty()),
            points: by_index[i].map(|g| g.points.clone()).unwrap_or_default(),
            from_side: by_index[i].map(|g| c.from_side.map(model_side).unwrap_or(g.sides.from)),
            to_side: by_index[i].map(|g| c.to_side.map(model_side).unwrap_or(g.sides.to)),
        })
        .collect();
    let obstacles: Vec<Rect> = p.components.iter().map(|c| c.rect).collect();
    let content = p.content_rects();
    let ids: HashSet<String> = p.components.iter().map(|c| c.id.clone()).collect();
    let findings = clean_route_detour_problems(&relations, &obstacles, Some(&content), &ids, true, &DetourThresholds::default());
    for f in findings {
        let message = f.message("architecture", "connections");
        let c = &connections[f.relation_index];
        let rounded = |v: f64| js_round(v * 100.0) / 100.0;
        let excursion = f.excursion.map_or(Value::Null, |e| {
            json!({"left": json_num(e.left), "top": json_num(e.top), "right": json_num(e.right), "bottom": json_num(e.bottom), "maximum": json_num(e.maximum)})
        });
        let clearance = f.empty_clearance.map_or(Value::Null, |e| json!({"maximum": json_num(e.maximum), "point": pt_json(e.point)}));
        let ev = evidence([
            ("points", Value::Array(f.points.iter().map(|q| pt_json(*q)).collect())),
            ("actualLengthPx", json_num(rounded(f.actual_length))),
            ("shortestLegalPoints", Value::Array(f.shortest_points.iter().map(|q| pt_json(*q)).collect())),
            ("shortestLegalLengthPx", json_num(rounded(f.shortest_length))),
            ("detourRatio", json_num(rounded(f.detour_ratio))),
            ("excessLengthPx", json_num(rounded(f.excess_length))),
            ("routeBounds", bounds_json(&f.route_bounds)),
            ("contentBounds", f.content_bounds.as_ref().map_or(Value::Null, bounds_json)),
            ("emptyExcursionPx", excursion),
            ("emptyControlPointClearancePx", clearance),
            ("obstacleCount", f.obstacle_count.into()),
            (
                "thresholds",
                json!({"minimumDetourRatio": json_num(f.thresholds.minimum_detour_ratio),
                    "minimumExcessLengthPx": json_num(f.thresholds.minimum_excess_length_px),
                    "minimumEmptyExcursionPx": json_num(f.thresholds.minimum_empty_excursion_px)}),
            ),
        ]);
        sink.error(
            Diagnostic::error("composition/excessive-route-detour", &message)
                .with_subject(connection_subject(f.relation_index, c))
                .with_evidence(ev)
                .with_fixes(["remove the distant via points and retry automatic routing, or keep the endpoint sides and move the via corridor near the connected nodes while preserving labels and direction"]),
        );
    }
}
