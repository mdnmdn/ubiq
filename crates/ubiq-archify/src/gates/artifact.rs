//! The post-render artifact checker (`scripts/check-render-output.mjs`, stage `check`, spec V10),
//! evaluated on the in-memory [`Scene`] and the gates' routed relations instead of on emitted HTML
//! (D5; D26 superseded). Archify's checker regex-parses its own SVG: every fact it reads is re-derived
//! here from the scene, with the same constants, the same flattening and the same message text.
//!
//! What the checker sees that the layout gates do not: a route as the browser draws it (each rounded
//! corner is a `Q` curve the checker flattens into 8 chords, so two routes can cross on a corner the
//! raw points do not cross on), the straight pieces of that path (border runs, orthogonality), the
//! legend's rects and text boxes, and the text sizes against the 930 px desktop budget.
//!
//! The nine checks (`single_svg`, `finite_svg`, `orthogonal_arrows`, `label_route_clearance`,
//! `relationship_crossings`, `relationship_corridors`, `container_border_runs`, `route_rhythm`,
//! `legend_clearance`) are the receipt's `checks`; the `composition` block is the checker's own,
//! with `desktopReadability`, `viewportHeight` and the review evidence. Only the browser stage (V11)
//! is not here. The input is what each type's compile already holds ([`Input`]); the diagnostics
//! are what `checkerDiagnostics` makes of the checker's receipt.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::composition::{self as comp, FrameInfo};
use super::{Gate, LabelRect, Rel, num, round1};
use crate::diag::{Diagnostic, Subject, js_round, json_num};
use crate::geom::{self, Pt, Rect, Seg, cross_product};
use crate::route::workflow::forward_collinear_segments;
use crate::scene::{Anchor, Detail, GroupKind, Layer, PolylineShape, RectShape, Scene, Shape, Stroke};
use crate::tokens::{Kind, Token};

/// `DESKTOP_READABILITY_VIEWPORT`, `DESKTOP_READER_DIAGRAM_WIDTH` (960 - 30), `MIN_PROJECTED_NODE_TEXT_PX`.
const VIEWPORT: (f64, f64) = (1440.0, 900.0);
const LEGACY_DIAGRAM_WIDTH: f64 = 930.0;
const MIN_PROJECTED_TEXT_PX: f64 = 6.0;
/// `DECLARED_WIDE_READER_*`: the Reader contract the HTML template always declares.
const READER_CONTRACT: &str = "declared-wide-v1";
const WIDE_RATIO: f64 = 1.55;
const WIDE_MAX_WIDTH: f64 = 1920.0;
const BODY_HORIZONTAL_PX: f64 = 64.0;
const DIAGRAM_HORIZONTAL_PX: f64 = 30.0;
const MIN_READER_WIDTH: f64 = 960.0;
/// `DESKTOP_FIXED_VERTICAL_CHROME_PX`: body 12 + header 39 + diagram 75.
const FIXED_CHROME_PX: f64 = 126.0;

/// What a type's compile knows of one relation that the scene does not say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EdgeFacts {
    /// `data-composition-route="straight"`: an authored direct diagonal (a `route: "straight"` with no
    /// `via` and two points, `authoredStraightRouteAttrs`). Architecture, dataflow, lifecycle.
    pub authored_straight: bool,
    /// `data-composition-independent`: an automatic architecture route with no authored `labelAt`.
    pub independent: bool,
    /// `data-edge-role` (workflow v2): part of what makes two trunks "the same".
    pub role: String,
}

/// The SVG root's facts (`data-*` of `<svg>`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Root {
    /// `data-layout-contract="readable-v2"`.
    pub workflow_v2: bool,
    /// `data-reader-fit`: `intrinsic-height`, `authored-height` or absent.
    pub reader_fit: Option<&'static str>,
    /// `data-diagram-type="architecture"`: an architecture with an authored viewBox.
    pub arch_authored: bool,
    /// `data-reader-min-text`: the architecture automatic canvas, 7.5.
    pub min_text: Option<f64>,
    /// `data-reader-primary-text="14"`: the architecture automatic canvas.
    pub primary_text_14: bool,
    /// `data-sequence-column-fit`.
    pub column_fit: Option<&'static str>,
}

/// Everything the checker reads. `rels`, `labels` and `frames` are the type's gate review; `facts`
/// is parallel to `rels`.
pub struct Input<'a> {
    pub doc_type: &'a str,
    pub scene: &'a Scene,
    pub rels: &'a [Rel],
    pub facts: &'a [EdgeFacts],
    pub labels: &'a [LabelRect],
    pub frames: &'a [FrameInfo],
    pub gate: Gate,
    pub root: Root,
}

/// The checker's receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub ok: bool,
    /// `checks[]`: `{name, ok, details}`, in the checker's order.
    pub checks: Vec<Value>,
    /// The checker's `composition` block.
    pub composition: Value,
    /// `checkerDiagnostics`: empty when `ok`.
    pub diagnostics: Vec<Diagnostic>,
}

/// One `<path class="a-*" marker-end>` of the artifact, as `collectArrows` reads it.
struct Arrow {
    /// 1-based position in the artifact (`artifactIndex`).
    index: usize,
    /// `data-edge-key`: the relation's index in its collection (`usize::MAX` for a decoration).
    key: usize,
    /// A relationship (`data-edge-from`/`-to`), not a decoration such as the v1 lifecycle rail: those
    /// are arrows to the checker too, but name no relationship.
    semantic: bool,
    from: String,
    to: String,
    id: Option<String>,
    label: String,
    raw: String,
    width: f64,
    variant: &'static str,
    role: String,
    authored_straight: bool,
    crossover_halo: bool,
    independent_ports: bool,
    /// The renderer's independence mark (architecture `data-composition-independent`, workflow v2 auto route).
    independent: bool,
    /// `pathSegments(d)`: the rounded path with each `Q` flattened into 8 chords.
    segments: Vec<Seg>,
    /// `straightPathSegments(d)`: the visible straight pieces.
    border: Vec<Seg>,
    /// `routePoints`.
    points: Vec<Pt>,
}

impl Arrow {
    /// `relationshipName`.
    fn name(&self) -> String {
        // A decoration has no `from`/`to`: the checker prints JavaScript's `undefined`.
        let (from, to) = if self.semantic { (self.from.as_str(), self.to.as_str()) } else { ("undefined", "undefined") };
        match &self.id {
            Some(id) => format!("relationship id \"{id}\" (\"{from}\" -> \"{to}\")"),
            None => format!("relationship \"{from}\" -> \"{to}\""),
        }
    }

    /// `relationshipRecord`.
    fn record(&self) -> Value {
        let mut m = Map::new();
        if let Some(id) = &self.id {
            m.insert("id".into(), id.clone().into());
        }
        if self.semantic {
            m.insert("from".into(), self.from.clone().into());
            m.insert("to".into(), self.to.clone().into());
        }
        m.insert("label".into(), self.label.clone().into());
        // `Number(undefined)` is not an integer: the artifact position stands in.
        m.insert("collectionIndex".into(), if self.semantic { self.key } else { self.index - 1 }.into());
        m.insert("artifactIndex".into(), self.index.into());
        Value::Object(m)
    }

    fn as_rel(&self) -> Rel {
        Rel {
            index: self.key,
            id: self.id.clone(),
            from: self.from.clone(),
            to: self.to.clone(),
            label: self.label.clone(),
            points: self.points.clone(),
            controls: Vec::new(),
            arrow_width: self.width,
            from_side: None,
            to_side: None,
        }
    }
}

/// `esc` of the renderers: the attribute and text escaping the artifact carries.
pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// A number as JavaScript's `String(x)` prints it, including the non-finite words.
fn js_num(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else {
        num(x)
    }
}

fn pt(p: Pt) -> Value {
    json!([json_num(p[0]), json_num(p[1])])
}

fn pt1(p: Pt) -> Value {
    json!([json_num(round1(p[0])), json_num(round1(p[1]))])
}

/// `formatPoint`.
fn fmt_pt(p: Pt) -> String {
    format!("{}, {}", js_num(round1(p[0])), js_num(round1(p[1])))
}

fn rounded_rect(r: &Rect) -> Value {
    json!({"x": json_num(round1(r.x)), "y": json_num(round1(r.y)),
        "width": json_num(round1(r.width)), "height": json_num(round1(r.height))})
}

fn opt(v: Option<f64>) -> Value {
    v.map_or(Value::Null, json_num)
}

// ---- reading the scene ---------------------------------------------------------------------------

/// The polyline of each edge group, by `data-edge-key`.
fn polylines(scene: &Scene) -> HashMap<u32, &PolylineShape> {
    let mut map = HashMap::new();
    for item in &scene.items {
        if item.layer != Layer::Edges {
            continue;
        }
        let (Shape::Polyline(p), Some(g)) = (&item.shape, item.group) else { continue };
        if let Some(edge) = scene.group(g).filter(|g| g.kind == GroupKind::Edge).and_then(|g| g.edge.as_ref()) {
            map.entry(edge.key).or_insert(p);
        }
    }
    map
}

/// `stroke` of an edge as the CSS class says it (`a-default`, `a-emphasis`, ...).
fn variant_of(stroke: &Stroke) -> &'static str {
    match stroke.token {
        Token::ArrowEmphasis => "emphasis",
        Token::KindStroke(Kind::Security) => "security",
        Token::KindStroke(Kind::Database) => "dashed",
        _ => "default",
    }
}

fn marker_of(variant: &str) -> &'static str {
    match variant {
        "emphasis" => "arrowhead-emphasis",
        "security" => "arrowhead-security",
        "dashed" => "arrowhead-dashed",
        _ => "arrowhead",
    }
}

