//! P9.3: the deferred motions M5-M8 through the public API: the relationship between two picked
//! nodes, the semantic lens, the route probe and the journey. Priority and preemption between the
//! owners, the lens cap, the dwell, the reduced-motion forms. (`tests/motion.rs` has the rest of
//! the model.)

use ubiq_archify::motion::{
    CometKind, LensFilter, Mode, MotionConfig, MotionState, Owner, RouteChoice, StrokeKind, sample,
};
use ubiq_archify::scene::{
    Bounds, EdgeRef, Group, GroupId, Layer, PolylineShape, RectShape, Scene, SceneBuilder, Shape, Stroke,
};
use ubiq_archify::tokens::{Kind, Token as Colour};

fn live() -> MotionConfig {
    MotionConfig::default()
}

fn not_capable() -> MotionConfig {
    MotionConfig::new(false)
}

fn line(points: &[[f64; 2]; 2]) -> Shape {
    Shape::Polyline(PolylineShape {
        points: points.to_vec(),
        radius: 0.0,
        stroke: Stroke::solid(Colour::Arrow, 1.4),
        marker: None,
        halo: false,
    })
}

/// a>b, b>c, a>d, d>c, c>e and f apart; kinds a frontend, b/d backend, c database, e/f cloud.
/// Node groups are 0..6; edge `i` (keys 1..) is group `5 + i`, 100 long.
fn graph() -> Scene {
    let mut b = SceneBuilder::new(400.0, 200.0);
    let kinds = [Kind::Frontend, Kind::Backend, Kind::Database, Kind::Backend, Kind::Cloud, Kind::Cloud];
    for (i, (id, kind)) in ["a", "b", "c", "d", "e", "f"].into_iter().zip(kinds).enumerate() {
        let bounds = Bounds::new(i as f64 * 50.0, 0.0, 40.0, 20.0);
        let g = b.group(Group::node(id, kind, id, bounds));
        b.push_in(g, Layer::Nodes, Shape::Rect(RectShape::mask(bounds, 6.0)));
    }
    for (i, (from, to)) in [("a", "b"), ("b", "c"), ("a", "d"), ("d", "c"), ("c", "e")].into_iter().enumerate() {
        let y = 10.0 * i as f64 + 50.0;
        let points = [[0.0, y], [100.0, y]];
        let edge = EdgeRef { key: i as u32 + 1, from: from.into(), to: to.into(), id: None };
        let g = b.group(Group::edge(edge, "", &points, 6.0));
        b.push_in(g, Layer::Edges, line(&points));
    }
    b.build()
}

/// `n` parallel edges x>y (x frontend, y backend).
fn parallel(n: u32) -> Scene {
    let mut b = SceneBuilder::new(100.0, 100.0);
    b.group(Group::node("x", Kind::Frontend, "x", Bounds::new(0.0, 0.0, 10.0, 10.0)));
    b.group(Group::node("y", Kind::Backend, "y", Bounds::new(50.0, 0.0, 10.0, 10.0)));
    for key in 0..n {
        let points = [[0.0, 0.0], [10.0, 0.0]];
        let edge = EdgeRef { key, from: "x".into(), to: "y".into(), id: None };
        let g = b.group(Group::edge(edge, "", &points, 2.0));
        b.push_in(g, Layer::Edges, line(&points));
    }
    b.build()
}

const SETTLED: f64 = 100_000.0;

