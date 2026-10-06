//! `sample(scene, state, t) -> Overlay`: the pure function of time (`04` §5.1; precedent
//! `V/export.js:792-1010`). No clock, no hidden state: the same inputs give the same overlay, so
//! it is golden-testable at fixed `t`, and an idle state gives an empty (identity) overlay.

use std::collections::BTreeMap;

use super::ambient::{self, Phase};
use super::comet::{CometKind, comet_frame, static_frame};
use super::dim::dimmable;
use super::overlay::{GroupAlpha, Highlight, Overlay, OverlayStroke, StrokeKind};
use super::state::MotionState;
use super::timeline::Ms;
use crate::geom::Pt;
use crate::scene::{Bounds, GroupId, GroupKind, Layer, PolylineShape, Scene, Shape};
use crate::tokens::{Kind, Token};

/// The corner radius of the ambient node outline when the scene has no node rect to copy
/// (the exporter's value).
const FALLBACK_RADIUS: f64 = 8.0;

/// The selected node's glow in the intent trace (`drop-shadow(0 0 10px frontend)`).
const SELECTED_GLOW_PX: f64 = 10.0;

pub fn sample(scene: &Scene, state: &MotionState, t: Ms) -> Overlay {
    let mut out = Overlay::default();
    let mut alphas: BTreeMap<GroupId, f64> = BTreeMap::new();
    let shapes = EdgeShapes::new(scene);

    ambient_pass(scene, state, t, &shapes, &mut out, &mut alphas);
    comets_pass(state, t, &shapes, &mut out);
    intent_selected(scene, state, t, &mut out);
    interaction_highlights(scene, state, t, &mut out);

    for dim in &state.dims {
        if dim.is_gone(t) {
            continue;
        }
        for (i, g) in scene.groups.iter().enumerate() {
            if dimmable(g.kind) {
                let id = GroupId(i as u32);
                *alphas.entry(id).or_insert(1.0) *= dim.alpha(scene, id, t);
            }
        }
    }
    out.group_alpha = alphas
        .into_iter()
        .filter(|(_, a)| (a - 1.0).abs() > 1e-9)
        .map(|(group, alpha)| GroupAlpha { group, alpha })
        .collect();

    if let Some(f) = state.lod.filter(|f| f.is_active(t)) {
        out.detail_alpha = Some(f.at(t));
    }
    out
}

/// The polylines of the edge groups, found once per sample, with each group's flattened outline
/// computed on demand.
struct EdgeShapes<'a> {
    by_group: BTreeMap<GroupId, &'a PolylineShape>,
}

impl<'a> EdgeShapes<'a> {
    fn new(scene: &'a Scene) -> Self {
        let mut by_group = BTreeMap::new();
        for item in scene.items.iter().filter(|i| i.layer == Layer::Edges) {
            if let (Some(g), Shape::Polyline(p)) = (item.group, &item.shape) {
                by_group.entry(g).or_insert(p);
            }
        }
        Self { by_group }
    }

    fn get(&self, g: GroupId) -> Option<&'a PolylineShape> {
        self.by_group.get(&g).copied()
    }

    fn flat(&self, g: GroupId) -> Vec<Pt> {
        self.get(g).map(PolylineShape::flattened).unwrap_or_default()
    }
}

fn ambient_pass(
    scene: &Scene,
    state: &MotionState,
    t: Ms,
    shapes: &EdgeShapes,
    out: &mut Overlay,
    alphas: &mut BTreeMap<GroupId, f64>,
) {
    let Phase::Running { start_ms, end_ms } = state.ambient.phase else { return };
    if !state.config.capable || t < start_ms || t >= end_ms {
        return;
    }
    let local = t - start_ms;
    let mut edge_ord = 0usize;
    for (i, g) in scene.groups.iter().enumerate() {
        let id = GroupId(i as u32);
        match g.kind {
            GroupKind::Edge => {
                let ord = edge_ord;
                edge_ord += 1;
                let p = (local - state.steps.edge_delay_ms(ord)) / ambient::EDGE_MS;
                if !(0.0..1.0).contains(&p) {
                    continue;
                }
                let (offset, alpha) = ambient::edge_at(p);
                *alphas.entry(id).or_insert(1.0) *= alpha;
                let Some(shape) = shapes.get(id) else { continue };
                for points in ambient::dashes(&shape.flattened(), offset) {
                    out.strokes.push(OverlayStroke {
                        kind: StrokeKind::AmbientDash,
                        group: id,
                        points,
                        token: shape.stroke.token,
                        width: shape.stroke.width,
                        non_scaling: false,
                        alpha: 1.0,
                        glow: 0.0,
                    });
                }
            }
            GroupKind::Node => {
                let Some(node_id) = g.node_id.as_deref() else { continue };
                let p = (local - state.steps.node_delay_ms(node_id)) / ambient::NODE_MS;
                if !(0.0..1.0).contains(&p) {
                    continue;
                }
                let k = ambient::node_pulse(p);
                if k <= 0.0 {
                    continue;
                }
                let (rect, radius) = node_rect(scene, id);
                out.highlights.push(Highlight {
                    group: id,
                    rect,
                    radius,
                    token: Token::ArrowEmphasis,
                    width: ambient::NODE_STROKE_REST + (ambient::NODE_STROKE_PEAK - ambient::NODE_STROKE_REST) * k,
                    alpha: k,
                    glow: ambient::NODE_GLOW_PX * k,
                });
            }
            _ => {}
        }
    }
}

