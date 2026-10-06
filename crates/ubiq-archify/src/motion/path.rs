//! Pure graph helpers over a scene's edge groups (`V/route-probe.js:265-334`, `V/focus.js` direct
//! relationships): the breadth-first shortest directed path of the route probe, the outgoing
//! edges a journey steps along, and the relationship between two picked nodes. No clock, no state.
//!
//! BFS order follows the scene's edge order (the authored order), so equal-length paths resolve to
//! the first one. A self-loop and an edge whose end is not a node of the scene are not links.

use std::collections::{HashMap, VecDeque};

use crate::scene::{GroupId, GroupKind, Scene};

/// One authored edge between two nodes of the scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub group: GroupId,
    pub key: u32,
    pub from: String,
    pub to: String,
}

/// A walk through the graph: `nodes.len() == edges.len() + 1`, and `edges[i]` joins `nodes[i]` and
/// `nodes[i + 1]` (against its direction when the walk was found undirected).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    pub nodes: Vec<String>,
    pub edges: Vec<GroupId>,
}

/// What two picked nodes have between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub path: Path,
    /// The nodes are joined by an edge of their own (parallel edges and both directions included).
    pub direct: bool,
}

/// Every group id of a scene, in order.
pub(super) fn ids(scene: &Scene) -> impl Iterator<Item = GroupId> + use<> {
    (0..scene.groups.len() as u32).map(GroupId)
}

/// The links of a scene: edge groups with two known, distinct ends, one per edge key, in order.
pub fn links(scene: &Scene) -> Vec<Link> {
    let mut seen: Vec<u32> = Vec::new();
    ids(scene)
        .filter_map(|group| {
            let g = scene.group(group)?;
            let e = g.edge.as_ref().filter(|_| g.kind == GroupKind::Edge)?;
            let known = scene.node(&e.from).is_some() && scene.node(&e.to).is_some();
            if !known || e.from == e.to || seen.contains(&e.key) {
                return None;
            }
            seen.push(e.key);
            Some(Link { group, key: e.key, from: e.from.clone(), to: e.to.clone() })
        })
        .collect()
}

/// The edges leaving `node`, with the node each one reaches, in authored order.
pub fn outgoing(scene: &Scene, node: &str) -> Vec<(GroupId, String)> {
    links(scene).into_iter().filter(|l| l.from == node).map(|l| (l.group, l.to)).collect()
}

/// The first shortest path from `from` to `to` along the edges' direction. `None` when there is
/// none, or the two are the same node or not nodes at all.
pub fn shortest_path(scene: &Scene, from: &str, to: &str) -> Option<Path> {
    walk(&links(scene), from, to, true)
}

fn walk(links: &[Link], from: &str, to: &str, directed: bool) -> Option<Path> {
    if from == to {
        return None;
    }
    let mut previous: HashMap<&str, (&str, GroupId)> = HashMap::new();
    let mut queue: VecDeque<&str> = VecDeque::from([from]);
    while let Some(at) = queue.pop_front() {
        for l in links {
            let next = if l.from == at {
                l.to.as_str()
            } else if !directed && l.to == at {
                l.from.as_str()
            } else {
                continue;
            };
            if next == from || previous.contains_key(next) {
                continue;
            }
            previous.insert(next, (at, l.group));
            queue.push_back(next);
        }
        if previous.contains_key(to) {
            break;
        }
    }
    previous.contains_key(to).then(|| {
        let (mut nodes, mut edges, mut at) = (vec![to.to_string()], Vec::new(), to);
        while let Some(&(before, edge)) = previous.get(at) {
            nodes.push(before.to_string());
            edges.push(edge);
            at = before;
        }
        nodes.reverse();
        edges.reverse();
        Path { nodes, edges }
    })
}

/// How `a` and `b` relate: the edges between them if any (either direction), else the shortest
/// directed path `a` to `b`, else `b` to `a`, else the shortest path ignoring direction. `None`
/// when they are the same node, unknown, or not connected at all.
pub fn relationship(scene: &Scene, a: &str, b: &str) -> Option<Relationship> {
    let all = links(scene);
    let direct: Vec<GroupId> = all
        .iter()
        .filter(|l| (l.from == a && l.to == b) || (l.from == b && l.to == a))
        .map(|l| l.group)
        .collect();
    if a != b && !direct.is_empty() {
        let path = Path { nodes: vec![a.to_string(), b.to_string()], edges: direct };
        return Some(Relationship { path, direct: true });
    }
    let path = walk(&all, a, b, true)
        .or_else(|| walk(&all, b, a, true))
        .or_else(|| walk(&all, a, b, false))?;
    Some(Relationship { path, direct: false })
}