/// `pointsFromPath` and `straightPathSegments` of the rounded path of a polyline.
fn path_pieces(p: &PolylineShape) -> (Vec<Seg>, Vec<Seg>) {
    let cmds = geom::rounded_cmds(&p.points, p.radius);
    let flat: Vec<Pt> = geom::flatten_cmds(&cmds, geom::CORNER_STEPS).into_iter().filter(|q| q[0].is_finite() && q[1].is_finite()).collect();
    let segments = flat.windows(2).map(|w| Seg::new(w[0], w[1])).collect();
    let mut border = Vec::new();
    let mut current = [0.0, 0.0];
    for cmd in &cmds {
        match *cmd {
            geom::PathCmd::M(q) => current = q,
            geom::PathCmd::L(q) => {
                border.push(Seg::new(current, q));
                current = q;
            }
            geom::PathCmd::Q(control, end) => {
                // A non-collinear Q is never flattened into chords here: only a fully collinear one is a
                // straight primitive.
                if cross_product(current, control, end).abs() <= 1e-9 {
                    border.push(Seg::new(current, end));
                }
                current = end;
            }
        }
    }
    border.retain(|s| s.start.iter().chain(&s.end).all(|v| v.is_finite()));
    (segments, border)
}

/// The `<path>` tag the renderer wrote, for the one message that quotes it (`orthogonal_arrows`).
fn raw_tag(doc_type: &str, v2: bool, a: &Arrow, points: &[Pt], d: &str) -> String {
    let route_points: Vec<String> = points
        .iter()
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .map(|p| format!("{},{}", js_num(p[0]), js_num(p[1])))
        .collect();
    let route_points = route_points.join(";");
    let class = format!("a-{}", if a.variant == "return" { "default" } else { a.variant });
    let marker = marker_of(a.variant);
    let width = js_num(a.width);
    let id = a.id.as_ref().map(|id| format!(" data-edge-id=\"{id}\"")).unwrap_or_default();
    if doc_type == "sequence" {
        let cid = a.id.as_ref().map(|id| format!(" data-composition-edge-id=\"{id}\"")).unwrap_or_default();
        let dash = if a.variant == "return" { " stroke-dasharray=\"3,5\"" } else { "" };
        return format!(
            "<path data-composition-edge-from=\"{}\" data-composition-edge-to=\"{}\"{cid} data-composition-points=\"{route_points}\" d=\"{d}\" class=\"{class}\" stroke-width=\"{width}\"{dash} marker-end=\"url(#{marker})\"/>",
            a.from, a.to
        );
    }
    let named = if a.label.is_empty() { String::new() } else { format!(" data-edge-label=\"{}\"", a.label) };
    let focus = format!("data-edge-from=\"{}\" data-edge-to=\"{}\"{named} data-edge-key=\"{}\"{id}", a.from, a.to, a.key);
    let crossover = if a.crossover_halo && matches!(doc_type, "architecture" | "lifecycle") {
        let independent = if a.independent_ports && doc_type == "architecture" { " data-composition-independent=\"true\"" } else { "" };
        format!(" data-composition-crossover=\"halo\"{independent}")
    } else {
        String::new()
    };
    let straight = if a.authored_straight { " data-composition-route=\"straight\"" } else { "" };
    if doc_type == "workflow" {
        let routing = if a.independent { " data-composition-routing=\"workflow-v2-auto\"" } else { "" };
        let role = if v2 { format!(" data-edge-role=\"{}\"", a.role) } else { String::new() };
        return format!(
            "<path {focus}{routing}{role} data-composition-points=\"{route_points}\" d=\"{d}\" class=\"{class}\" stroke-width=\"{width}\" marker-end=\"url(#{marker})\"/>"
        );
    }
    format!(
        "<path {focus} data-composition-points=\"{route_points}\"{crossover}{straight} d=\"{d}\" class=\"{class}\" stroke-width=\"{width}\" marker-end=\"url(#{marker})\"/>"
    )
}

