//! Architecture end to end (P3.3/P3.4): [`build`] places the components ([`super::architecture`]),
//! routes every connection through [`crate::route::architecture`] with the renderer's options
//! (`render-architecture.mjs:314`: distinct ports, readable routes, the raw boundary frames, the
//! reserved label rect), finishes the canvas over the routes and the default label rects, and emits
//! the scene: frames, edges (marker, dash, crossover halo), components, labels, legend.
//!
//! Labels start at their default anchors (`labelPoint`); a showcase document then relocates them in
//! [`relocate_labels`], after the canvas is fixed, as Archify does (`:409`).

use crate::diag::{Diagnostic, Subject};
use crate::geom::{Frame, Pt, Rect};
use crate::labels::{self, Hints, Plate};
use crate::layout::architecture::{ArchitecturePlacement, begin_scene, prepare, push_components, push_legend};
use crate::legend::{Measured, relationship_obstacles};
use crate::model::architecture::{Architecture, Connection, ConnectionRoute};
use crate::model::common::{QualityProfile, Side as ModelSide, Variant};
use crate::route::architecture::{LabelRectFor, PlanningMetrics, RouteKind, RouterBox, RouterOptions, RouterRel, Sides, plan_routes};
use crate::scene::{Anchor, Detail, EdgeRef, Group, GroupKind, Layer, Marker, PolylineShape, RectShape, Scene, Shape, Stroke, TextShape};
use crate::tokens::EdgeVariant;

/// The rounded-corner radius of an architecture route (`roundedPath(points, 8)`).
pub const ROUTE_RADIUS: f64 = 8.0;
/// Stroke widths (`render-architecture.mjs:858`).
pub const EDGE_WIDTH: f64 = 1.5;
pub const EMPHASIS_WIDTH: f64 = 1.8;
/// The label mask radius and font (`renderConnectionLabel`, `:876-877`).
pub const LABEL_RADIUS: f64 = 3.0;
pub const LABEL_FONT: f64 = 8.0;
/// Edge hit rail (screen-independent, vu); not an Archify constant.
pub const EDGE_HIT_HALF_WIDTH: f64 = 6.0;

/// A connection label: the text, its anchor (`lx`, `ly`) and the mask rect.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelGeom {
    pub text: String,
    pub at: Pt,
    pub rect: Rect,
}

/// A routed connection.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionGeom {
    /// Its index in `connections`.
    pub index: usize,
    pub from: String,
    pub to: String,
    /// `pathFor(conn).points`, not rounded.
    pub points: Vec<Pt>,
    /// `connectionSides(conn)`.
    pub sides: Sides,
    pub stroke: f64,
    /// Automatic route geometry: drawn with the crossover halo (`data-composition-crossover`).
    pub halo: bool,
    /// `data-composition-independent`: an automatic route without an authored `labelAt`.
    pub independent: bool,
    pub label: Option<LabelGeom>,
}

/// Everything the gates (P3.6) and the receipt read.
#[derive(Clone, Debug, PartialEq)]
pub struct ArchitectureGeometry {
    pub placement: ArchitecturePlacement,
    /// The connections whose endpoints both exist, in document order.
    pub connections: Vec<ConnectionGeom>,
    /// `None` when no legend is drawn.
    pub legend: Option<Measured>,
    /// `legend/content-overlap` (or the band error) of an authored legend that does not fit: no
    /// legend is drawn, the geometry stands (Archify still prints it), the gate reports this.
    pub legend_problem: Option<Diagnostic>,
    pub metrics: PlanningMetrics,
}

impl ArchitectureGeometry {
    /// The placement alone, with no routes: what the gates measure when a component has no finite
    /// position and [`build`] refuses ([`validate`](crate::gates::architecture::validate) reports it).
    pub fn unrouted(doc: &Architecture) -> Self {
        ArchitectureGeometry {
            placement: crate::layout::architecture::place(doc),
            connections: Vec::new(),
            legend: None,
            legend_problem: None,
            metrics: PlanningMetrics::default(),
        }
    }
}

/// The geometry's variant name (`default` when unset).
pub fn variant_of(v: Option<Variant>) -> &'static str {
    variant_name(v)
}

pub(crate) fn side(s: ModelSide) -> crate::geom::Side {
    match s {
        ModelSide::Left => crate::geom::Side::Left,
        ModelSide::Right => crate::geom::Side::Right,
        ModelSide::Top => crate::geom::Side::Top,
        ModelSide::Bottom => crate::geom::Side::Bottom,
    }
}

fn variant_name(v: Option<Variant>) -> &'static str {
    match v {
        None | Some(Variant::Default) => "default",
        Some(Variant::Emphasis) => "emphasis",
        Some(Variant::Security) => "security",
        Some(Variant::Dashed) => "dashed",
    }
}

