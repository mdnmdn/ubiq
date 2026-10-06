//! Focus, dim and reachability on a [`Scene`] (P2.8, `00` §7): which groups stay lit when a node
//! is focused. Pure: the painter asks and draws; no GPUI here.
//!
//! - `Near` (the default): the node, its incident edges and the nodes at their other ends.
//! - `Up` / `Down`: a breadth-first walk over the authored directed edges, against (`Up`, what
//!   feeds the node) or along (`Down`, what the node feeds) their direction. Cycles end at the
//!   first revisit.
//!
//! Everything else is dimmed, but not here: `animate.rs` hands the lit set to the core's focus dim
//! (`DIM_FOCUS`, a 180 ms fade; frames and the legend are never dimmed), and the painter reads the
//! group multipliers (P7.3). An edge's label group follows its edge.

use std::collections::{HashSet, VecDeque};

use ubiq_archify::scene::{Bounds, GroupId, GroupKind, Scene};

/// What a focused node lights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reach {
    #[default]
    Near,
    Up,
    Down,
}

impl Reach {
    pub const ALL: [Reach; 3] = [Reach::Near, Reach::Up, Reach::Down];

    pub fn label(self) -> &'static str {
        match self {
            Reach::Near => "Near",
            Reach::Up => "Upstream",
            Reach::Down => "Downstream",
        }
    }

    /// `want` when it is not already the mode, else back to `Near`: what the `u` and `d` keys do.
    pub fn toggled(self, want: Reach) -> Reach {
        if self == want { Reach::Near } else { want }
    }
}

/// What a click on the picture does, by what is under it, the shift key and what is already going
/// on (`click_plan`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Click {
    /// Focus this node, or clear the focus (a click on nothing). Ends a lens or a relationship.
    Focus(Option<String>),
    /// Shift-click on a second node while one is focused: how the two relate.
    Relate { from: String, to: String },
    /// A route is on: the node is its destination.
    Route(String),
    /// A route is on and the click hit no node.
    Nothing,
}

/// The pure half of the picture's click: `target` is the node hit (if any), `shift` the modifier,
/// `focus` the focused node and `route` whether a route is on (its clicks choose a destination).
pub fn click_plan(target: Option<String>, shift: bool, focus: Option<&str>, route: bool) -> Click {
    match target {
        Some(node) if route => Click::Route(node),
        None if route => Click::Nothing,
        Some(to) if shift && focus.is_some_and(|from| from != to) => Click::Relate {
            from: focus.map(str::to_string).unwrap_or_default(),
            to,
        },
        other => Click::Focus(other),
    }
}

/// The groups that stay lit around a focused node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lit {
    /// The focused node's group.
    pub focus: GroupId,
    /// Every lit group: the focus, the other nodes, their edges and those edges' labels.
    pub groups: HashSet<GroupId>,
    /// The nodes lit besides the focus, in discovery order.
    pub others: Vec<GroupId>,
    pub reach: Reach,
}

/// The lit set for `node_id` under `reach`, or `None` when the scene has no such node.
pub fn lit(scene: &Scene, node_id: &str, reach: Reach) -> Option<Lit> {
    let focus = scene.node(node_id)?;
    // Every edge as (group, key, from, to).
    let edges: Vec<(GroupId, u32, &str, &str)> = (0..scene.groups.len() as u32)
        .map(GroupId)
        .filter_map(|id| {
            let g = scene.group(id)?;
            let e = g.edge.as_ref().filter(|_| g.kind == GroupKind::Edge)?;
            Some((id, e.key, e.from.as_str(), e.to.as_str()))
        })
        .collect();

    let mut nodes: Vec<&str> = vec![node_id];
    let mut lit_edges: Vec<(GroupId, u32)> = Vec::new();
    match reach {
        Reach::Near => {
            for (id, key, from, to) in &edges {
                let other = if *from == node_id {
                    to
                } else if *to == node_id {
                    from
                } else {
                    continue;
                };
                lit_edges.push((*id, *key));
                if !nodes.contains(other) {
                    nodes.push(other);
                }
            }
        }
        Reach::Up | Reach::Down => {
            let mut queue = VecDeque::from([node_id]);
            while let Some(at) = queue.pop_front() {
                for (id, key, from, to) in &edges {
                    let (here, next) = match reach {
                        Reach::Down => (from, to),
                        _ => (to, from),
                    };
                    if *here != at || lit_edges.iter().any(|(seen, _)| seen == id) {
                        continue;
                    }
                    lit_edges.push((*id, *key));
                    if !nodes.contains(next) {
                        nodes.push(next);
                        queue.push_back(next);
                    }
                }
            }
        }
    }

    let mut groups: HashSet<GroupId> = HashSet::from([focus]);
    let mut others = Vec::new();
    for id in nodes.iter().skip(1).filter_map(|n| scene.node(n)) {
        groups.insert(id);
        others.push(id);
    }
    groups.extend(lit_edges.iter().map(|(id, _)| *id));
    let keys: HashSet<u32> = lit_edges.iter().map(|(_, key)| *key).collect();
    for id in (0..scene.groups.len() as u32).map(GroupId) {
        let Some(g) = scene.group(id) else { continue };
        if g.kind == GroupKind::Label && g.edge.as_ref().is_some_and(|e| keys.contains(&e.key)) {
            groups.insert(id);
        }
    }
    Some(Lit {
        focus,
        groups,
        others,
        reach,
    })
}