/// `collectArrows`: one per relation, in artifact order (the collection order).
fn arrows(input: &Input<'_>) -> Vec<Arrow> {
    let polys = polylines(input.scene);
    // Artifact order: by layer, then the collection order of the relationships; a decoration (an arrow
    // that is no relationship: the v1 lifecycle rail) sits where its layer puts it.
    enum Slot<'a> {
        Rel(usize),
        Decor(&'a PolylineShape),
    }
    let mut slots: Vec<(Layer, usize, Slot<'_>)> = (0..input.rels.len()).map(|i| (Layer::Edges, input.rels[i].index, Slot::Rel(i))).collect();
    for (seq, item) in input.scene.items.iter().enumerate() {
        let Shape::Polyline(p) = &item.shape else { continue };
        let in_edge = item.group.and_then(|g| input.scene.group(g)).is_some_and(|g| g.kind == GroupKind::Edge);
        let in_legend = item.layer == Layer::Legend || item.group.and_then(|g| input.scene.group(g)).is_some_and(|g| g.kind == GroupKind::Legend);
        if p.marker.is_some() && !in_edge && !in_legend {
            slots.push((item.layer, seq, Slot::Decor(p)));
        }
    }
    slots.sort_by_key(|(layer, order, _)| (*layer, *order));
    let mut out = Vec::with_capacity(slots.len());
    for (k, (_, _, slot)) in slots.into_iter().enumerate() {
        let fallback;
        let (rel, facts, poly) = match slot {
            Slot::Rel(i) => {
                let rel = &input.rels[i];
                let poly = match polys.get(&(rel.index as u32)) {
                    Some(p) => *p,
                    None => {
                        fallback = PolylineShape {
                            points: rel.points.clone(),
                            radius: 0.0,
                            stroke: Stroke::solid(Token::Arrow, rel.arrow_width),
                            marker: Some(crate::scene::Marker { fill: Token::Arrow }),
                            halo: false,
                        };
                        &fallback
                    }
                };
                (Some(rel), input.facts.get(i).cloned().unwrap_or_default(), poly)
            }
            Slot::Decor(p) => (None, EdgeFacts::default(), p),
        };
        let (segments, border) = path_pieces(poly);
        let mut variant = variant_of(&poly.stroke);
        if variant == "default" && poly.stroke.dash == [3.0, 5.0] {
            variant = "return";
        }
        // `data-composition-points` and `d` are written from the same routed points.
        let finite: Vec<Pt> = poly.points.iter().copied().filter(|p| p[0].is_finite() && p[1].is_finite()).collect();
        let derived = || -> Vec<Pt> {
            if border.is_empty() {
                Vec::new()
            } else {
                std::iter::once(border[0].start).chain(border.iter().map(|s| s.end)).collect()
            }
        };
        let points = if rel.is_none() || input.root.workflow_v2 || finite.len() < 2 { derived() } else { finite.clone() };
        let id = rel.and_then(|r| r.id.as_deref()).filter(|id| !id.trim().is_empty()).map(esc);
        let crossover_halo = poly.halo;
        let mut arrow = Arrow {
            index: k + 1,
            key: rel.map_or(usize::MAX, |r| r.index),
            semantic: rel.is_some(),
            from: rel.map(|r| esc(&r.from)).unwrap_or_default(),
            to: rel.map(|r| esc(&r.to)).unwrap_or_default(),
            id,
            label: rel.map(|r| esc(&r.label)).unwrap_or_default(),
            raw: String::new(),
            width: poly.stroke.width,
            variant,
            role: facts.role.clone(),
            authored_straight: false,
            crossover_halo,
            independent_ports: crossover_halo && facts.independent,
            independent: facts.independent,
            segments,
            border,
            points,
        };
        // The straight waiver trusts a semantic edge with one visible direct segment.
        arrow.authored_straight = facts.authored_straight && arrow.segments.len() == 1 && arrow.border.len() == 1;
        let d = geom::rounded_path(&poly.points, poly.radius);
        arrow.raw = if arrow.semantic {
            raw_tag(input.doc_type, input.root.workflow_v2, &arrow, &finite, &d)
        } else {
            format!(
                "<path d=\"{d}\" class=\"a-{}\" stroke-width=\"{}\" marker-end=\"url(#{})\"/>",
                arrow.variant,
                js_num(arrow.width),
                marker_of(arrow.variant)
            )
        };
        out.push(arrow);
    }
    out
}

// ---- the checks ----------------------------------------------------------------------------------

/// `NON_FINITE_TOKEN` over the geometry attributes the scene would emit.
fn finite_details(scene: &Scene) -> Vec<String> {
    let mut details = Vec::new();
    let mut attr = |element: &str, name: &str, value: f64| {
        if !value.is_finite() {
            details.push(format!("{element} {name}=\"{}\"", js_num(value)));
        }
    };
    if !scene.view_box[0].is_finite() || !scene.view_box[1].is_finite() {
        attr("svg", "viewBox", if scene.view_box[0].is_finite() { scene.view_box[1] } else { scene.view_box[0] });
    }
    for item in &scene.items {
        match &item.shape {
            Shape::Rect(r) => {
                for (name, v) in [("x", r.rect.x), ("y", r.rect.y), ("width", r.rect.width), ("height", r.rect.height)] {
                    attr("rect", name, v);
                }
            }
            Shape::Text(t) => {
                attr("text", "x", t.at[0]);
                attr("text", "y", t.at[1]);
                attr("text", "font-size", t.size);
            }
            Shape::Polyline(p) => {
                if let Some(bad) = p.points.iter().flatten().find(|v| !v.is_finite()) {
                    attr("path", "d", *bad);
                }
            }
            Shape::Path(_) => {}
        }
    }
    details
}

/// `diagonalStraightSegments` over every arrow.
fn diagonal_details(arrows: &[Arrow]) -> Vec<String> {
    let mut details = Vec::new();
    for a in arrows {
        if a.authored_straight {
            continue;
        }
        for (i, s) in a.border.iter().enumerate() {
            if (s.start[0] - s.end[0]).abs() > 0.01 && (s.start[1] - s.end[1]).abs() > 0.01 {
                details.push(format!(
                    "path {} segment {}: expected an orthogonal segment or an explicitly authored direct straight route; {}",
                    a.index,
                    i + 1,
                    a.raw
                ));
            }
        }
    }
    details
}

/// The checker's own `properSegmentIntersection` (epsilon 1e-9, not geometry.mjs's 1e-4).
fn proper_intersection(a: Pt, b: Pt, c: Pt, d: Pt) -> Option<Pt> {
    let (ab_c, ab_d, cd_a, cd_b) = (cross_product(a, b, c), cross_product(a, b, d), cross_product(c, d, a), cross_product(c, d, b));
    let eps = 1e-9;
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
    Some([(ab * (c[0] - d[0]) - (a[0] - b[0]) * cd) / denominator, (ab * (c[1] - d[1]) - (a[1] - b[1]) * cd) / denominator])
}

struct Crossing {
    left: usize,
    right: usize,
    point: Pt,
}

/// `collectRelationshipCrossings` (indices into `arrows`).
fn crossings(arrows: &[Arrow], workflow_v2: bool) -> Vec<Crossing> {
    let eligible: Vec<usize> = (0..arrows.len()).filter(|&i| arrows[i].semantic && !arrows[i].segments.is_empty()).collect();
    let segments: HashMap<usize, Vec<Seg>> = eligible
        .iter()
        .map(|&i| {
            let segs = if workflow_v2 {
                forward_collinear_segments(&arrows[i].points).into_iter().map(|s| Seg::new(s.start, s.end)).collect()
            } else {
                arrows[i].segments.clone()
            };
            (i, segs)
        })
        .collect();
    let mut hits = Vec::new();
    for (li, &l) in eligible.iter().enumerate() {
        for &r in &eligible[li + 1..] {
            let (left, right) = (&arrows[l], &arrows[r]);
            let shares = [&left.from, &left.to].iter().any(|id| **id == right.from || **id == right.to);
            if shares && !workflow_v2 && !(left.independent_ports && right.independent_ports) {
                continue;
            }
            let point = segments[&l]
                .iter()
                .find_map(|a| segments[&r].iter().find_map(|b| proper_intersection(a.start, a.end, b.start, b.end)));
            if let Some(point) = point {
                hits.push(Crossing { left: l, right: r, point });
            }
        }
    }
    hits
}

/// `shortWorkflowTrunk` on arrows (workflow v2: `allowShortWorkflowTrunks`).
fn short_trunk(a: &Arrow, b: &Arrow, lp: &[Pt], rp: &[Pt], ls: usize, rs: usize, length: f64) -> bool {
    if length > 24.0 + 0.0001 {
        return false;
    }
    if a.variant != b.variant || a.width != b.width || a.role != b.role {
        return false;
    }
    let same = |p: Pt, q: Pt| (p[0] - q[0]).abs() < 0.0001 && (p[1] - q[1]).abs() < 0.0001;
    let source = a.from == b.from && ls == 0 && rs == 0 && same(lp[0], rp[0]);
    let target = a.to == b.to && ls == lp.len() - 2 && rs == rp.len() - 2 && same(lp[lp.len() - 1], rp[rp.len() - 1]);
    if !source && !target {
        return false;
    }
    let (p, q, r, s) = (lp[ls], lp[ls + 1], rp[rs], rp[rs + 1]);
    (q[0] - p[0]) * (s[0] - r[0]) + (q[1] - p[1]) * (s[1] - r[1]) > 0.0
}

/// `collectLegendBoxes`: every rect and text of the legend, as `[x1, y1, x2, y2]` and its label.
fn legend_boxes(scene: &Scene) -> Vec<([f64; 4], String)> {
    let mut boxes = Vec::new();
    for item in scene.items.iter().filter(|i| i.layer == Layer::Legend) {
        match &item.shape {
            Shape::Rect(r) if r.rect.rect().is_finite() => {
                let b = r.rect;
                boxes.push(([b.x, b.y, b.x + b.width, b.y + b.height], format!("rect@{},{}", js_num(b.x), js_num(b.y))));
            }
            Shape::Text(t) => {
                if let Some(b) = text_box(t.at, t.size, t.anchor, &esc(t.text.trim())) {
                    boxes.push(b);
                }
            }
            _ => {}
        }
    }
    boxes
}

/// `estimatedTextWidth`.
fn text_width(text: &str, size: f64) -> f64 {
    let units: f64 = text.chars().map(|c| if c as u32 > 255 { 1.8 } else { 0.62 }).sum();
    size.max(units * size)
}

/// `textBox`.
fn text_box(at: Pt, size: f64, anchor: Anchor, text: &str) -> Option<([f64; 4], String)> {
    if !at[0].is_finite() || !at[1].is_finite() || !size.is_finite() {
        return None;
    }
    let width = text_width(text, size);
    let x1 = match anchor {
        Anchor::Middle => at[0] - width / 2.0,
        Anchor::Start => at[0],
    };
    let label = if text.is_empty() { format!("text@{},{}", js_num(at[0]), js_num(at[1])) } else { text.to_owned() };
    Some(([x1, at[1] - size, x1 + width, at[1] + size * 0.25], label))
}

fn orientation(a: Pt, b: Pt, c: Pt) -> u8 {
    let value = (b[1] - a[1]) * (c[0] - b[0]) - (b[0] - a[0]) * (c[1] - b[1]);
    if value.abs() < 1e-9 {
        0
    } else if value > 0.0 {
        1
    } else {
        2
    }
}

fn on_segment(a: Pt, b: Pt, c: Pt) -> bool {
    b[0] <= a[0].max(c[0]) + 1e-9 && b[0] + 1e-9 >= a[0].min(c[0]) && b[1] <= a[1].max(c[1]) + 1e-9 && b[1] + 1e-9 >= a[1].min(c[1])
}

fn segments_cross(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let (o1, o2, o3, o4) = (orientation(a, b, c), orientation(a, b, d), orientation(c, d, a), orientation(c, d, b));
    (o1 != o2 && o3 != o4) || (o1 == 0 && on_segment(a, c, b)) || (o2 == 0 && on_segment(a, d, b)) || (o3 == 0 && on_segment(c, a, d)) || (o4 == 0 && on_segment(c, b, d))
}

/// `segmentIntersectsBox(segment, padBox(box, 2))`.
fn hits_box(s: &Seg, b: &[f64; 4]) -> bool {
    let [x1, y1, x2, y2] = [b[0] - 2.0, b[1] - 2.0, b[2] + 2.0, b[3] + 2.0];
    let inside = |p: Pt| p[0] >= x1 && p[0] <= x2 && p[1] >= y1 && p[1] <= y2;
    if inside(s.start) || inside(s.end) {
        return true;
    }
    [([x1, y1], [x2, y1]), ([x2, y1], [x2, y2]), ([x2, y2], [x1, y2]), ([x1, y2], [x1, y1])]
        .iter()
        .any(|(a, b)| segments_cross(s.start, s.end, *a, *b))
}

// ---- desktop readability -------------------------------------------------------------------------

struct Entry {
    node_id: Option<String>,
    owner: Value,
    text: String,
    detail: &'static str,
    source: f64,
}

/// The semantic texts `collectDesktopReadability` measures: node labels, node sublabels, edge labels
/// and (architecture) boundary titles. Fine texts, legend and loose copy are not.
fn semantic_texts(input: &Input<'_>) -> (Vec<Entry>, bool) {
    let scene = input.scene;
    let mut entries = Vec::new();
    let mut invalid = false;
    for item in &scene.items {
        let (Shape::Text(t), Some(g)) = (&item.shape, item.group) else { continue };
        let Some(group) = scene.group(g) else { continue };
        if t.detail == Detail::Fine || item.layer == Layer::Legend {
            continue;
        }
        let primary = group.kind == GroupKind::Node && t.detail == Detail::Anchor;
        let boundary = group.kind == GroupKind::Frame && t.detail == Detail::Anchor && input.doc_type == "architecture";
        let context = t.detail == Detail::Context;
        let node = |id: &Option<String>| json!({"kind": "node", "id": id});
        let owner = if primary {
            node(&group.node_id)
        } else if boundary {
            json!({"kind": "boundary", "id": null})
        } else if let (true, GroupKind::Label, Some(e)) = (context, group.kind, group.edge.as_ref()) {
            json!({"kind": "edge", "id": e.id.as_deref().filter(|s| !s.is_empty()).map(esc), "from": esc(&e.from), "to": esc(&e.to)})
        } else if context && group.kind == GroupKind::Node {
            node(&group.node_id)
        } else {
            continue;
        };
        if !t.size.is_finite() {
            invalid = true;
            continue;
        }
        if t.size <= 0.0 {
            invalid = true;
        }
        let kind_node = owner["kind"] == "node";
        entries.push(Entry {
            node_id: kind_node.then(|| group.node_id.clone()).flatten().filter(|s| !s.is_empty()).map(|s| esc(&s)),
            detail: if primary {
                "primary"
            } else if boundary {
                "boundary"
            } else if owner["kind"] == "edge" {
                "edge"
            } else {
                "context"
            },
            owner,
            text: esc(t.text.trim()),
            source: t.size,
        });
    }
    (entries, invalid)
}

/// `declaredWideReadabilityBudget`.
struct Declared {
    desired: f64,
    cap: f64,
    actual: f64,
    guaranteed: f64,
    limit: &'static str,
}

fn declared_budget(vb: [f64; 2], min_source: f64, requested: f64) -> Option<Declared> {
    let values = [vb[0], vb[1], min_source, requested];
    if !values.iter().all(|v| v.is_finite()) || vb[0] <= 0.0 || vb[1] <= 0.0 || min_source <= 0.0 || requested <= 0.0 || vb[0] / vb[1] < WIDE_RATIO {
        return None;
    }
    let target = MIN_PROJECTED_TEXT_PX.max(requested);
    let scale = (target / min_source).min(1.0);
    let desired = MIN_READER_WIDTH.max(vb[0] * scale + DIAGRAM_HORIZONTAL_PX);
    let cap = (VIEWPORT.0 - BODY_HORIZONTAL_PX).max(0.0);
    let actual = desired.min(WIDE_MAX_WIDTH.min(cap));
    let guaranteed = (actual - DIAGRAM_HORIZONTAL_PX).max(0.0);
    let limit = if actual < desired {
        if cap <= WIDE_MAX_WIDTH { "viewport-cap" } else { "reader-cap" }
    } else {
        "source-size"
    };
    Some(Declared { desired, cap, actual, guaranteed, limit })
}

/// `projectedNodeTextPx`.
fn projected(source: f64, vb_width: f64, width: f64) -> f64 {
    if !source.is_finite() || !vb_width.is_finite() || !width.is_finite() || vb_width <= 0.0 || width <= 0.0 {
        return f64::NAN;
    }
    source * (width / vb_width).min(1.0)
}

struct Readability {
    evidence: Value,
    issue: Option<Value>,
    min_projected: Option<f64>,
}

fn readability(input: &Input<'_>) -> Readability {
    let vb = input.scene.view_box;
    let (entries, invalid) = semantic_texts(input);
    let requested = input.root.min_text.unwrap_or(f64::NAN);
    let min_source = entries.iter().map(|e| e.source).fold(f64::NAN, f64::min);
    let eligible = input.root.reader_fit == Some("intrinsic-height") && requested.is_finite() && requested > 0.0 && !invalid && !entries.is_empty();
    let declared = if eligible { declared_budget(vb, min_source, requested) } else { None };
    let available = declared.as_ref().map_or(LEGACY_DIAGRAM_WIDTH, |d| d.guaranteed);
    let basis = if declared.is_some() { "recognized-declared-wide" } else { "legacy-930" };
    let scale = if vb[0].is_finite() && vb[0] > 0.0 { (available / vb[0]).min(1.0) } else { f64::NAN };
    let mut worst: Option<(usize, f64)> = None;
    for (i, e) in entries.iter().enumerate() {
        let p = projected(e.source, vb[0], available);
        if worst.is_none_or(|(_, w)| p < w) {
            worst = Some((i, p));
        }
    }
    let min_projected = worst.map(|(_, p)| p).filter(|p| p.is_finite());
    let hard_floor_met = min_projected.map(|p| p >= MIN_PROJECTED_TEXT_PX);
    let requested_met = worst.and_then(|_| requested.is_finite().then(|| min_projected.is_some_and(|p| p >= requested)));
    let requested_target = requested.is_finite().then_some(requested);
    let mut evidence = Map::new();
    evidence.insert("budgetBasis".into(), basis.into());
    evidence.insert("readerContract".into(), READER_CONTRACT.into());
    evidence.insert("availableDiagramWidth".into(), json_num(available));
    evidence.insert("actualBudgetPx".into(), json_num(available));
    match &declared {
        Some(d) => {
            evidence.insert("actualReaderWidth".into(), json_num(d.actual));
            evidence.insert("desiredReaderWidth".into(), json_num(d.desired));
            evidence.insert("viewportCap".into(), json_num(d.cap));
            evidence.insert("limit".into(), d.limit.into());
        }
        None => {
            evidence.insert("limit".into(), "legacy".into());
        }
    }
    evidence.insert("requestedTargetPx".into(), opt(requested_target));
    evidence.insert("requestedTargetMet".into(), requested_met.map_or(Value::Null, Value::Bool));
    evidence.insert("hardFloorPx".into(), json_num(MIN_PROJECTED_TEXT_PX));
    evidence.insert("hardFloorMet".into(), hard_floor_met.map_or(Value::Null, Value::Bool));
    evidence.insert("minimumOwner".into(), worst.map_or(Value::Null, |(i, _)| entries[i].owner.clone()));
    evidence.insert("minimumSourceTextPx".into(), worst.map_or(Value::Null, |(i, _)| json_num(entries[i].source)));
    evidence.insert("minimumProjectedTextPx".into(), opt(min_projected));
    evidence.insert("semanticTextCount".into(), entries.len().into());
    let issue = worst.filter(|_| hard_floor_met == Some(false)).map(|(i, p)| {
        let e = &entries[i];
        let mut m = Map::new();
        if let Some(id) = &e.node_id {
            m.insert("nodeId".into(), id.clone().into());
        }
        m.insert("owner".into(), e.owner.clone());
        m.insert("viewportWidth".into(), json_num(VIEWPORT.0));
        m.insert("viewportHeight".into(), json_num(VIEWPORT.1));
        m.insert("availableDiagramWidth".into(), json_num(available));
        m.insert("budgetBasis".into(), basis.into());
        m.insert("readerContract".into(), READER_CONTRACT.into());
        m.insert("requestedTargetPx".into(), opt(requested_target));
        m.insert("requestedTargetMet".into(), requested_met.map_or(Value::Null, Value::Bool));
        m.insert("budgetLimit".into(), evidence["limit"].clone());
        m.insert("viewBoxWidth".into(), json_num(vb[0]));
        m.insert("scale".into(), json_num(scale));
        m.insert("text".into(), e.text.clone().into());
        m.insert("detail".into(), e.detail.into());
        m.insert("sourceFontPx".into(), json_num(e.source));
        m.insert("projectedFontPx".into(), json_num(p));
        m.insert("minimumProjectedFontPx".into(), json_num(MIN_PROJECTED_TEXT_PX));
        Value::Object(m)
    });
    Readability { evidence: Value::Object(evidence), issue, min_projected }
}

/// `describeFixedWidthOverflow` and `predictedFixedWidthOverflow`: `composition/viewport-height`.
fn viewport_height(input: &Input<'_>) -> Option<(Value, String)> {
    let [w, h] = input.scene.view_box;
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let ratio = w / h;
    let fit = input.root.reader_fit;
    if fit == Some("intrinsic-height") || (fit == Some("authored-height") && input.root.arch_authored) || ratio >= WIDE_RATIO {
        return None;
    }
    let svg_width = VIEWPORT.0 - BODY_HORIZONTAL_PX - DIAGRAM_HORIZONTAL_PX;
    let svg_height = js_round(svg_width * h / w);
    let page = svg_height + FIXED_CHROME_PX;
    if page <= VIEWPORT.1 {
        return None;
    }
    let rounded_ratio = js_round(ratio * 100.0) / 100.0;
    let max_height = (w / WIDE_RATIO).floor();
    let wide_width = (h * WIDE_RATIO).ceil();
    let detail = format!(
        "Preserve every node, relationship, and label. This {}x{} canvas (ratio {}) declares no intrinsic-height fit and is below the {} wide ratio, so the desktop Reader can neither narrow it nor accept vertical scroll: it renders {}px tall at the full {}px width and the page reaches {}px before cards against {}px, a certain visual-check failure. Either compact vertical spacing so meta.viewBox height is at most {} at this width, or spread content sideways so the width is at least {} at this height; for architecture, omitting meta.viewBox lets the renderer size the canvas and declare the fit.",
        js_num(w), js_num(h), js_num(rounded_ratio), js_num(WIDE_RATIO), js_num(svg_height), js_num(svg_width), js_num(page), js_num(VIEWPORT.1), js_num(max_height), js_num(wide_width)
    );
    let evidence = json!({
        "viewportWidth": json_num(VIEWPORT.0), "viewportHeight": json_num(VIEWPORT.1), "ratio": json_num(rounded_ratio),
        "wideRatio": json_num(WIDE_RATIO), "svgWidthPx": json_num(svg_width), "svgHeightPx": json_num(svg_height),
        "fixedChromePx": json_num(FIXED_CHROME_PX), "pageHeightPx": json_num(page), "overflowPx": json_num(page - VIEWPORT.1),
    });
    Some((evidence, detail))
}

// ---- review evidence -----------------------------------------------------------------------------

struct NodeBox {
    id: String,
    label: String,
    b: [f64; 4],
}

/// `collectUntransformedNodeRects`: the opaque mask of every node group (a duplicated id is ambiguous
/// and dropped).
fn node_boxes(scene: &Scene) -> Vec<NodeBox> {
    let mut seen: Vec<(String, Option<NodeBox>)> = Vec::new();
    for (gi, g) in scene.groups.iter().enumerate().filter(|(_, g)| g.kind == GroupKind::Node) {
        let Some(id) = g.node_id.clone() else { continue };
        let mask = scene.items.iter().find_map(|i| match (&i.shape, i.group) {
            (Shape::Rect(r), Some(group)) if group.0 as usize == gi && is_mask(r) => Some(r.rect),
            _ => None,
        });
        let Some(b) = mask.filter(|b| b.x.is_finite() && b.y.is_finite() && b.width > 0.0 && b.height > 0.0 && b.width.is_finite() && b.height.is_finite()) else {
            continue;
        };
        match seen.iter_mut().find(|(known, _)| *known == id) {
            Some((_, slot)) => *slot = None,
            None => seen.push((id.clone(), Some(NodeBox { id: esc(&id), label: esc(&g.label), b: [b.x, b.y, b.width, b.height] }))),
        }
    }
    seen.into_iter().filter_map(|(_, n)| n).collect()
}

fn is_mask(r: &RectShape) -> bool {
    r.stroke.is_none() && r.fill.is_some_and(|f| f.token == Token::Mask)
}

/// `directCorridorBlockers`.
fn corridor_blockers(a: &Arrow, nodes: &[NodeBox]) -> Vec<Value> {
    let (Some(from), Some(to)) = (nodes.iter().find(|n| n.id == a.from), nodes.iter().find(|n| n.id == a.to)) else {
        return Vec::new();
    };
    if std::ptr::eq(from, to) {
        return Vec::new();
    }
    let center = |n: &NodeBox| [n.b[0] + n.b[2] / 2.0, n.b[1] + n.b[3] / 2.0];
    let (ca, cb) = (center(from), center(to));
    let horizontal = (ca[1] - cb[1]).abs() < 0.01;
    let vertical = (ca[0] - cb[0]).abs() < 0.01;
    if horizontal == vertical {
        return Vec::new();
    }
    let axis = usize::from(!horizontal);
    let cross = 1 - axis;
    let (first, last) = if ca[axis] < cb[axis] { (from, to) } else { (to, from) };
    let low = first.b[axis] + first.b[axis + 2];
    let high = last.b[axis];
    if high <= low {
        return Vec::new();
    }
    nodes
        .iter()
        .filter(|n| {
            !std::ptr::eq(*n, from)
                && !std::ptr::eq(*n, to)
                && n.b[axis] < high
                && n.b[axis] + n.b[axis + 2] > low
                && n.b[cross] < ca[cross]
                && n.b[cross] + n.b[cross + 2] > ca[cross]
        })
        .map(|n| json!({"id": n.id, "label": n.label, "box": n.b.iter().map(|v| json_num(*v)).collect::<Vec<_>>()}))
        .collect()
}

/// `crowdedNodeSides`.
fn crowded_sides(arrows: &[Arrow], nodes: &[NodeBox]) -> Vec<Value> {
    let mut demand: Vec<(String, &'static str, usize, usize)> = Vec::new();
    for a in arrows {
        let (Some(from), Some(to)) = (nodes.iter().position(|n| n.id == a.from), nodes.iter().position(|n| n.id == a.to)) else { continue };
        if from == to {
            continue;
        }
        for (node, other) in [(from, to), (to, from)] {
            let [x, y, w, h] = nodes[node].b;
            let o = nodes[other].b;
            let dx = o[0] + o[2] / 2.0 - (x + w / 2.0);
            let dy = o[1] + o[3] / 2.0 - (y + h / 2.0);
            let side = if dx != 0.0 && dx.abs() >= dy.abs() {
                if dx < 0.0 { "left" } else { "right" }
            } else if dy > 0.0 {
                "bottom"
            } else {
                "top"
            };
            match demand.iter_mut().find(|(_, s, _, n)| *s == side && *n == node) {
                Some(entry) => entry.2 += 1,
                None => demand.push((nodes[node].id.clone(), side, 1, node)),
            }
        }
    }
    demand
        .into_iter()
        .filter_map(|(_, side, relationships, node)| {
            let n = &nodes[node];
            let side_px = if side == "left" || side == "right" { n.b[3] } else { n.b[2] };
            let needed = 32.0 + 14.0 * (relationships as f64 - 1.0);
            (relationships > 1 && side_px < needed).then(|| {
                json!({"node": n.id, "label": n.label, "side": side, "relationships": relationships,
                    "sidePx": json_num(side_px), "neededPx": json_num(needed)})
            })
        })
        .collect()
}

/// `collectArchitectureLeadingSpace`: evidence for the author, never an error.
fn leading_space(input: &Input<'_>, arrows: &[Arrow], nodes: &[NodeBox], labels: &[LabelRect]) -> Value {
    let unmeasured = json!({"measured": false, "reviewSuggested": false});
    if input.root.reader_fit != Some("intrinsic-height") || !input.root.primary_text_14 {
        return unmeasured;
    }
    let [width, height] = input.scene.view_box;
    if !(width > 0.0 && height > 0.0) {
        return unmeasured;
    }
    let node_count = input.scene.groups.iter().filter(|g| g.kind == GroupKind::Node).count();
    if nodes.is_empty() || nodes.len() != node_count || arrows.iter().any(|a| a.semantic && a.points.is_empty()) {
        return unmeasured;
    }
    let mut occupied: Vec<f64> = nodes.iter().map(|n| n.b[1]).collect();
    for f in input.frames {
        occupied.push(match f.frame {
            geom::Frame::Line { start, end } => start[1].min(end[1]),
            geom::Frame::Rect { rect, .. } => rect.y,
        });
    }
    // Boundary titles can protrude above their structural frame.
    for (gi, g) in input.scene.groups.iter().enumerate().filter(|(_, g)| g.kind == GroupKind::Frame) {
        let _ = g;
        for item in &input.scene.items {
            if let (Shape::Rect(r), Some(group)) = (&item.shape, item.group)
                && group.0 as usize == gi
                && item.layer == Layer::FrameTitles
                && is_mask(r)
            {
                occupied.push(r.rect.y);
            }
        }
    }
    for a in arrows.iter().filter(|a| a.semantic) {
        occupied.extend(a.points.iter().map(|p| p[1]));
    }
    occupied.extend(labels.iter().map(|l| l.rect.y));
    if !occupied.iter().all(|v| v.is_finite()) {
        return unmeasured;
    }
    let occupied_top = occupied.iter().copied().fold(f64::INFINITY, f64::min);
    let gap = (occupied_top - 0.0).max(0.0);
    let mut heights: Vec<f64> = nodes.iter().map(|n| n.b[3]).collect();
    heights.sort_by(f64::total_cmp);
    let typical = heights[heights.len() / 2];
    let ratio = gap / height;
    json!({
        "measured": true,
        "emptyTopPx": json_num(round1(gap)),
        "emptyTopRatio": json_num(js_round(ratio * 1000.0) / 1000.0),
        "occupiedTop": json_num(round1(occupied_top)),
        "viewBoxTop": 0,
        "canvasHeight": json_num(height),
        "typicalNodeHeight": json_num(typical),
        "reviewSuggested": gap > 2.0 * typical && ratio > 0.2,
    })
}

/// `collectSequenceColumnSpace`: evidence for the author, never an error.
fn sequence_column_space(input: &Input<'_>, arrows: &[Arrow], nodes: &[NodeBox]) -> Option<Value> {
    let fit = input.root.column_fit?;
    let unmeasured = json!({"measured": false, "reviewSuggested": false, "columnFit": fit});
    let [width, height] = input.scene.view_box;
    if !(width > 0.0 && height > 0.0) {
        return Some(unmeasured);
    }
    let node_count = input.scene.groups.iter().filter(|g| g.kind == GroupKind::Node).count();
    if nodes.is_empty() || nodes.len() != node_count || arrows.iter().any(|a| a.semantic && a.points.is_empty()) {
        return Some(unmeasured);
    }
    let mut right_edges: Vec<f64> = nodes.iter().map(|n| n.b[0] + n.b[2]).collect();
    for a in arrows.iter().filter(|a| a.semantic) {
        right_edges.extend(a.points.iter().map(|p| p[0]));
    }
    for item in input.scene.items.iter().filter(|i| i.layer != Layer::Legend) {
        match &item.shape {
            // Label plates, activations and segment titles also reserve horizontal room.
            Shape::Rect(r) if is_mask(r) => right_edges.push(r.rect.x + r.rect.width),
            Shape::Text(t) => match text_box(t.at, t.size, t.anchor, &esc(t.text.trim())) {
                Some((b, _)) => right_edges.push(b[2]),
                None => return Some(unmeasured),
            },
            _ => {}
        }
    }
    if !right_edges.iter().all(|v| v.is_finite()) {
        return Some(unmeasured);
    }
    let occupied_right = right_edges.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let gap = (width - occupied_right).max(0.0);
    let ratio = gap / width;
    let mut widths: Vec<f64> = nodes.iter().map(|n| n.b[2]).collect();
    widths.sort_by(f64::total_cmp);
    let typical = widths[widths.len() / 2];
    Some(json!({
        "measured": true,
        "columnFit": fit,
        "participantCount": node_count,
        "occupiedRight": json_num(round1(occupied_right)),
        "viewBoxLeft": 0,
        "canvasWidth": json_num(width),
        "emptyRightPx": json_num(round1(gap)),
        "emptyRightRatio": json_num(js_round(ratio * 1000.0) / 1000.0),
        "typicalParticipantWidth": json_num(typical),
        "reviewSuggested": fit == "fixed" && node_count >= 4 && gap > 2.0 * typical && ratio > 0.25,
    }))
}

// ---- the checker ---------------------------------------------------------------------------------

const CHECK_FIXES: [(&str, &str); 4] = [
    ("single_svg", "remove additional SVG roots so the artifact contains exactly one diagram SVG"),
    ("finite_svg", "replace non-finite coordinates before rendering again"),
    ("orthogonal_arrows", "use renderer-supported orthogonal routing controls"),
    ("legend_clearance", "move the route or enlarge the viewBox so relationships do not enter the legend"),
];

const CONTROLS: &str = "if authored via/route/channelX/channelY controls exist and are not required by the user, remove them to let the renderer re-plan; otherwise preserve that intent and";
const LABEL_CONTROLS: &str = "if authored labelAt/labelDx/labelDy/labelSegment controls exist and are not required by the user, remove them to let the renderer re-plan; otherwise preserve that intent and";

/// `COMPOSITION_FIXES` and `compositionFixes`.
fn composition_fixes(issue: &Value) -> Vec<String> {
    let code = issue["code"].as_str().unwrap_or_default();
    match code {
        "composition/proper-crossing" => vec![format!("{CONTROLS} adjust route/via or channel coordinates so unrelated relationships use separate corridors")],
        "composition/ambiguous-corridor" => vec![format!("{CONTROLS} adjust route/via or channel coordinates so unrelated relationships do not visually merge")],
        "composition/container-border-run" => vec![format!("{CONTROLS} route across the frame perpendicularly through a clear opening")],
        "composition/label-route-clearance" => vec![format!("{LABEL_CONTROLS} adjust labelAt, labelDx, labelDy, labelSegment, message y, or the other relationship route")],
        "composition/label-canvas-containment" => vec![format!("{LABEL_CONTROLS} adjust labelAt, labelDx, labelDy, or labelSegment so the label rect stays inside the viewBox, or enlarge meta.viewBox")],
        "composition/micro-segment" => vec![format!("{CONTROLS} move the route/channel/via point so every visible segment is at least 8px")],
        "composition/short-interior-segment" => vec![format!("{CONTROLS} move the route/channel/via point so every interior turn has at least 16px")],
        "composition/viewport-height" => {
            let detail = issue["detail"].as_str().unwrap_or_default();
            vec![detail.split_once("] ").map_or(detail, |(_, rest)| rest).to_owned()]
        }
        "composition/desktop-readability" => vec![readability_fix(issue)],
        _ => Vec::new(),
    }
}

fn readability_fix(issue: &Value) -> String {
    let n = |key: &str| issue[key].as_f64().unwrap_or(f64::NAN);
    let (source, budget, floor, vb_width) = (n("sourceFontPx"), n("availableDiagramWidth"), n("minimumProjectedFontPx"), n("viewBoxWidth"));
    let preserve = "Preserve the semantic text and any supplied coordinates, routes, sides, channels, and labels.";
    let cap = if issue["budgetBasis"] == "recognized-declared-wide" {
        format!(
            " The declared Reader reports a {} limit and {}px actual diagram budget; do not assume an uncapped viewport.",
            issue["budgetLimit"].as_str().unwrap_or_default(),
            js_num(budget)
        )
    } else {
        String::new()
    };
    if ![source, budget, floor, vb_width].iter().all(|v| v.is_finite()) || budget <= 0.0 || floor <= 0.0 || vb_width <= 0.0 {
        return format!("{preserve} Repair the measured source font, desktop budget, or complete viewBox width; position-only label controls do not change projected text size.");
    }
    if source < floor {
        return format!(
            "{preserve} The diagnosed {}px source text is below the {}px hard floor even at scale 1, so use a renderer-supported semantic text-size setting or renderer-level fix. Position-only label controls cannot repair its projection.{cap}",
            js_num(source),
            js_num(floor)
        );
    }
    let max_width = (source * budget / floor).floor();
    format!(
        "{preserve} Compactly reflow automatic spacing and empty corridors so the complete viewBox width is at most {}px (current {}px; {}px source text at {}px desktop budget). If supplied geometry fixes that width, use a renderer-supported semantic text-size setting or renderer-level fix instead. Position-only label controls cannot repair its projection.{cap}",
        js_num(max_width),
        js_num(vb_width),
        js_num(source),
        js_num(budget)
    )
}

fn check_entry(name: &str, ok: bool, details: Vec<String>) -> Value {
    json!({"name": name, "ok": ok, "details": details})
}

/// Run the checker.
pub fn check(input: &Input<'_>) -> Report {
    let showcase = input.gate.showcase();
    let enforced = input.gate.declared();
    let profile = if showcase { "showcase" } else { "standard" };
    let v2 = input.root.workflow_v2;
    let arrows = arrows(input);
    let view_box = input.scene.view_box;

    // Route relations (`from && to && routePoints.length`), as the geometry collectors read them.
    let routed: Vec<usize> = (0..arrows.len()).filter(|&i| !arrows[i].from.is_empty() && !arrows[i].to.is_empty() && !arrows[i].points.is_empty()).collect();
    let rels: Vec<Rel> = routed.iter().map(|&i| arrows[i].as_rel()).collect();

    // Label masks, owned by the arrow whose key they carry.
    let labels: Vec<LabelRect> = input
        .labels
        .iter()
        .filter_map(|l| {
            let owner = arrows.iter().find(|a| a.key == l.rel)?;
            Some(LabelRect { text: owner.label.clone(), ..l.clone() })
        })
        .collect();
    let owner_of = |label: &LabelRect| arrows.iter().find(|a| a.key == label.rel).expect("labels are filtered to owned ones");

    let non_finite = finite_details(input.scene);
    let diagonal = diagonal_details(&arrows);

    let measured = crossings(&arrows, v2);
    let (resolved, crossing): (Vec<&Crossing>, Vec<&Crossing>) =
        measured.iter().partition(|h| arrows[h.left].crossover_halo && arrows[h.right].crossover_halo);

    let frames = input.frames;
    let border_routes: Vec<Vec<Seg>> = routed.iter().map(|&i| arrows[i].border.clone()).collect();
    let border_runs = comp::collect_border_runs_over(&border_routes, frames);

    let include_shared = |l: usize, r: usize| v2 || (arrows[routed[l]].independent_ports && arrows[routed[r]].independent_ports);
    let trunk = |c: &comp::Corridor, lp: &[Pt], rp: &[Pt]| {
        v2 && short_trunk(&arrows[routed[c.left]], &arrows[routed[c.right]], lp, rp, c.left_segment, c.right_segment, c.length)
    };
    let corridors = comp::collect_corridors_with(&rels, 8.0, &include_shared, &trunk);
    let arrow_exempt = |l: usize, r: usize, lp: &[Pt], rp: &[Pt], length: f64| {
        v2 && short_trunk(&arrows[routed[l]], &arrows[routed[r]], lp, rp, lp.len() - 2, rp.len() - 2, length)
    };
    let heads = comp::collect_arrowhead_collisions_with(&rels, &|pos| v2 || arrows[routed[pos]].independent_ports, &arrow_exempt);

    let rhythm = comp::collect_rhythm(&rels);
    let threshold = if showcase { 4.0 } else { 2.0 };
    // Label clearance reads every arrow's route, decorations included.
    let clear_idx: Vec<usize> = (0..arrows.len()).filter(|&i| arrows[i].points.len() >= 2).collect();
    let clear_rels: Vec<Rel> = clear_idx.iter().map(|&i| arrows[i].as_rel()).collect();
    let clearance = comp::collect_label_clearance(&labels, &clear_rels, threshold);
    let overflow = comp::collect_label_overflow(&labels, view_box, comp::LABEL_TOLERANCE_PX);
    let metrics = comp::route_metrics(&rels);
    let nodes = node_boxes(input.scene);
    let read = readability(input);
    let viewport = viewport_height(input);

    // Severity by profile, as the checker's `*IsError` constants.
    let sev = |error: bool| if error { "error" } else { "warning" };
    let rel_of_pos = |pos: usize| &arrows[routed[pos]];
    let mut issues: Vec<Value> = Vec::new();
    let mut details_border = Vec::new();
    for h in &border_runs {
        let a = rel_of_pos(h.rel);
        let info = &frames[h.frame];
        let kind = if info.kind.is_empty() { "frame".to_owned() } else { info.kind.clone() };
        let frame_id = match &info.id {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let side = comp_side_name(h.side);
        let length = round1(h.length);
        issues.push(json!({"severity": sev(enforced), "code": "composition/container-border-run",
            "relationship": a.record(), "frame": {"kind": kind, "id": frame_id}, "side": side,
            "segmentIndex": h.segment, "overlapLength": json_num(length), "from": pt1(h.start), "to": pt1(h.end)}));
        details_border.push(format!(
            "[composition/container-border-run] {} follows {kind} \"{frame_id}\" {side} border for {}px on segment {} [{}] -> [{}]",
            a.name(), js_num(length), h.segment, fmt_pt(h.start), fmt_pt(h.end)
        ));
    }
    let mut details_clearance = Vec::new();
    for h in &clearance {
        let label = &labels[h.label];
        let (owner, other) = (owner_of(label), &arrows[clear_idx[h.other]]);
        issues.push(json!({"severity": sev(showcase), "code": "composition/label-route-clearance",
            "label": label.text, "labelRelationship": owner.record(), "otherRelationship": other.record(),
            "segmentIndex": h.segment, "labelRect": rounded_rect(&label.rect), "clearance": json_num(round1(h.clearance)),
            "intersectionLength": json_num(round1(h.intersection)), "threshold": json_num(h.threshold),
            "from": pt1(h.start), "to": pt1(h.end)}));
        let hidden = if h.intersection > 0.0 { format!(" with {}px hidden by the mask", js_num(round1(h.intersection))) } else { String::new() };
        details_clearance.push(format!(
            "[composition/label-route-clearance] {profile} label \"{}\" on {} is {}px from {} segment {} [{}] -> [{}]{hidden} (minimum {}px) \u{2014} use renderer-supported label controls (message y for sequence; otherwise labelAt, labelDx, labelDy, or labelSegment), or adjust the other relationship route/via/channel.",
            label.text, owner.name(), js_num(round1(h.clearance)), other.name(), h.segment, fmt_pt(h.start), fmt_pt(h.end), js_num(h.threshold)
        ));
    }
    for h in &overflow {
        let label = &labels[h.label];
        let owner = owner_of(label);
        let overflow_px: Map<String, Value> = h.sides.iter().map(|(s, px)| ((*s).to_owned(), json_num(*px))).collect();
        let detail = format!(
            "[composition/label-canvas-containment] {profile} label \"{}\" on {} extends past the {} (label rect {}; viewBox {}x{}) \u{2014} use renderer-supported label controls (shorten the label or reorder participants for sequence; otherwise labelAt, labelDx, labelDy, or labelSegment), or enlarge meta.viewBox.",
            label.text, owner.name(), comp::describe_overflow(h), super::format_rect(&label.rect), js_num(view_box[0]), js_num(view_box[1])
        );
        issues.push(json!({"severity": sev(showcase), "code": "composition/label-canvas-containment",
            "label": label.text, "relationship": owner.record(), "labelRect": rounded_rect(&label.rect),
            "viewBox": [json_num(view_box[0]), json_num(view_box[1])], "viewBoxOrigin": [0, 0],
            "overflowPx": overflow_px, "detail": detail}));
    }
    let mut details_crossing = Vec::new();
    for h in &crossing {
        let (l, r) = (&arrows[h.left], &arrows[h.right]);
        issues.push(json!({"severity": sev(showcase), "code": "composition/proper-crossing",
            "relationship": l.record(), "otherRelationship": r.record(), "point": pt1(h.point)}));
        details_crossing.push(format!("[composition/proper-crossing] {profile} {} crosses {} at [{}]", l.name(), r.name(), fmt_pt(h.point)));
    }
    let mut details_corridor = Vec::new();
    for h in &corridors {
        let (l, r) = (rel_of_pos(h.left), rel_of_pos(h.right));
        issues.push(json!({"severity": sev(showcase), "code": "composition/ambiguous-corridor",
            "relationship": l.record(), "otherRelationship": r.record(), "segmentIndex": h.left_segment,
            "otherSegmentIndex": h.right_segment, "overlapLength": json_num(round1(h.length)),
            "from": pt1(h.start), "to": pt1(h.end)}));
        details_corridor.push(format!(
            "[composition/ambiguous-corridor] {profile} {} shares a {}px corridor with {} at [{}] -> [{}]",
            l.name(), js_num(round1(h.length)), r.name(), fmt_pt(h.start), fmt_pt(h.end)
        ));
    }
    for h in &heads {
        let (l, r) = (rel_of_pos(h.left), rel_of_pos(h.right));
        issues.push(json!({"severity": sev(showcase), "code": "composition/arrowhead-collision",
            "relationship": l.record(), "otherRelationship": r.record(), "distancePx": json_num(h.distance),
            "minimumPx": json_num(h.minimum), "endpoints": [pt(h.left_tip), pt(h.right_tip)]}));
        details_corridor.push(format!(
            "[composition/arrowhead-collision] {profile} {} and {} have incoming arrowheads {}px apart (minimum {}px) \u{2014} enlarge or reposition the destination, or choose separate toSide ports.",
            l.name(), r.name(), js_num(h.distance), js_num(h.minimum)
        ));
    }
    let mut details_rhythm = Vec::new();
    for h in &rhythm {
        let a = rel_of_pos(h.rel);
        let code = if h.micro { "composition/micro-segment" } else { "composition/short-interior-segment" };
        issues.push(json!({"severity": sev(showcase), "code": code, "relationship": a.record(),
            "segmentIndex": h.segment, "position": h.position.name(), "length": json_num(round1(h.length)),
            "from": pt1(h.start), "to": pt1(h.end)}));
        details_rhythm.push(format!(
            "[{code}] {profile} {} has a {}px {} segment {} [{}] -> [{}]",
            a.name(), js_num(round1(h.length)), h.position.name(), h.segment, fmt_pt(h.start), fmt_pt(h.end)
        ));
    }
    let readability_issue = read.issue.clone();
    if let Some(issue) = &readability_issue {
        let mut m = Map::new();
        m.insert("severity".into(), sev(showcase).into());
        m.insert("code".into(), "composition/desktop-readability".into());
        for (k, v) in issue.as_object().into_iter().flatten() {
            m.insert(k.clone(), v.clone());
        }
        issues.push(Value::Object(m));
    }
    if let Some((evidence, detail)) = &viewport {
        let mut m = Map::new();
        m.insert("severity".into(), "warning".into());
        m.insert("code".into(), "composition/viewport-height".into());
        m.insert("readerFit".into(), input.root.reader_fit.map_or(Value::Null, Value::from));
        m.insert("viewBoxWidth".into(), json_num(view_box[0]));
        m.insert("viewBoxHeight".into(), json_num(view_box[1]));
        for (k, v) in evidence.as_object().into_iter().flatten() {
            m.insert(k.clone(), v.clone());
        }
        m.insert("detail".into(), format!("[composition/viewport-height] {detail}").into());
        issues.push(Value::Object(m));
    }
    let count = |severity: &str| issues.iter().filter(|i| i["severity"] == severity).count();
    let (errors, warnings) = (count("error"), count("warning"));

    // The legend: every arrow segment against every padded legend box.
    let boxes = legend_boxes(input.scene);
    let mut details_legend = Vec::new();
    for a in &arrows {
        for s in &a.segments {
            for (b, label) in &boxes {
                if hits_box(s, b) {
                    details_legend.push(format!("path {} crosses legend {label}", a.index));
                }
            }
        }
    }

    let checks = vec![
        check_entry("single_svg", true, vec!["found 1 <svg> block(s)".to_owned()]),
        check_entry("finite_svg", non_finite.is_empty(), non_finite),
        check_entry("orthogonal_arrows", diagonal.is_empty(), diagonal),
        check_entry("label_route_clearance", !showcase || clearance.is_empty(), details_clearance),
        check_entry("relationship_crossings", !showcase || crossing.is_empty(), details_crossing),
        check_entry("relationship_corridors", !showcase || corridors.len() + heads.len() == 0, details_corridor),
        check_entry("container_border_runs", !enforced || border_runs.is_empty(), details_border),
        check_entry("route_rhythm", !showcase || rhythm.is_empty(), details_rhythm),
        check_entry("legend_clearance", details_legend.is_empty(), details_legend),
    ];

    // `routeReview`: the crossovers the renderer resolved, the crowded sides, the detours.
    let architecture = input.doc_type == "architecture";
    let review_crossings: Vec<Value> = resolved
        .iter()
        .map(|h| {
            let (l, r) = (&arrows[h.left], &arrows[h.right]);
            let shared = [&l.from, &l.to].into_iter().find(|id| **id == r.from || **id == r.to);
            let mut m = json!({"left": l.record(), "right": r.record(), "point": pt(h.point)});
            if let Some(shared) = shared {
                m["sharedNode"] = shared.clone().into();
            }
            m
        })
        .collect();
    let crowded = if architecture { crowded_sides(&arrows, &nodes) } else { Vec::new() };
    let detours: Vec<Value> = routed
        .iter()
        .filter_map(|&i| {
            let a = &arrows[i];
            let one = comp::route_metrics(&[a.as_rel()]);
            (one.routes_over_bends > 0 || one.routes_over_stretch > 0).then(|| {
                let blockers = corridor_blockers(a, &nodes);
                let mut m = json!({"relationship": a.record(), "bends": one.max_bends,
                    "stretch": opt(one.max_stretch.map(|s| js_round(s * 1000.0) / 1000.0))});
                if !blockers.is_empty() {
                    m["directCorridorBlockers"] = blockers.into();
                }
                m
            })
        })
        .collect();
    let mut route_review = json!({"crossings": review_crossings, "detours": detours});
    if !crowded.is_empty() {
        route_review["crowdedSides"] = crowded.into();
    }

    let mut composition = json!({
        "schemaVersion": 1,
        "profile": profile,
        "status": if errors > 0 { "fail" } else { "pass" },
        "summary": {"errors": errors, "warnings": warnings},
        "metrics": {
            "properCrossings": crossing.len(),
            "resolvedCrossovers": resolved.len(),
            "ambiguousCorridors": corridors.len(),
            "arrowheadCollisions": heads.len(),
            "containerBorderRuns": border_runs.len(),
            "labelRouteClearanceIssues": clearance.len(),
            "labelCanvasOverflowIssues": overflow.len(),
            "minLabelRouteClearance": opt(comp::min_label_clearance(&labels, &clear_rels)),
            "desktopReadabilityIssues": usize::from(readability_issue.is_some()),
            "viewportHeightIssues": usize::from(viewport.is_some()),
            "minProjectedNodeTextPx": opt(read.min_projected),
            "maxBends": metrics.max_bends,
            "routesOverSuggestedBends": metrics.routes_over_bends,
            "maxStretch": opt(metrics.max_stretch.map(|s| js_round(s * 1000.0) / 1000.0)),
            "routesOverSuggestedStretch": metrics.routes_over_stretch,
            "minSegmentPx": opt(metrics.min_segment.map(round1)),
            "minInteriorSegmentPx": opt(metrics.min_interior_segment.map(round1)),
            "shortSegmentCount": metrics.short_segments,
            "shortEndpointSegmentCount": metrics.short_endpoint_segments,
            "shortInteriorSegmentCount": metrics.short_interior_segments,
            "microSegmentCount": metrics.micro_segments,
        },
        "suggestedLimits": {"bendsPerRelationship": 2, "stretch": 1.35, "segmentPx": 16, "microSegmentPx": 8},
        "routeReview": route_review,
        "leadingSpace": leading_space(input, &arrows, &nodes, &labels),
        "desktopReadability": read.evidence,
        "issues": issues,
    });
    if let Some(space) = sequence_column_space(input, &arrows, &nodes) {
        composition["sequenceColumnSpace"] = space;
    }

    let ok = checks.iter().all(|c| c["ok"] == true) && errors == 0;
    let diagnostics = if ok { Vec::new() } else { diagnostics(&composition, &checks) };
    Report { ok, checks, composition, diagnostics }
}

fn comp_side_name(side: geom::BorderSide) -> &'static str {
    match side {
        geom::BorderSide::Top => "top",
        geom::BorderSide::Right => "right",
        geom::BorderSide::Bottom => "bottom",
        geom::BorderSide::Left => "left",
        geom::BorderSide::Line => "line",
    }
}

/// `checkerDiagnostics`: the composition errors first, then every failed non-composition check.
fn diagnostics(composition: &Value, checks: &[Value]) -> Vec<Diagnostic> {
    const COMPOSITION_CHECKS: [&str; 5] =
        ["label_route_clearance", "relationship_crossings", "relationship_corridors", "container_border_runs", "route_rhythm"];
    let mut out = Vec::new();
    for issue in composition["issues"].as_array().into_iter().flatten() {
        if issue["severity"] != "error" {
            continue;
        }
        let code = issue["code"].as_str().unwrap_or_default();
        let mut evidence = Map::new();
        for (k, v) in issue.as_object().into_iter().flatten() {
            if !matches!(k.as_str(), "severity" | "code" | "relationship" | "nodeId") {
                evidence.insert(k.clone(), v.clone());
            }
        }
        let subject = match issue.get("relationship") {
            Some(rel) if !rel.is_null() => Subject::default().with_extra("relationship", rel.clone()),
            _ => {
                let mut s = Subject::default().with_extra("check", "composition".into());
                if let Some(id) = issue.get("nodeId") {
                    s = s.with_extra("nodeId", id.clone());
                }
                s
            }
        };
        out.push(
            Diagnostic::error(code, &format!("Final artifact failed {code}."))
                .with_subject(subject)
                .with_evidence(evidence)
                .with_fixes(composition_fixes(issue)),
        );
    }
    for check in checks {
        let name = check["name"].as_str().unwrap_or_default();
        if check["ok"] == true || COMPOSITION_CHECKS.contains(&name) {
            continue;
        }
        let details: Vec<String> = check["details"].as_array().into_iter().flatten().filter_map(|d| d.as_str().map(str::to_owned)).collect();
        let message = details.iter().find(|d| !d.is_empty()).cloned().unwrap_or_else(|| format!("Final artifact failed {name}."));
        let fixes: Vec<&str> = CHECK_FIXES.iter().filter(|(n, _)| *n == name).map(|(_, f)| *f).collect();
        let evidence = Map::from_iter([("details".to_owned(), json!(details))]);
        out.push(
            Diagnostic::error(&format!("artifact/{}", name.replace('_', "-")), &message)
                .with_subject(Subject::default().with_extra("check", name.into()))
                .with_evidence(evidence)
                .with_fixes(fixes),
        );
    }
    if out.is_empty() {
        out.push(
            Diagnostic::error("artifact/check-failed", "Final artifact check failed without a classified diagnostic.")
                .with_subject(Subject::default().with_extra("check", "unknown".into())),
        );
    }
    out
}

/// The checker's own receipt, as `check-render-output.mjs` prints it (without the file and its hash,
/// which the in-memory scene does not have).
pub fn receipt(report: &Report, file: &str) -> Value {
    json!({"ok": report.ok, "file": file, "checks": report.checks, "composition": report.composition})
}

// ---- what each type hands the checker ------------------------------------------------------------

/// A type's gate review in the checker's terms: everything of [`Input`] but the scene and the gate.
#[derive(Clone, Debug, Default)]
pub struct Parts {
    pub rels: Vec<Rel>,
    pub labels: Vec<LabelRect>,
    pub frames: Vec<FrameInfo>,
    pub facts: Vec<EdgeFacts>,
    pub root: Root,
}

impl Parts {
    pub fn input<'a>(&'a self, doc_type: &'a str, scene: &'a Scene, gate: Gate) -> Input<'a> {
        Input { doc_type, scene, rels: &self.rels, facts: &self.facts, labels: &self.labels, frames: &self.frames, gate, root: self.root }
    }
}