/// The label groups that follow the edges `edges` (same edge key).
pub fn labels_of(scene: &Scene, edges: &[GroupId]) -> Vec<GroupId> {
    let keys: Vec<u32> =
        edges.iter().filter_map(|e| scene.group(*e)).filter_map(|g| g.edge.as_ref()).map(|e| e.key).collect();
    ids(scene)
        .filter(|id| {
            scene
                .group(*id)
                .is_some_and(|g| g.kind == GroupKind::Label && g.edge.as_ref().is_some_and(|e| keys.contains(&e.key)))
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod fixture {
    use crate::scene::{Bounds, EdgeRef, Group, GroupId, GroupKind, Scene};
    use crate::tokens::Kind;

    /// Nodes a..f (groups 0..6, kinds as given); then one edge group per `(key, from, to)` and a
    /// label group of the same key after each. Edge group of the i-th edge is `6 + 2 i`.
    pub fn scene(nodes: &[(&str, Kind)], edges: &[(u32, &str, &str)]) -> Scene {
        let mut s = Scene::default();
        for (i, (id, kind)) in nodes.iter().enumerate() {
            s.groups.push(Group::node(id, *kind, id, Bounds::new(i as f64 * 20.0, 0.0, 10.0, 10.0)));
        }
        for (key, from, to) in edges {
            let e = EdgeRef { key: *key, from: (*from).into(), to: (*to).into(), id: None };
            s.groups.push(Group::edge(e.clone(), "", &[[0.0, 0.0], [10.0, 0.0]], 4.0));
            let mut l = Group::new(GroupKind::Label, format!("label-{key}"), Bounds::new(0.0, 0.0, 4.0, 4.0));
            l.edge = Some(e);
            s.groups.push(l);
        }
        s
    }

    pub fn id(s: &Scene, node: &str) -> GroupId {
        s.node(node).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::scene;
    use super::*;
    use crate::tokens::Kind;

    const NODES: [(&str, Kind); 6] = [
        ("a", Kind::Frontend),
        ("b", Kind::Backend),
        ("c", Kind::Database),
        ("d", Kind::Backend),
        ("e", Kind::Cloud),
        ("f", Kind::External),
    ];

    /// a>b>c, a>d>c (a longer way round, listed later), c>e, and f apart.
    fn graph() -> Scene {
        scene(&NODES, &[(1, "a", "b"), (2, "b", "c"), (3, "a", "d"), (4, "d", "c"), (5, "c", "e")])
    }

    #[test]
    fn the_shortest_directed_path_is_the_first_of_its_length() {
        let s = graph();
        let p = shortest_path(&s, "a", "c").unwrap();
        assert_eq!(p.nodes, ["a", "b", "c"], "a>b>c is listed before a>d>c");
        assert_eq!(p.edges, [GroupId(6), GroupId(8)]);
        assert_eq!(shortest_path(&s, "a", "e").unwrap().nodes, ["a", "b", "c", "e"]);
    }

    #[test]
    fn a_path_runs_with_the_edges_never_against_them() {
        let s = graph();
        assert_eq!(shortest_path(&s, "e", "a"), None);
        assert_eq!(shortest_path(&s, "a", "f"), None, "f is not connected");
        assert_eq!(shortest_path(&s, "a", "a"), None, "a node has no route to itself");
        assert_eq!(shortest_path(&s, "a", "zz"), None);
    }

    #[test]
    fn a_cycle_does_not_loop_the_search() {
        let s = scene(&NODES, &[(1, "a", "b"), (2, "b", "a"), (3, "b", "c")]);
        assert_eq!(shortest_path(&s, "a", "c").unwrap().nodes, ["a", "b", "c"]);
    }

    #[test]
    fn links_skip_self_loops_unknown_ends_and_repeated_keys() {
        let s = scene(&NODES, &[(1, "a", "a"), (2, "a", "zz"), (3, "a", "b"), (3, "b", "c")]);
        let l = links(&s);
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].from.as_str(), l[0].to.as_str()), ("a", "b"));
        assert_eq!(outgoing(&graph(), "a").iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), ["b", "d"]);
        assert!(outgoing(&graph(), "e").is_empty());
    }

    #[test]
    fn a_relationship_prefers_the_direct_edges_in_either_direction() {
        let s = scene(&NODES, &[(1, "a", "b"), (2, "b", "a"), (3, "b", "c")]);
        let r = relationship(&s, "a", "b").unwrap();
        assert!(r.direct);
        assert_eq!(r.path.edges, [GroupId(6), GroupId(8)], "both parallel edges");
        let r = relationship(&s, "b", "a").unwrap();
        assert!(r.direct && r.path.nodes == ["b", "a"]);
    }

    #[test]
    fn a_relationship_falls_back_to_a_directed_path_then_the_reverse_then_any() {
        let s = graph();
        let r = relationship(&s, "a", "e").unwrap();
        assert!(!r.direct);
        assert_eq!(r.path.nodes, ["a", "b", "c", "e"]);
        let r = relationship(&s, "e", "a").unwrap();
        assert_eq!(r.path.nodes, ["a", "b", "c", "e"], "the way a feeds e, found from the other side");
        // b and d only meet through c: neither reaches the other, but they are connected.
        let r = relationship(&s, "b", "d").unwrap();
        assert!(!r.direct);
        assert_eq!(r.path.nodes.len(), r.path.edges.len() + 1);
        assert_eq!((r.path.nodes[0].as_str(), r.path.nodes.last().unwrap().as_str()), ("b", "d"));
        assert_eq!(relationship(&s, "a", "f"), None);
        assert_eq!(relationship(&s, "a", "a"), None);
    }

    #[test]
    fn labels_follow_their_edges() {
        let s = graph();
        assert_eq!(labels_of(&s, &[GroupId(6)]), [GroupId(7)]);
        assert!(labels_of(&s, &[]).is_empty());
    }
}