fn alpha_of(s: &Scene, m: &MotionState, node: &str, t: f64) -> f64 {
    sample(s, m, t).alpha_of(s.node(node).unwrap())
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ---- the relationship (M5) ----

#[test]
fn a_relationship_hops_along_the_path_dims_the_rest_and_owns_the_budget() {
    let s = graph();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.relate(&s, "a", "c", 0.0), "a feeds c through b");
    assert_eq!(m.owner(), Some(Owner::Relationship));
    let rel = m.relationship().unwrap();
    assert_eq!(rel.nodes, ["a", "b", "c"]);
    assert!(!rel.direct);
    let delays: Vec<f64> = m.runs().iter().map(|r| r.timeline.delay_ms).collect();
    assert_eq!(delays, [0.0, 160.0], "one comet per hop");
    assert!(m.runs().iter().all(|r| r.kind == CometKind::Relationship));
    // The path stays lit and the rest fades to .16.
    assert!(near(alpha_of(&s, &m, "e", 180.0), 0.16));
    assert_eq!(alpha_of(&s, &m, "b", 180.0), 1.0);
    let mid = sample(&s, &m, 600.0);
    assert_eq!(mid.strokes.len(), 2, "both hops are running");
    assert!(mid.strokes.iter().all(|k| k.token == Colour::ArrowEmphasis));
    assert_eq!(mid.highlights.len(), 2, "the source and target glow");
    assert!(m.is_active(600.0));
    // Done: only the dim is held, nothing asks for frames.
    m.tick(SETTLED);
    assert!(!m.is_active(SETTLED) && sample(&s, &m, SETTLED).strokes.is_empty());
    // Clearing fades the dim back and hands the budget on.
    m.clear_relationship(SETTLED);
    assert_eq!(m.owner(), None);
    assert!(m.runs().is_empty());
    assert!(sample(&s, &m, SETTLED + 181.0).is_identity());
}

#[test]
fn a_direct_relationship_pulses_every_parallel_edge_together() {
    let mut b = SceneBuilder::new(100.0, 100.0);
    for (i, id) in ["a", "b"].into_iter().enumerate() {
        b.group(Group::node(id, Kind::Backend, id, Bounds::new(i as f64 * 50.0, 0.0, 10.0, 10.0)));
    }
    for (key, from, to) in [(1, "a", "b"), (2, "b", "a")] {
        let edge = EdgeRef { key, from: from.into(), to: to.into(), id: None };
        b.group(Group::edge(edge, "", &[[0.0, 0.0], [10.0, 0.0]], 2.0));
    }
    let s = b.build();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.relate(&s, "a", "b", 0.0) && m.relationship().unwrap().direct);
    assert!(m.runs().iter().all(|r| r.timeline.delay_ms == 0.0) && m.runs().len() == 2);
}

#[test]
fn a_relationship_needs_two_connected_nodes_and_changes_nothing_otherwise() {
    let s = graph();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(!m.relate(&s, "a", "f", 0.0), "f is not connected");
    assert!(!m.relate(&s, "a", "a", 0.0));
    assert!(!m.relate(&s, "a", "zz", 0.0));
    assert!(m.relationship().is_none() && m.owner().is_none() && m.runs().is_empty());
}

#[test]
fn a_relationship_clears_the_intent_trace_and_a_new_one_replaces_the_old() {
    let s = graph();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.hover_intent(&s, "a", 0.0));
    m.tick(90.0);
    assert!(m.relate(&s, "a", "b", 100.0));
    assert!(m.intent().is_none());
    assert!(m.runs().iter().all(|r| r.kind == CometKind::Relationship));
    assert!(!m.hover_intent(&s, "d", 120.0), "the relationship blocks the trace");
    assert!(m.relate(&s, "c", "e", 200.0));
    assert_eq!(m.relationship().unwrap().from, "c");
    assert_eq!(m.runs().len(), 1, "the first one's comets are gone");
}

// ---- priority and preemption ----

#[test]
fn route_beats_lens_beats_relationship_in_the_derived_owner() {
    let s = graph();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    for o in [Owner::Focus, Owner::Intent, Owner::Relationship, Owner::Lens, Owner::Route] {
        m.set_interaction(o, true, 0.0);
        assert_eq!(m.owner(), Some(o));
    }
    // With a lens or route flag up a relationship is refused: they own the budget.
    assert!(m.relationship_blocked());
    assert!(!m.relate(&s, "a", "b", 1.0));
    m.set_interaction(Owner::Route, false, 2.0);
    assert!(m.relationship_blocked() && m.owner() == Some(Owner::Lens));
    m.set_interaction(Owner::Lens, false, 3.0);
    assert!(!m.relationship_blocked() && m.owner() == Some(Owner::Relationship));
}