/// The pan (px, in the panel) that brings `rect` into view, or `None` when it already is.
///
/// `scale` and `offset` are the camera's (a viewBox point `p` lands at `offset + p * scale` from the
/// panel's top-left), `panel` is its size, and `margin` is the gap kept between the box and the
/// panel's edge. A box that is out of view, or that touches the margin, is centred rather than
/// nudged: one move, and the focused node's neighbourhood lies around it. The painter and the
/// base viewport own the camera (`D14`); this only says by how much to move it.
pub fn reveal_delta(
    scale: f32,
    offset: [f32; 2],
    panel: [f32; 2],
    rect: Bounds,
    margin: f32,
) -> Option<[f32; 2]> {
    let left = offset[0] + rect.x as f32 * scale;
    let top = offset[1] + rect.y as f32 * scale;
    let right = left + rect.width as f32 * scale;
    let bottom = top + rect.height as f32 * scale;
    let inside = left >= margin
        && top >= margin
        && right <= panel[0] - margin
        && bottom <= panel[1] - margin;
    if inside {
        return None;
    }
    Some([
        panel[0] / 2.0 - (left + right) / 2.0,
        panel[1] / 2.0 - (top + bottom) / 2.0,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_archify::scene::{Bounds, EdgeRef, Group, SceneBuilder};
    use ubiq_archify::tokens::Kind;

    /// a -> b -> c, d -> b, and a loop e -> f -> e apart from them. Edge keys are 1..=5 (a->b, b->c,
    /// d->b, e->f, f->e), each with a label group of the same key.
    fn scene() -> Scene {
        let mut b = SceneBuilder::new(100.0, 100.0);
        for (i, id) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
            b.group(Group::node(
                id,
                Kind::Backend,
                id,
                Bounds::new(i as f64 * 10.0, 0.0, 8.0, 8.0),
            ));
        }
        b.group(Group::new(
            GroupKind::Frame,
            "frame-stage-x",
            Bounds::new(0.0, 0.0, 100.0, 100.0),
        ));
        for (key, from, to) in [
            (1, "a", "b"),
            (2, "b", "c"),
            (3, "d", "b"),
            (4, "e", "f"),
            (5, "f", "e"),
        ] {
            let edge = EdgeRef {
                key,
                from: from.into(),
                to: to.into(),
                id: None,
            };
            b.group(Group::edge(
                edge.clone(),
                "",
                &[[0.0, 0.0], [5.0, 5.0]],
                2.0,
            ));
            let mut label = Group::new(
                GroupKind::Label,
                format!("label-{key}"),
                Bounds::new(0.0, 0.0, 4.0, 4.0),
            );
            label.edge = Some(edge);
            b.group(label);
        }
        b.build()
    }

    fn ids(scene: &Scene, lit: &Lit) -> Vec<String> {
        let mut out: Vec<String> = lit
            .groups
            .iter()
            .filter_map(|g| scene.group(*g))
            .map(|g| g.id.clone())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn near_lights_the_node_its_edges_and_their_other_ends() {
        let s = scene();
        let l = lit(&s, "b", Reach::Near).unwrap();
        assert_eq!(
            ids(&s, &l),
            [
                "edge-1", "edge-2", "edge-3", "label-1", "label-2", "label-3", "node-a", "node-b",
                "node-c", "node-d"
            ]
        );
        assert_eq!(l.others.len(), 3);
    }

    #[test]
    fn down_follows_the_edges_and_up_runs_against_them() {
        let s = scene();
        let down = lit(&s, "a", Reach::Down).unwrap();
        assert_eq!(
            ids(&s, &down),
            [
                "edge-1", "edge-2", "label-1", "label-2", "node-a", "node-b", "node-c"
            ]
        );
        let up = lit(&s, "c", Reach::Up).unwrap();
        assert_eq!(
            ids(&s, &up),
            [
                "edge-1", "edge-2", "edge-3", "label-1", "label-2", "label-3", "node-a", "node-b",
                "node-c", "node-d"
            ]
        );
        // Nothing is downstream of a sink, nothing upstream of a source.
        assert_eq!(ids(&s, &lit(&s, "c", Reach::Down).unwrap()), ["node-c"]);
        assert_eq!(ids(&s, &lit(&s, "a", Reach::Up).unwrap()), ["node-a"]);
    }

    #[test]
    fn a_cycle_ends_at_the_first_revisit() {
        let s = scene();
        let l = lit(&s, "e", Reach::Down).unwrap();
        assert_eq!(
            ids(&s, &l),
            ["edge-4", "edge-5", "label-4", "label-5", "node-e", "node-f"]
        );
    }

    #[test]
    fn an_unknown_node_lights_nothing() {
        assert!(lit(&scene(), "zz", Reach::Near).is_none());
    }

    #[test]
    fn a_click_plan_chooses_focus_relationship_or_route_destination() {
        let node = |n: &str| Some(n.to_string());
        // Plain: focus the node, or clear on empty space.
        assert_eq!(click_plan(node("b"), false, Some("a"), false), Click::Focus(node("b")));
        assert_eq!(click_plan(None, false, Some("a"), false), Click::Focus(None));
        // Shift on a second node while one is focused: the relationship.
        assert_eq!(
            click_plan(node("b"), true, Some("a"), false),
            Click::Relate { from: "a".into(), to: "b".into() }
        );
        // Shift on the focused node itself, or with nothing focused, is a plain click.
        assert_eq!(click_plan(node("a"), true, Some("a"), false), Click::Focus(node("a")));
        assert_eq!(click_plan(node("b"), true, None, false), Click::Focus(node("b")));
        // A route owns the clicks: a node is the destination (shift or not), space does nothing.
        assert_eq!(click_plan(node("b"), false, None, true), Click::Route("b".into()));
        assert_eq!(click_plan(node("b"), true, Some("a"), true), Click::Route("b".into()));
        assert_eq!(click_plan(None, false, Some("a"), true), Click::Nothing);
    }

    #[test]
    fn the_keys_toggle_between_a_mode_and_near() {
        assert_eq!(Reach::Near.toggled(Reach::Up), Reach::Up);
        assert_eq!(Reach::Up.toggled(Reach::Up), Reach::Near);
        assert_eq!(Reach::Up.toggled(Reach::Down), Reach::Down);
    }

    #[test]
    fn a_box_in_view_stays_and_one_out_of_view_is_centred() {
        let panel = [400.0, 300.0];
        // Scale 2, offset (10, 10): a 20x10 box at (50, 40) sits at (110..150, 90..110).
        let inside = Bounds::new(50.0, 40.0, 20.0, 10.0);
        assert_eq!(reveal_delta(2.0, [10.0, 10.0], panel, inside, 24.0), None);

        // The same box at x = 400 is at 810..850: off to the right. Its centre (830, 100) goes to
        // the panel's centre (200, 150).
        let far = Bounds::new(400.0, 40.0, 20.0, 10.0);
        let delta = reveal_delta(2.0, [10.0, 10.0], panel, far, 24.0).unwrap();
        assert_eq!(delta, [-630.0, 50.0]);

        // Inside the panel but inside the margin too: still moved.
        let edge = Bounds::new(0.0, 40.0, 10.0, 10.0);
        assert!(reveal_delta(2.0, [10.0, 10.0], panel, edge, 24.0).is_some());
    }
}