/// `authoredStraightRouteAttrs`: a direct, explicitly authored diagonal.
fn straight_waiver(straight: bool, via: Option<usize>, points: &[Pt]) -> bool {
    straight
        && via.unwrap_or(0) == 0
        && points.len() == 2
        && (points[0][0] - points[1][0]).abs() > 0.01
        && (points[0][1] - points[1][1]).abs() > 0.01
}

/// `data-reader-fit` of the types that omit it for an authored viewBox (dataflow, lifecycle, sequence).
fn fit_unless_authored(authored: bool) -> Option<&'static str> {
    (!authored).then_some("intrinsic-height")
}

pub fn architecture(a: &crate::model::architecture::Architecture, review: &super::architecture::Review) -> Parts {
    use crate::model::architecture::ConnectionRoute;
    let connections = a.connections.as_deref().unwrap_or(&[]);
    let facts = review
        .rels
        .iter()
        .enumerate()
        .map(|(k, rel)| {
            let c = connections.get(rel.index);
            EdgeFacts {
                authored_straight: c.is_some_and(|c| {
                    straight_waiver(c.route == Some(ConnectionRoute::Straight), c.via.as_ref().map(Vec::len), &rel.points)
                }),
                independent: review.halo.get(k).copied().unwrap_or(false) && c.is_some_and(|c| c.label_at.is_none()),
                role: String::new(),
            }
        })
        .collect();
    let authored = a.meta.view_box.is_some();
    Parts {
        rels: review.rels.clone(),
        labels: review.labels.clone(),
        frames: review.frames.clone(),
        facts,
        root: Root {
            reader_fit: Some(if authored { "authored-height" } else { "intrinsic-height" }),
            arch_authored: authored,
            min_text: (!authored).then_some(7.5),
            primary_text_14: !authored,
            ..Root::default()
        },
    }
}

