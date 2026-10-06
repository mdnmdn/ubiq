//! P7.1: the motion model through its public API. Invariants from `motion-governor`/`animation`/
//! `intent-trace` tests, and `sample` goldens at fixed `t` on a hand-made scene (serde snapshots
//! inline, floats rounded to 4 dp).

use ubiq_archify::motion::{
    CometKind, DimKind, Direction, Dwell, Mode, MotionConfig, MotionState, Overlay, Owner, Phase, Settle, Token,
    sample,
};
use ubiq_archify::scene::{
    Bounds, EdgeRef, Group, GroupId, Layer, Marker, PolylineShape, RectShape, Scene, SceneBuilder, Shape, Stroke, Tier,
};
use ubiq_archify::tokens::{Kind, Token as Colour};
use serde_json::{Value, json};

/// a ---> b, and an unconnected c. Node groups: a 0, b 1, c 2; the edge group is 3.
fn scene() -> Scene {
    let mut b = SceneBuilder::new(200.0, 140.0);
    for (id, x, y) in [("a", 0.0, 40.0), ("b", 160.0, 40.0), ("c", 80.0, 100.0)] {
        let bounds = Bounds::new(x, y, 40.0, 20.0);
        let g = b.group(Group::node(id, Kind::Backend, id, bounds));
        b.push_in(g, Layer::Nodes, Shape::Rect(RectShape::mask(bounds, 6.0)));
    }
    let points = [[40.0, 50.0], [160.0, 50.0]];
    let edge = b.group(Group::edge(
        EdgeRef { key: 0, from: "a".into(), to: "b".into(), id: None },
        "",
        &points,
        6.0,
    ));
    b.push_in(
        edge,
        Layer::Edges,
        Shape::Polyline(PolylineShape {
            points: points.to_vec(),
            radius: 0.0,
            stroke: Stroke::solid(Colour::Arrow, 1.4),
            marker: Some(Marker { fill: Colour::Arrow }),
            halo: false,
        }),
    );
    b.build()
}

const C: GroupId = GroupId(2);
const EDGE: GroupId = GroupId(3);

fn live() -> MotionConfig {
    MotionConfig::default()
}

fn not_capable() -> MotionConfig {
    MotionConfig::new(false)
}

/// The overlay as JSON with every float rounded to 4 dp.
fn snap(o: &Overlay) -> Value {
    fn round(v: Value) -> Value {
        match v {
            Value::Number(n) if n.is_f64() => {
                let x = (n.as_f64().unwrap() * 1e4).round() / 1e4;
                json!(if x == 0.0 { 0.0 } else { x })
            }
            Value::Array(a) => Value::Array(a.into_iter().map(round).collect()),
            Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, round(v))).collect()),
            other => other,
        }
    }
    round(serde_json::to_value(o).unwrap())
}

/// Compare a golden; on a mismatch print only the actual value, so a deliberate change is easy to bless.
fn golden(actual: &Overlay, expected: Value) {
    let actual = snap(actual);
    assert!(actual == expected, "golden mismatch, actual:\n{actual}");
}

// ---- the governor ----

#[test]
fn a_document_without_trace_has_no_ambient_and_no_claims() {
    let s = scene();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert_eq!(m.ambient().phase, Phase::Pending);
    assert!(sample(&s, &m, 100.0).is_identity() && !m.is_active(100.0));
    assert_eq!(m.claim(Owner::Story, 0.0).token, Token(0), "every method no-ops");
}