#[test]
fn starting_a_lens_ends_a_relationship_and_a_route_and_a_route_ends_a_lens() {
    let s = graph();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.relate(&s, "a", "b", 0.0));
    assert!(m.set_lens(&s, &[LensFilter::Kind(Kind::Database)], 10.0));
    assert!(m.relationship().is_none() && m.owner() == Some(Owner::Lens));
    assert!(m.runs().iter().all(|r| r.kind == CometKind::Lens));
    assert!(!m.relate(&s, "a", "b", 20.0), "blocked while the lens is up");

    assert!(m.route_begin(&s, "a", 30.0));
    assert!(m.lens().is_none() && m.owner() == Some(Owner::Route));
    assert!(m.runs().is_empty(), "the lens comets went with it");
    assert!(m.set_lens(&s, &[LensFilter::Kind(Kind::Backend)], 40.0));
    assert!(m.route().is_none() && m.owner() == Some(Owner::Lens));
    assert!(m.end_interaction(50.0) && m.owner().is_none());
    assert!(!m.end_interaction(51.0), "nothing left to end");
}

#[test]
fn a_journey_claim_beats_a_lens_flag_and_a_story_takes_it_over() {
    let s = graph();
    let mut m = MotionState::new(live(), &s, 0.0);
    m.tick(10_000.0);
    m.route_begin(&s, "a", 10_000.0);
    m.route_target(&s, "c", 10_000.0);
    m.route_play(10_000.0);
    m.set_interaction(Owner::Lens, true, 10_001.0);
    assert_eq!(m.owner(), Some(Owner::Route));
    let story = m.claim(Owner::Story, 10_002.0);
    assert_eq!(story.preempted, Some(Owner::Route), "a story takes the budget from a journey");
    assert_eq!(m.owner(), Some(Owner::Story));
}

// ---- the lens (M6) ----

#[test]
fn the_lens_dims_to_011_keeps_peers_at_062_and_runs_one_comet_per_matched_edge() {
    let s = graph();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.set_lens(&s, &[LensFilter::Kind(Kind::Database)], 0.0));
    let lens = m.lens().unwrap();
    assert_eq!((lens.edges, lens.quiet, lens.selected.len()), (3, false, 1));
    assert_eq!(m.runs().len(), 3);
    let delays: Vec<f64> = m.runs().iter().map(|r| r.timeline.delay_ms).collect();
    assert_eq!(delays, [0.0, 80.0, 160.0], "the lens delay is 80 ms a step");
    let t = 200.0;
    assert_eq!(alpha_of(&s, &m, "c", t), 1.0);
    assert!(near(alpha_of(&s, &m, "b", t), 0.62), "b feeds c: a peer");
    assert!(near(alpha_of(&s, &m, "f", t), 0.11), "f is outside the lens");
    assert_eq!(sample(&s, &m, t).highlights.len(), 1, "the selected node glows");
    assert!(!m.set_lens(&s, &[], 300.0), "an empty lens clears");
    assert!(m.lens().is_none() && m.runs().is_empty() && m.owner().is_none());
}

#[test]
fn a_lens_at_24_edges_runs_them_all_and_over_24_keeps_the_dim_without_comets() {
    let s = parallel(24);
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.set_lens(&s, &[LensFilter::Kind(Kind::Frontend)], 0.0));
    assert_eq!(m.runs().len(), 24, "exactly 24 still run");

    let s = parallel(25);
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.set_lens(&s, &[LensFilter::Kind(Kind::Frontend)], 0.0));
    let lens = m.lens().unwrap();
    assert!(lens.quiet && lens.edges == 25 && m.runs().is_empty(), "quiet density");
    assert!(near(alpha_of(&s, &m, "y", SETTLED), 0.62), "the dim holds: y is a peer");
    assert!(!m.is_active(SETTLED));
}

#[test]
fn a_lens_by_two_kinds_compares_the_direct_relationships_between_them() {
    let s = graph();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.set_lens(&s, &[LensFilter::Kind(Kind::Backend), LensFilter::Kind(Kind::Database)], 0.0));
    assert_eq!(m.lens().unwrap().edges, 2, "b>c and d>c");
    assert!(near(alpha_of(&s, &m, "a", 200.0), 0.11), "no peers in a comparison");
}