/// `conn.width || (conn.variant === 'emphasis' ? 1.8 : 1.5)`.
pub fn stroke_width(c: &Connection) -> f64 {
    c.width.filter(|w| *w != 0.0).unwrap_or(if c.variant == Some(Variant::Emphasis) { EMPHASIS_WIDTH } else { EDGE_WIDTH })
}

fn hints(c: &Connection) -> Hints {
    Hints { at: c.label_at, dx: c.label_dx, dy: c.label_dy, segment: c.label_segment }
}

fn label_text(c: &Connection) -> Option<&str> {
    c.label.as_deref().filter(|l| !l.is_empty())
}

/// The router's view of a connection.
pub fn router_rel(c: &Connection) -> RouterRel {
    RouterRel {
        id: c.id.clone(),
        from: c.from.clone(),
        to: c.to.clone(),
        label: c.label.clone(),
        from_side: c.from_side.map(side),
        to_side: c.to_side.map(side),
        route: match c.route {
            None | Some(ConnectionRoute::Auto) => RouteKind::Auto,
            Some(ConnectionRoute::Straight) => RouteKind::Straight,
            Some(ConnectionRoute::OrthogonalH) => RouteKind::OrthogonalH,
            Some(ConnectionRoute::OrthogonalV) => RouteKind::OrthogonalV,
        },
        via: c.via.clone(),
        channel_x: None,
        channel_y: None,
        label_at: c.label_at,
        label_placement: c.label_at.is_some() || c.label_dx.is_some() || c.label_dy.is_some() || c.label_segment.is_some(),
        stroke: stroke_width(c),
    }
}

/// The default label (`connectionLabelBoxAt(conn, labelPoint(conn, points))`), `None` unlabelled.
pub fn default_label(c: &Connection, points: &[Pt]) -> Option<LabelGeom> {
    let text = label_text(c)?;
    let at = labels::label_point(&hints(c), points);
    Some(LabelGeom { text: text.to_owned(), at, rect: labels::architecture_label_rect(text, at) })
}

/// The showcase relocation (`render-architecture.mjs:409-425`): `placeAutomaticLabels` over the
/// final canvas, the title rects and the legend band (`label_placement_bottom`). Only a showcase
/// document relocates; a pinned label (any authored `label*` hint) never moves.
pub fn relocate_labels(doc: &Architecture, placement: &ArchitecturePlacement, connections: &mut [ConnectionGeom]) {
    if doc.meta.quality_profile != Some(QualityProfile::Showcase) {
        return;
    }
    let authored: &[Connection] = doc.connections.as_deref().unwrap_or(&[]);
    let plates: Vec<(usize, Plate)> = connections
        .iter()
        .enumerate()
        .filter_map(|(k, g)| {
            let l = g.label.as_ref()?;
            let pinned = router_rel(&authored[g.index]).label_placement;
            Some((k, Plate { rel: g.index as i64, pinned, rect: l.rect, at: l.at }))
        })
        .collect();
    let routes: Vec<(i64, &[Pt])> = connections.iter().map(|g| (g.index as i64, g.points.as_slice())).collect();
    let components: Vec<Rect> = placement.components.iter().map(|c| c.rect).collect();
    let titles: Vec<Rect> = placement.boundaries.iter().map(|b| b.title.rect).collect();
    let flat: Vec<Plate> = plates.iter().map(|(_, p)| *p).collect();
    let placed = labels::place_automatic_labels(
        &flat,
        &labels::Placement {
            routes: &routes,
            components: &components,
            titles: &titles,
            view_box: placement.view_box,
            placement_bottom: placement.label_placement_bottom(),
            fallback_ring: true,
            keep_fallback_near_route: true,
        },
    );
    for ((k, _), plate) in plates.iter().zip(placed) {
        if let Some(l) = connections[*k].label.as_mut() {
            l.at = plate.at;
            l.rect = plate.rect;
        }
    }
}

fn constraint(message: String) -> Diagnostic {
    Diagnostic::error("layout/constraint", &message).with_subject(Subject::of("architecture").with_rule("component-geometry"))
}