#[test]
fn ambient_starts_once_stays_inside_the_budget_and_settles() {
    let s = scene();
    let mut m = MotionState::new(live(), &s, 1000.0);
    // edge step 0 (2400), a 0 (3600), b 1 (160 + 3600), c 2 (320 + 3600).
    assert!(m.is_active(1000.0));
    assert!(m.is_active(1000.0 + 3919.0) && !m.is_active(1000.0 + 3920.0));
    assert!(!sample(&s, &m, 1500.0).is_identity());
    m.tick(1000.0 + 3920.0);
    assert_eq!(m.ambient().phase, Phase::Settled(Settle::Complete));
    assert!(sample(&s, &m, 1000.0 + 3920.0).is_identity(), "settled equals the static scene");
    // It never restarts: not by Still -> Live, not by a suspension ending.
    m.set_mode(Mode::Still, 9000.0);
    m.set_mode(Mode::Live, 9100.0);
    m.suspend("visibility", 9200.0);
    m.unsuspend("visibility", 9300.0);
    assert_eq!(m.ambient().phase, Phase::Settled(Settle::Complete));
    assert!(!m.is_active(9400.0));
}

#[test]
fn an_owner_or_a_pause_settles_the_ambient_trace_for_good() {
    let s = scene();
    let mut m = MotionState::new(live(), &s, 0.0);
    m.set_interaction(Owner::Focus, true, 100.0);
    assert_eq!(m.ambient().phase, Phase::Settled(Settle::Suppressed));
    m.set_interaction(Owner::Focus, false, 200.0);
    assert!(sample(&s, &m, 300.0).is_identity() && !m.is_active(300.0));

    let still = MotionConfig { mode: Mode::Still, ..live() };
    let mut p = MotionState::new(still, &s, 0.0);
    assert_eq!(p.ambient().phase, Phase::Settled(Settle::Suppressed), "paused at load: the one pass is spent");
    p.set_mode(Mode::Live, 50.0);
    assert!(sample(&s, &p, 60.0).is_identity() && !p.is_active(60.0));

    let mut r = MotionState::new(MotionConfig { system_reduced: true, ..live() }, &s, 0.0);
    assert!(matches!(r.ambient().phase, Phase::Settled(Settle::Suppressed)));
    r.set_system_reduced(false, 1.0);
    assert!(!r.is_active(2.0));
}

#[test]
fn a_hidden_window_requests_no_frames_and_a_claim_beats_the_flags() {
    let s = scene();
    let mut m = MotionState::new(live(), &s, 0.0);
    m.suspend(MotionConfig::VISIBILITY, 10.0);
    assert!(!m.is_active(20.0));
    m.set_interaction(Owner::Legend, true, 30.0);
    m.set_interaction(Owner::Lens, true, 30.0);
    assert_eq!(m.owner(), Some(Owner::Lens));
    let c = m.claim(Owner::Story, 40.0);
    assert_eq!(m.owner(), Some(Owner::Story));
    assert!(m.release(c.token, 50.0) && !m.release(c.token, 51.0));
    assert_eq!(m.owner(), Some(Owner::Lens));
}

// ---- the intent trace ----

#[test]
fn intent_waits_ninety_ms_then_owns_the_budget_and_costs_nothing_once_held() {
    let s = scene();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.hover_intent(&s, "a", 0.0));
    assert!(sample(&s, &m, 89.0).is_identity(), "nothing before the hover delay");
    assert!(m.is_active(89.0), "but the next frame is the show");
    m.tick(90.0);
    assert_eq!(m.owner(), Some(Owner::Intent));
    assert!(m.is_active(100.0));
    // Comets 1150 ms, dim 180 ms: after that the held preview needs no frames.
    assert!(m.is_active(90.0 + 1149.0) && !m.is_active(90.0 + 1150.0));
    let held = sample(&s, &m, 5000.0);
    assert!(held.strokes.is_empty(), "the comet is one-shot");
    assert!((held.alpha_of(C) - 0.2).abs() < 1e-12, "the dim stays while the preview is held");
    assert_eq!(held.alpha_of(EDGE), 1.0);
}

#[test]
fn keyboard_focus_shows_at_once_and_a_second_node_replaces_the_first() {
    let s = scene();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.focus_intent(&s, "a", 10.0));
    assert!(!sample(&s, &m, 10.0).is_identity() || m.is_active(10.0));
    assert!(m.focus_intent(&s, "b", 20.0));
    assert_eq!(m.intent().unwrap().node, "b");
    assert_eq!(m.runs().len(), 1, "the edge, once, as an in-edge of b");
    assert_eq!(m.runs()[0].direction, Direction::In);
    assert!(!m.focus_intent(&s, "zzz", 30.0) && m.intent().is_none());
}