pub fn dataflow(d: &crate::model::dataflow::Dataflow, review: &super::dataflow::Review) -> Parts {
    use crate::model::dataflow::FlowRoute;
    let facts = review
        .rels
        .iter()
        .map(|rel| EdgeFacts {
            authored_straight: d.flows.get(rel.index).is_some_and(|f| {
                straight_waiver(f.route == Some(FlowRoute::Straight), f.via.as_ref().map(Vec::len), &rel.points)
            }),
            ..EdgeFacts::default()
        })
        .collect();
    Parts {
        rels: review.rels.clone(),
        labels: review.labels.clone(),
        frames: review.frames.clone(),
        facts,
        root: Root { reader_fit: fit_unless_authored(d.meta.view_box.is_some()), ..Root::default() },
    }
}

pub fn sequence(s: &crate::model::sequence::Sequence, review: &super::sequence::Review, fit: crate::model::sequence::ColumnFit) -> Parts {
    use crate::model::sequence::ColumnFit;
    Parts {
        rels: review.rels.clone(),
        labels: review.labels.clone(),
        frames: review.frames.clone(),
        facts: vec![EdgeFacts::default(); review.rels.len()],
        root: Root {
            reader_fit: fit_unless_authored(s.meta.view_box.is_some()),
            column_fit: Some(if fit == ColumnFit::Spread { "spread" } else { "fixed" }),
            ..Root::default()
        },
    }
}

