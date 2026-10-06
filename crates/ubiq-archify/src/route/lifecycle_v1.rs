//! The architecture router as lifecycle v1 drives it (P4.3): `createRouter(states,
//! plannedTransitions, { labelRectFor })` of `render-lifecycle.mjs`. `03` section 3.4.
//!
//! Lifecycle v1 (and a v2 document whose authored sides contradict the grid plan) hands every
//! transition without `via`, a `route` preset or a channel to [`plan_routes`], with the router's
//! default options (no distinct automatic ports, no readability sweep, no frames) and a label
//! reservation that, unlike architecture's, sees the real canvas and the bottom the labels may not
//! pass (`lifecycleAreaBottom()`). The presets stay in [`crate::layout::lifecycle`].

use crate::geom::{Pt, Rect, rects_overlap};
use crate::labels::{self, Hints, Placement, Plate, label_point};
use crate::layout::lifecycle::{Plan, label_box_at, label_or_note, planner_routed};
use crate::model::common::Variant;
use crate::model::lifecycle::Transition;
use crate::route::architecture::{LabelRectFor, RouteKind, Routed, RouterBox, RouterOptions, RouterRel, plan_routes};
use crate::route::dataflow::side as model_side;

/// `reservedLabelRect` with the canvas and the placement bottom `labelRectFor` passes it
/// (`labels::reserved_label_rect` leaves both unbounded, as architecture's reservation does).
fn reserved_rect(
    label: Plate,
    points: &[Pt],
    routes: &[&[Pt]],
    others: &[Rect],
    components: &[Rect],
    view_box: [f64; 2],
    bottom: f64,
) -> Option<Rect> {
    let mut all: Vec<(i64, &[Pt])> = vec![(-1, points)];
    all.extend(routes.iter().enumerate().map(|(i, r)| (i as i64, *r)));
    let mut plates = vec![Plate { rel: -1, ..label }];
    plates.extend(others.iter().map(|r| Plate { rel: -2, pinned: false, rect: *r, at: [r.cx(), r.y + 10.0] }));
    let placement = Placement {
        routes: &all,
        components,
        titles: &[],
        view_box,
        placement_bottom: bottom,
        fallback_ring: false,
        keep_fallback_near_route: false,
    };
    // Only the first plate is read; the others are obstacles, and moving them changes nothing.
    let rect = labels::place_automatic_labels(&plates, &placement)[0].rect;
    (!components.iter().any(|c| rects_overlap(&rect, c, -2.0))).then_some(rect)
}

fn pinned(t: &Transition) -> bool {
    t.label_at.is_some() || t.label_dx.is_some() || t.label_dy.is_some() || t.label_segment.is_some()
}

/// The router's view of an automatic transition.
fn router_rel(t: &Transition) -> RouterRel {
    RouterRel {
        id: t.id.clone(),
        from: t.from.clone(),
        to: t.to.clone(),
        label: t.label.clone(),
        from_side: t.from_side.map(model_side),
        to_side: t.to_side.map(model_side),
        route: RouteKind::Auto,
        via: None,
        channel_x: None,
        channel_y: None,
        label_at: t.label_at,
        label_placement: pinned(t),
        stroke: t.width.filter(|w| *w != 0.0).unwrap_or(if t.variant == Some(Variant::Emphasis) { 1.8 } else { 1.5 }),
    }
}

/// Route every planner-routed transition of `plan` (those [`planner_routed`] accepts, in document
/// order): the entry is `None` when an endpoint is not a state.
pub fn plan(plan: &Plan<'_>) -> Vec<Option<Routed>> {
    let planned: Vec<&Transition> = plan.doc.transitions.iter().filter(|t| planner_routed(t)).collect();
    let boxes: Vec<RouterBox> = plan.states.iter().map(|s| RouterBox { id: s.id.clone(), rect: s.rect }).collect();
    let components: Vec<Rect> = boxes.iter().map(|b| b.rect).collect();
    let rels: Vec<RouterRel> = planned.iter().map(|t| router_rel(t)).collect();
    let (view_box, bottom) = (plan.view_box, plan.area_bottom);
    let label_rect_for = |rel: usize, points: &[Pt], routes: &[&[Pt]], reserved: &[Rect]| -> Option<Rect> {
        let t = planned[rel];
        label_or_note(t)?;
        let hints = Hints { at: t.label_at, dx: t.label_dx, dy: t.label_dy, segment: t.label_segment };
        let label = label_box_at(t, label_point(&hints, points));
        let plate = Plate { rel: -1, pinned: pinned(t), rect: label.rect, at: label.at };
        reserved_rect(plate, points, routes, reserved, &components, view_box, bottom)
    };
    let label_rect_for: &LabelRectFor<'_> = &label_rect_for;
    plan_routes(&boxes, &rels, &RouterOptions::default(), Some(label_rect_for)).routes
}