#[test]
fn directions_are_out_in_and_loop_relative_to_the_node() {
    let mut b = SceneBuilder::new(100.0, 100.0);
    b.group(Group::node("n", Kind::Backend, "n", Bounds::new(0.0, 0.0, 10.0, 10.0)));
    b.group(Group::node("m", Kind::Backend, "m", Bounds::new(50.0, 0.0, 10.0, 10.0)));
    for (key, from, to) in [(0, "n", "m"), (1, "m", "n"), (2, "n", "n")] {
        let pts = [[0.0, 0.0], [10.0, 0.0]];
        let g = b.group(Group::edge(EdgeRef { key, from: from.into(), to: to.into(), id: None }, "", &pts, 4.0));
        b.push_in(
            g,
            Layer::Edges,
            Shape::Polyline(PolylineShape {
                points: pts.to_vec(),
                radius: 0.0,
                stroke: Stroke::solid(Colour::Arrow, 1.4),
                marker: None,
                halo: false,
            }),
        );
    }
    let s = b.build();
    let mut st = MotionState::new(not_capable(), &s, 0.0);
    st.focus_intent(&s, "n", 0.0);
    let dirs: Vec<Direction> = st.runs().iter().map(|r| r.direction).collect();
    assert_eq!(dirs, vec![Direction::Out, Direction::In, Direction::Loop]);
    let tokens: Vec<Colour> = st.runs().iter().map(|r| r.kind.token(r.direction)).collect();
    assert_eq!(
        tokens,
        vec![
            Colour::KindStroke(Kind::Frontend),
            Colour::KindStroke(Kind::Database),
            Colour::KindStroke(Kind::Security)
        ]
    );
}

#[test]
fn intent_is_blocked_by_lens_relationship_route_focus_or_a_claim() {
    let s = scene();
    for owner in [Owner::Lens, Owner::Relationship, Owner::Route, Owner::Focus] {
        let mut m = MotionState::new(not_capable(), &s, 0.0);
        m.set_interaction(owner, true, 0.0);
        assert!(!m.hover_intent(&s, "a", 0.0), "{owner:?}");
        assert!(m.intent().is_none());
    }
    let mut m = MotionState::new(live(), &s, 0.0);
    m.claim(Owner::Story, 0.0);
    assert!(!m.focus_intent(&s, "a", 1.0));
    let mut ok = MotionState::new(not_capable(), &s, 0.0);
    ok.set_interaction(Owner::Legend, true, 0.0);
    assert!(ok.hover_intent(&s, "a", 0.0), "a legend preview does not block it");
}

#[test]
fn clearing_removes_the_comets_at_once_and_fades_the_dim_back_in_180_ms() {
    let s = scene();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    m.focus_intent(&s, "a", 0.0);
    m.tick(0.0);
    assert!(!sample(&s, &m, 300.0).strokes.is_empty());
    m.clear_intent(300.0);
    let o = sample(&s, &m, 300.0);
    assert!(o.strokes.is_empty() && o.highlights.is_empty(), "overlays are removed immediately");
    assert!(m.owner().is_none());
    assert!(m.is_active(479.0) && !m.is_active(480.0));
    let mid = sample(&s, &m, 390.0).alpha_of(C);
    assert!(mid > 0.2 && mid < 1.0, "fading back: {mid}");
    m.tick(480.0);
    assert!(sample(&s, &m, 480.0).is_identity(), "back to the static scene");
}