#[test]
fn a_lens_by_tag_selects_the_tagged_nodes() {
    let mut s = graph();
    for g in s.groups.iter_mut().filter(|g| matches!(g.node_id.as_deref(), Some("a" | "e"))) {
        g.tags.push("edge".into());
    }
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.set_lens(&s, &[LensFilter::Tag("edge".into())], 0.0));
    assert_eq!(m.lens().unwrap().selected.len(), 2);
    assert!(!m.set_lens(&s, &[LensFilter::Tag("nope".into())], 1.0), "a tag nothing carries selects nothing");
    assert!(m.lens().is_none());
}

#[test]
fn the_lens_under_reduced_motion_is_a_static_line_at_034_and_an_instant_dim() {
    let s = graph();
    let mut m = MotionState::new(MotionConfig { system_reduced: true, ..not_capable() }, &s, 0.0);
    assert!(m.set_lens(&s, &[LensFilter::Kind(Kind::Database)], 0.0));
    assert!(!m.is_active(10.0), "nothing animates");
    let o = sample(&s, &m, 10.0);
    assert_eq!(o.strokes.len(), 3);
    assert!(o.strokes.iter().all(|k| k.alpha == 0.34 && k.width == 2.2 && k.points.len() == 2));
    assert!(near(alpha_of(&s, &m, "f", 0.0), 0.11), "no fade in");
    // A relationship has no static form: only its dim shows.
    m.clear_lens(20.0);
    assert!(m.relate(&s, "a", "b", 30.0));
    assert!(sample(&s, &m, 40.0).strokes.is_empty());
    assert!(near(alpha_of(&s, &m, "f", 30.0), 0.16));
}

// ---- the route (M7, M8) ----

#[test]
fn a_route_resolves_the_shortest_directed_path_and_probes_it_hop_by_hop() {
    let s = graph();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert_eq!(m.route_target(&s, "c", 0.0), RouteChoice::Nothing, "no route begun");
    assert!(m.route_begin(&s, "a", 0.0));
    assert_eq!(m.route_target(&s, "a", 1.0), RouteChoice::Same);
    assert_eq!(m.route_target(&s, "f", 1.0), RouteChoice::Unreachable);
    assert_eq!(m.route_target(&s, "zz", 1.0), RouteChoice::Nothing);
    assert_eq!(m.route_target(&s, "e", 10.0), RouteChoice::Found);
    let r = m.route().unwrap();
    assert_eq!(r.nodes, ["a", "b", "c", "e"]);
    assert_eq!(r.index, None, "the overview: the whole route is lit");
    let delays: Vec<f64> = m.runs().iter().map(|r| r.timeline.delay_ms).collect();
    assert_eq!(delays, [0.0, 160.0, 320.0]);
    assert!(m.runs().iter().all(|r| r.kind == CometKind::RouteProbe));
    assert_eq!(m.owner(), Some(Owner::Route));
    // Lit: the route; dimmed to .11: d (off the path) and f.
    let t = 400.0;
    assert_eq!(alpha_of(&s, &m, "b", t), 1.0);
    assert!(near(alpha_of(&s, &m, "d", t), 0.11));
    // The probe ends parked at .58 and then costs nothing.
    let late = sample(&s, &m, SETTLED);
    assert_eq!(late.strokes.iter().filter(|k| k.alpha == 0.58).count(), 3);
    m.tick(SETTLED);
    assert!(!m.is_active(SETTLED));
}