pub fn lifecycle(l: &crate::model::lifecycle::Lifecycle, review: &super::lifecycle::Review) -> Parts {
    use crate::model::lifecycle::TransitionRoute;
    let facts = review
        .rels
        .iter()
        .map(|rel| EdgeFacts {
            authored_straight: l.transitions.get(rel.index).is_some_and(|t| {
                straight_waiver(t.route == Some(TransitionRoute::Straight), t.via.as_ref().map(Vec::len), &rel.points)
            }),
            ..EdgeFacts::default()
        })
        .collect();
    Parts {
        rels: review.rels.clone(),
        labels: review.labels.clone(),
        frames: review.frames.clone(),
        facts,
        root: Root { reader_fit: fit_unless_authored(l.meta.view_box.is_some()), ..Root::default() },
    }
}

/// A workflow: `v2` is the readable geometry of a `schema_version` 2 document (its canonical workflow
/// and lane heights decide the reader fit); v1 declares neither a contract nor a fit.
pub fn workflow(review: &super::workflow_v1::Review, v2: Option<&crate::layout::workflow::readable_build::ReadableGeometry>) -> Parts {
    use crate::model::workflow::Role;
    let Some(g) = v2 else {
        return Parts {
            rels: review.rels.clone(),
            labels: review.labels.clone(),
            frames: review.frames.clone(),
            facts: vec![EdgeFacts::default(); review.rels.len()],
            root: Root::default(),
        };
    };
    let w = g.placement.workflow();
    let facts = review
        .rels
        .iter()
        .map(|rel| {
            let e = w.edges.get(rel.index);
            EdgeFacts {
                authored_straight: false,
                independent: e.is_some_and(crate::route::workflow::independent_automatic_route),
                role: e
                    .and_then(|e| e.role)
                    .map(|r| match r {
                        Role::Main => "main",
                        Role::Branch => "branch",
                        Role::Async => "async",
                        Role::Return => "return",
                        Role::Error => "error",
                    })
                    .unwrap_or_default()
                    .to_owned(),
            }
        })
        .collect();
    let intrinsic = w.meta.view_box.is_none()
        && crate::layout::workflow::readable::has_vertical_stack(w)
        && g.placement.layout.lane_heights.iter().any(|h| *h > 104.0);
    Parts {
        rels: review.rels.clone(),
        labels: review.labels.clone(),
        frames: review.frames.clone(),
        facts,
        root: Root { workflow_v2: true, reader_fit: intrinsic.then_some("intrinsic-height"), ..Root::default() },
    }
}