#[test]
fn reduced_motion_has_no_hover_delay_static_comets_and_instant_dims() {
    let s = scene();
    let mut m = MotionState::new(MotionConfig { system_reduced: true, ..not_capable() }, &s, 0.0);
    assert!(m.hover_intent(&s, "a", 0.0));
    assert_eq!(m.intent().unwrap().shown_at, 0.0, "0 ms under reduced motion");
    assert!(!m.is_active(0.0) && !m.is_active(1000.0), "nothing moves, so no frames");
    golden(
        &sample(&s, &m, 10.0),
        json!({
          "strokes": [{"kind": {"comet": "intent"}, "group": 3, "points": [[40.0, 50.0], [160.0, 50.0]],
            "token": {"kind-stroke": "frontend"}, "width": 3.1, "non_scaling": true, "alpha": 0.72, "glow": 4.0}],
          "highlights": [{"group": 0, "rect": {"x": 0.0, "y": 40.0, "width": 40.0, "height": 20.0}, "radius": 6.0,
            "token": {"kind-stroke": "frontend"}, "width": 0.0, "alpha": 1.0, "glow": 10.0}],
          "group_alpha": [{"group": 2, "alpha": 0.2}],
          "detail_alpha": null
        }),
    );
}

#[test]
fn still_parks_comets_as_static_lines_but_keeps_the_fades() {
    let s = scene();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    m.set_mode(Mode::Still, 0.0);
    m.focus_intent(&s, "a", 0.0);
    let o = sample(&s, &m, 40.0);
    assert_eq!((o.strokes[0].alpha, o.strokes[0].points.len()), (0.72, 2));
    assert!(o.alpha_of(C) < 1.0 && o.alpha_of(C) > 0.2, "Still keeps the 180 ms transition");
}

// ---- other comets ----

#[test]
fn the_route_probe_comets_hop_every_160_ms_and_park_at_058() {
    let s = scene();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    m.play_comets(CometKind::RouteProbe, &[(EDGE, Direction::Out), (EDGE, Direction::Out)], 0.0, 160.0);
    assert_eq!(m.runs()[1].timeline.delay_ms, 160.0);
    let late = sample(&s, &m, 10_000.0);
    assert_eq!(late.strokes.len(), 2);
    assert!(late.strokes.iter().all(|s| s.alpha == 0.58 && s.points.len() == 2));
    m.clear_comets(CometKind::RouteProbe);
    assert!(sample(&s, &m, 10_000.0).is_identity());
}

#[test]
fn the_lens_animates_at_most_24_edges() {
    let s = scene();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    let edges = vec![(EDGE, Direction::Within); 40];
    m.play_comets(CometKind::Lens, &edges, 0.0, 30.0);
    assert_eq!(m.runs().len(), 24);
}

// ---- fades ----

#[test]
fn a_tier_change_cross_fades_detail_over_160_ms_and_the_first_call_does_not() {
    let s = scene();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    m.set_tier(Tier::Map, 0.0);
    assert!(!m.is_active(1.0) && sample(&s, &m, 1.0).detail_alpha.is_none());
    m.set_tier(Tier::Read, 1000.0);
    let mid = sample(&s, &m, 1080.0).detail_alpha.unwrap();
    assert!(mid.context > 0.0 && mid.context < 1.0 && mid.fine == 0.0);
    assert!(m.is_active(1159.0) && !m.is_active(1160.0));
    m.tick(1160.0);
    assert!(sample(&s, &m, 1160.0).detail_alpha.is_none(), "the painter's tier rule takes over");
}

#[test]
fn a_dim_replaces_the_previous_dim_of_its_kind() {
    let s = scene();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    let focus = |node| ubiq_archify::motion::Dim::neighbourhood(DimKind::Focus, &s, node, 0.0, true).unwrap();
    m.dim(focus("a"), 0.0);
    m.dim(focus("c"), 100.0);
    m.tick(100.0);
    let o = sample(&s, &m, 200.0);
    assert_eq!(o.alpha_of(C), 1.0);
    assert!((o.alpha_of(GroupId(0)) - 0.13).abs() < 1e-9);
}

// ---- purity and the journey dwell ----