#[test]
fn a_route_steps_along_the_first_unvisited_outgoing_edge_and_stops_at_a_dead_end() {
    let s = graph();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    assert!(m.route_begin(&s, "a", 0.0));
    assert_eq!(m.route_step(&s, 10.0).as_deref(), Some("b"));
    assert_eq!(m.route_step(&s, 20.0).as_deref(), Some("c"));
    assert_eq!(m.route_step(&s, 30.0).as_deref(), Some("e"));
    assert_eq!(m.route_step(&s, 40.0), None, "e feeds nothing");
    let r = m.route().unwrap();
    assert_eq!(r.nodes, ["a", "b", "c", "e"]);
    assert_eq!(r.index, Some(3));
    assert_eq!(r.window(), ["c", "e"], "the camera frames the node before and the one reached");
    // Each step pulses the edge just crossed (and only that one): c>e is edge 5, group 10.
    let kinds: Vec<_> = m.runs().iter().map(|r| (r.kind, r.group)).collect();
    assert_eq!(kinds, [(CometKind::RouteJourney, GroupId(10))]);
    // Past nodes sit at .62, the current at 1.
    assert!(near(alpha_of(&s, &m, "a", 1000.0), 0.62));
    assert_eq!(alpha_of(&s, &m, "e", 1000.0), 1.0);
    // Back and forth over the walk.
    assert!(m.route_back(50.0) && m.route().unwrap().index == Some(2));
    assert_eq!(m.route_step(&s, 60.0).as_deref(), Some("e"), "forward over the known walk");
    assert!(m.route_back(70.0) && m.route_back(80.0) && m.route_back(90.0) && !m.route_back(100.0));
}

#[test]
fn the_journey_dwells_1100_ms_a_node_pulses_each_edge_and_completes_without_looping() {
    let s = graph();
    let mut m = MotionState::new(live(), &s, 0.0);
    m.tick(10_000.0); // the ambient pass is over
    assert!(m.route_begin(&s, "a", 10_000.0));
    assert_eq!(m.route_target(&s, "c", 10_000.0), RouteChoice::Found);
    assert!(m.route_play(10_000.0));
    let r = m.route().unwrap();
    assert!(r.playing() && r.token.is_some() && r.index == Some(0));
    assert_eq!(m.owner(), Some(Owner::Route));
    assert!(m.runs().is_empty(), "the parked probe is replaced by the journey");
    assert!(m.is_active(10_001.0));
    // Frames of 100 ms: 1100 ms in, the journey is on node 1 with a pulse on the edge to it.
    let mut t = 10_000.0;
    while t < 11_100.0 {
        t += 100.0;
        m.tick(t);
    }
    assert_eq!(m.route().unwrap().index, Some(1));
    assert_eq!(m.runs().len(), 1);
    assert_eq!(m.runs()[0].kind, CometKind::RouteJourney);
    assert_eq!(m.runs()[0].timeline.duration_ms, 780.0);
    // The next dwell ends the walk: complete, claim given back, nothing loops.
    while t < 12_300.0 {
        t += 100.0;
        m.tick(t);
    }
    let r = m.route().unwrap();
    assert_eq!(r.index, Some(2));
    assert!(!r.playing() && r.token.is_none() && r.dwell.complete);
    assert_eq!(m.owner(), Some(Owner::Route), "the route flag still owns the budget");
    m.tick(t + 5000.0);
    assert_eq!(m.route().unwrap().index, Some(2));
    assert!(!m.is_active(t + 5000.0), "idle once the last pulse ends");
    // Playing again from the end restarts at the first node.
    assert!(m.route_play(t + 6000.0));
    assert_eq!(m.route().unwrap().index, Some(0));
}

#[test]
fn a_journey_survives_a_frame_gap_without_skipping_and_pauses_when_the_window_is_hidden() {
    let s = graph();
    let mut m = MotionState::new(live(), &s, 0.0);
    m.tick(10_000.0);
    m.route_begin(&s, "a", 10_000.0);
    m.route_target(&s, "e", 10_000.0);
    assert!(m.route_play(10_000.0));
    m.tick(10_050.0);
    // An hour with no frames (the window was hidden) counts for at most one capped frame.
    m.tick(10_050.0 + 3_600_000.0);
    assert_eq!(m.route().unwrap().index, Some(0), "no step was skipped");
    // A suspension pauses it for good: it does not resume on its own, and asks for no frames.
    m.suspend(MotionConfig::VISIBILITY, 3_700_000.0);
    m.tick(3_700_100.0);
    assert!(!m.route().unwrap().playing() && m.route().unwrap().token.is_none());
    assert!(!m.is_active(3_700_200.0));
    m.unsuspend(MotionConfig::VISIBILITY, 3_700_300.0);
    m.tick(3_700_400.0);
    assert!(!m.route().unwrap().playing());
}