/// Lay out an architecture document: the scene to paint and the geometry.
///
/// `Err` carries what makes the document unplaceable: a component without a finite position or
/// size (the renderer would route `NaN`s). An authored legend that does not fit is
/// [`ArchitectureGeometry::legend_problem`]; every other `validateArchitecture` rule is
/// `gates::architecture`'s.
pub fn build(doc: &Architecture) -> Result<(Scene, ArchitectureGeometry), Vec<Diagnostic>> {
    let prepared = prepare(doc);
    let broken: Vec<Diagnostic> = prepared
        .components
        .iter()
        .filter(|c| !c.rect.is_finite())
        .map(|c| constraint(format!("Component \"{}\" has non-finite pos/size \u{2014} pos and size must be [number, number].", c.id)))
        .collect();
    if !broken.is_empty() {
        return Err(broken);
    }

    let boxes: Vec<RouterBox> = prepared.components.iter().map(|c| RouterBox { id: c.id.clone(), rect: c.rect }).collect();
    let component_rects: Vec<Rect> = boxes.iter().map(|b| b.rect).collect();
    let connections: &[Connection] = doc.connections.as_deref().unwrap_or(&[]);
    let rels: Vec<RouterRel> = connections.iter().map(router_rel).collect();
    let frames: Vec<Frame> = prepared.raw_boundaries.iter().map(|b| Frame::Rect { rect: b.rect, radius: b.radius() }).collect();
    let options = RouterOptions::architecture(frames);
    let label_rect_for = |rel: usize, points: &[Pt], routes: &[&[Pt]], labels: &[Rect]| -> Option<Rect> {
        let c = &connections[rel];
        let label = default_label(c, points)?;
        let plate = Plate { rel: -1, pinned: rels[rel].label_placement, rect: label.rect, at: label.at };
        labels::reserved_label_rect(plate, points, routes, labels, &component_rects).map(|p| p.rect)
    };
    let label_rect_for: &LabelRectFor<'_> = &label_rect_for;
    let plan = plan_routes(&boxes, &rels, &options, Some(label_rect_for));

    let mut routed: Vec<ConnectionGeom> = Vec::new();
    for (index, (c, r)) in connections.iter().zip(&plan.routes).enumerate() {
        let Some(r) = r else { continue };
        let automatic = !rels[index].authored_geometry();
        routed.push(ConnectionGeom {
            index,
            from: c.from.clone(),
            to: c.to.clone(),
            points: r.points.clone(),
            sides: r.sides,
            stroke: rels[index].stroke,
            halo: automatic,
            independent: automatic && c.label_at.is_none(),
            label: default_label(c, &r.points),
        });
    }

    // `connectionGeometry`: the label rects, then every route point as a 0 x 0 rect.
    let extra: Vec<Rect> = routed
        .iter()
        .filter_map(|g| g.label.as_ref().map(|l| l.rect))
        .chain(routed.iter().flat_map(|g| g.points.iter().map(|p| Rect::new(p[0], p[1], 0.0, 0.0))))
        .collect();
    let placement = prepared.finish(&extra);
    relocate_labels(doc, &placement, &mut routed);

    let obstacles = relationship_obstacles(routed.iter().map(|g| (g.points.as_slice(), g.label.as_ref().map(|l| l.rect))));
    let (legend, legend_problem) = match placement.measure_legend(&obstacles) {
        Ok(legend) => (legend, None),
        Err(d) => (None, Some(*d)),
    };

    let mut b = begin_scene(&placement);
    for g in &routed {
        push_edge(&mut b, &connections[g.index], g);
    }
    push_components(&mut b, doc, &placement);
    for g in &routed {
        push_label(&mut b, &connections[g.index], g);
    }
    push_legend(&mut b, legend.as_ref(), &doc.meta.locale());
    let scene = b.build();
    Ok((scene, ArchitectureGeometry { placement, connections: routed, legend, legend_problem, metrics: plan.metrics }))
}

fn edge_ref(c: &Connection, g: &ConnectionGeom) -> EdgeRef {
    EdgeRef { key: g.index as u32, from: c.from.clone(), to: c.to.clone(), id: c.id.clone() }
}

fn push_edge(b: &mut crate::scene::SceneBuilder, c: &Connection, g: &ConnectionGeom) {
    let variant = EdgeVariant::parse(Some(variant_name(c.variant)));
    let label = label_text(c).unwrap_or_default();
    let id = b.group(Group::edge(edge_ref(c, g), label, &g.points, EDGE_HIT_HALF_WIDTH.max(g.stroke / 2.0)));
    b.push_in(
        id,
        Layer::Edges,
        Shape::Polyline(PolylineShape {
            points: g.points.clone(),
            radius: ROUTE_RADIUS,
            stroke: Stroke { token: variant.stroke(), width: g.stroke, dash: variant.dash().to_vec() },
            marker: Some(Marker { fill: variant.stroke() }),
            halo: g.halo,
        }),
    );
}

fn push_label(b: &mut crate::scene::SceneBuilder, c: &Connection, g: &ConnectionGeom) {
    let Some(label) = &g.label else { return };
    let variant = EdgeVariant::parse(Some(variant_name(c.variant)));
    let mut group = Group::new(GroupKind::Label, format!("label-{}", g.index), label.rect.into());
    group.edge = Some(edge_ref(c, g));
    group.label = label.text.clone();
    let id = b.group(group);
    b.push_in(id, Layer::EdgeLabels, Shape::Rect(RectShape::mask(label.rect.into(), LABEL_RADIUS)));
    b.push_in(
        id,
        Layer::EdgeLabels,
        Shape::Text(TextShape {
            at: label.at,
            text: label.text.clone(),
            size: LABEL_FONT,
            weight: 400,
            anchor: Anchor::Middle,
            token: variant.label(),
            detail: Detail::Context,
        }),
    );
}