#[test]
fn sample_is_a_pure_function_of_state_and_time() {
    let s = scene();
    let mut m = MotionState::new(live(), &s, 0.0);
    m.tick(0.0);
    let a = sample(&s, &m, 1234.5);
    assert_eq!(a, sample(&s, &m, 1234.5));
    assert_ne!(a, sample(&s, &m, 1500.0));
    // A state survives a serde round trip and samples the same.
    let back: MotionState = serde_json::from_value(serde_json::to_value(&m).unwrap()).unwrap();
    assert_eq!(a, sample(&s, &back, 1234.5));
    // And an overlay does too.
    let o: Overlay = serde_json::from_value(serde_json::to_value(&a).unwrap()).unwrap();
    assert_eq!(o, a);
}

#[test]
fn the_journey_dwell_is_resumable_and_finite() {
    let mut d = Dwell::journey(3);
    d.play();
    d.advance(600.0);
    d.pause();
    d.advance(60_000.0);
    d.play();
    assert_eq!(d.remaining_ms(), 500.0);
    d.advance(500.0);
    d.advance(1100.0);
    assert!(d.complete && d.index == 2);
}

// ---- goldens ----

#[test]
fn golden_ambient_pass_at_1200_ms() {
    let s = scene();
    let m = MotionState::new(live(), &s, 0.0);
    // Edge p = .5: dash offset 54 * (1 - .5/.88) = 23.318, edge alpha .42 + .58 * .5/.88 = .7495;
    // dashes sit at 18k - 23.318 along the 120 long edge (the first is clipped at the source).
    // Node a is at p = 1/3, b (delay 160) at .289, c (delay 320) at .244: all on the 18-36 %
    // plateau, so full pulse (stroke 2.4, glow 8).
    let dash = |from: f64, to: f64| {
        json!({"kind": "ambient-dash", "group": 3, "points": [[from, 50.0], [to, 50.0]],
               "token": "arrow", "width": 1.4, "non_scaling": false, "alpha": 1.0, "glow": 0.0})
    };
    let pulse = |group: u32, x: f64, y: f64| {
        json!({"group": group, "rect": {"x": x, "y": y, "width": 40.0, "height": 20.0}, "radius": 6.0,
               "token": "arrow-emphasis", "width": 2.4, "alpha": 1.0, "glow": 8.0})
    };
    golden(
        &sample(&s, &m, 1200.0),
        json!({
          "strokes": [dash(40.0, 44.6818), dash(52.6818, 62.6818), dash(70.6818, 80.6818), dash(88.6818, 98.6818),
                      dash(106.6818, 116.6818), dash(124.6818, 134.6818), dash(142.6818, 152.6818)],
          "highlights": [pulse(0, 0.0, 40.0), pulse(1, 160.0, 40.0), pulse(2, 80.0, 100.0)],
          "group_alpha": [{"group": 3, "alpha": 0.7495}],
          "detail_alpha": null
        }),
    );
}

#[test]
fn golden_intent_trace_at_half_way() {
    let s = scene();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    m.hover_intent(&s, "a", 0.0);
    m.tick(90.0);
    // Shown at 90 ms; at 665 the comet is at p = .5: dash [60, 72] of the 120 edge, out of a, so the
    // frontend colour, alpha .94. c is outside the neighbourhood: .2.
    golden(
        &sample(&s, &m, 665.0),
        json!({
          "strokes": [{"kind": {"comet": "intent"}, "group": 3, "points": [[100.0, 50.0], [112.0, 50.0]],
            "token": {"kind-stroke": "frontend"}, "width": 3.1, "non_scaling": true, "alpha": 0.94, "glow": 4.0}],
          "highlights": [{"group": 0, "rect": {"x": 0.0, "y": 40.0, "width": 40.0, "height": 20.0}, "radius": 6.0,
            "token": {"kind-stroke": "frontend"}, "width": 0.0, "alpha": 1.0, "glow": 10.0}],
          "group_alpha": [{"group": 2, "alpha": 0.2}],
          "detail_alpha": null
        }),
    );
}