#[test]
fn pause_keeps_the_dwell_and_a_manual_step_stops_the_journey() {
    let s = graph();
    let mut m = MotionState::new(live(), &s, 0.0);
    m.tick(10_000.0);
    m.route_begin(&s, "a", 10_000.0);
    m.route_target(&s, "e", 10_000.0);
    m.route_play(10_000.0);
    m.tick(10_100.0);
    m.tick(10_300.0);
    m.tick(10_500.0);
    m.route_pause(10_500.0);
    let r = m.route().unwrap();
    assert!(!r.playing() && r.token.is_none());
    assert_eq!(r.dwell.elapsed_ms, 500.0, "the elapsed dwell is kept for resume");
    assert_eq!(m.owner(), Some(Owner::Route));
    m.route_play(11_000.0);
    assert!(m.route().unwrap().playing());
    assert_eq!(m.route().unwrap().dwell.elapsed_ms, 500.0, "resume continues with the remainder");
    assert_eq!(m.route_step(&s, 11_100.0).as_deref(), Some("b"));
    assert!(!m.route().unwrap().playing(), "stepping by hand stops the journey");
}

#[test]
fn a_journey_is_refused_while_motion_is_paused_but_stepping_by_hand_still_works() {
    let s = graph();
    for config in [MotionConfig { mode: Mode::Still, ..live() }, MotionConfig { system_reduced: true, ..live() }] {
        let mut m = MotionState::new(config, &s, 0.0);
        m.route_begin(&s, "a", 0.0);
        m.route_target(&s, "c", 0.0);
        assert!(!m.route_play(1.0), "a journey needs an unpaused governor");
        assert!(m.route_step(&s, 2.0).is_some() && m.route().unwrap().index == Some(0));
        assert!(m.route_step(&s, 3.0).is_some() && m.route().unwrap().index == Some(1));
        let o = sample(&s, &m, 4.0);
        assert!(
            o.strokes.iter().all(|k| k.kind != StrokeKind::Comet(CometKind::RouteJourney)),
            "the journey pulse has no static form"
        );
    }
    let mut m = MotionState::new(MotionConfig { mode: Mode::Still, ..live() }, &s, 0.0);
    m.route_begin(&s, "a", 0.0);
    m.route_target(&s, "c", 0.0);
    let o = sample(&s, &m, 4.0);
    assert_eq!(o.strokes.len(), 2);
    assert!(o.strokes.iter().all(|k| k.alpha == 0.72), "static probe comets");
}

#[test]
fn ending_a_route_gives_back_the_claim_the_comets_and_the_dim() {
    let s = graph();
    let mut m = MotionState::new(live(), &s, 0.0);
    m.tick(10_000.0);
    m.route_begin(&s, "a", 10_000.0);
    m.route_target(&s, "c", 10_000.0);
    m.route_play(10_000.0);
    assert!(m.end_interaction(10_100.0));
    assert!(m.route().is_none() && m.owner().is_none() && m.runs().is_empty());
    assert!(sample(&s, &m, 10_100.0 + 181.0).is_identity());
    assert!(!m.is_active(10_100.0 + 181.0));
    // The released claim is stale: it cannot take the next owner down.
    assert!(m.route_begin(&s, "b", 20_000.0) && m.owner() == Some(Owner::Route));
}

#[test]
fn the_state_survives_a_serde_round_trip_mid_route() {
    let s = graph();
    let mut m = MotionState::new(not_capable(), &s, 0.0);
    m.route_begin(&s, "a", 0.0);
    m.route_target(&s, "c", 0.0);
    let back: MotionState = serde_json::from_value(serde_json::to_value(&m).unwrap()).unwrap();
    assert_eq!(back.route(), m.route());
    assert_eq!(sample(&s, &back, 500.0), sample(&s, &m, 500.0));
}