/// The node's outline: its first rect in the node layer (the mask), else the group bounds.
fn node_rect(scene: &Scene, id: GroupId) -> (Bounds, f64) {
    scene
        .items_of(id)
        .find_map(|i| match (&i.shape, i.layer) {
            (Shape::Rect(r), Layer::Nodes) => Some((r.rect, r.radius)),
            _ => None,
        })
        .or_else(|| scene.group(id).map(|g| (g.bounds, FALLBACK_RADIUS)))
        .unwrap_or((Bounds::new(0.0, 0.0, 0.0, 0.0), FALLBACK_RADIUS))
}

fn comets_pass(state: &MotionState, t: Ms, shapes: &EdgeShapes, out: &mut Overlay) {
    let is_static = state.config.comets_static();
    for run in &state.runs {
        if t < run.timeline.start_ms {
            continue;
        }
        let flat = shapes.flat(run.group);
        let token = run.kind.token(run.direction);
        let params = run.kind.params();
        if is_static {
            if let Some((points, width, alpha)) = static_frame(run.kind, &flat) {
                out.strokes.push(OverlayStroke {
                    kind: StrokeKind::Comet(run.kind),
                    group: run.group,
                    points,
                    token,
                    width,
                    non_scaling: true,
                    alpha,
                    glow: if run.kind == CometKind::Lens { 0.0 } else { params.glow },
                });
            }
        } else if let Some(frame) = comet_frame(run.kind, &flat, &run.timeline, t) {
            out.strokes.push(OverlayStroke {
                kind: if frame.parked { StrokeKind::Parked(run.kind) } else { StrokeKind::Comet(run.kind) },
                group: run.group,
                points: frame.points,
                token,
                width: params.width,
                non_scaling: true,
                alpha: frame.alpha,
                glow: params.glow,
            });
        }
    }
}

/// How far the held dim of `kind` has taken effect (1 once settled, 0 when none is held).
fn held_depth(state: &MotionState, kind: super::dim::DimKind, t: Ms) -> f64 {
    state.dims.iter().find(|d| d.kind == kind && d.released.is_none()).map_or(0.0, |d| d.depth(t))
}

/// The glows of the deferred interactions (`viewer.css`: `data-relationship-preview-source`/
/// `-target` 7 px frontend/database, `data-lens-selected` 10 px database, the route's start and
/// end 11 px frontend/security and its steps 7 px backend), fading with their dim.
fn interaction_highlights(scene: &Scene, state: &MotionState, t: Ms, out: &mut Overlay) {
    use super::dim::DimKind;
    let mut glow = |group: GroupId, kind: Kind, px: f64, alpha: f64| {
        let (rect, radius) = node_rect(scene, group);
        out.highlights.push(Highlight {
            group,
            rect,
            radius,
            token: Token::KindStroke(kind),
            width: 0.0,
            alpha,
            glow: px,
        });
    };
    if let Some(r) = &state.relationship {
        let depth = held_depth(state, DimKind::Relationship, t);
        if let (Some(&a), Some(&b)) = (r.node_groups.first(), r.node_groups.last()) {
            glow(a, Kind::Frontend, 7.0, depth);
            glow(b, Kind::Database, 7.0, depth);
        }
    }
    if let Some(l) = &state.lens {
        let depth = held_depth(state, DimKind::Lens, t);
        for g in &l.selected {
            glow(*g, Kind::Database, 10.0, depth);
        }
    }
    if let Some(r) = &state.route {
        let depth = held_depth(state, DimKind::Route, t);
        let last = r.node_groups.len().saturating_sub(1);
        for (k, g) in r.node_groups.iter().enumerate() {
            let (kind, px) = match k {
                0 => (Kind::Frontend, 11.0),
                k if k == last => (Kind::Security, 11.0),
                _ => (Kind::Backend, 7.0),
            };
            glow(*g, kind, px, depth);
        }
    }
}

/// The selected node of an intent trace glows (`[data-intent-trace-selected]`), fading with the dim.
fn intent_selected(scene: &Scene, state: &MotionState, t: Ms, out: &mut Overlay) {
    let Some(intent) = state.intent.as_ref().filter(|i| t >= i.shown_at) else { return };
    let depth = state
        .dims
        .iter()
        .find(|d| d.kind == super::dim::DimKind::Intent && d.released.is_none())
        .map_or(1.0, |d| d.depth(t));
    let (rect, radius) = node_rect(scene, intent.node_group);
    out.highlights.push(Highlight {
        group: intent.node_group,
        rect,
        radius,
        token: Token::KindStroke(Kind::Frontend),
        width: 0.0,
        alpha: depth,
        glow: SELECTED_GLOW_PX,
    });
}
