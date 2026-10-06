//! The semantic lens (M6, `V/semantic-lens.js:405-470`): pick a node kind (or an authored tag) and
//! every relationship touching it stays lit; pick two and only the direct relationships between
//! them do, the rest of the diagram kept as a dimmed spatial reference. Pure selection sets: the
//! state turns them into a dim and comets (`interact.rs`).
//!
//! More than [`LENS_MAX_EDGES`] matched edges is "quiet density": the dim stays, no comets run.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::comet::{Direction, LENS_MAX_EDGES};
use super::dim::DIM_LENS_PEER;
use super::path::{Link, ids, labels_of, links};
use crate::scene::{Group, GroupId, GroupKind, Scene};
use crate::tokens::Kind;

/// At most this many filters are combined (the JS: two kinds).
pub const MAX_FILTERS: usize = 2;

/// What a lens selects nodes by.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LensFilter {
    Kind(Kind),
    Tag(String),
}

impl LensFilter {
    pub fn label(&self) -> String {
        match self {
            LensFilter::Kind(k) => k.as_str().to_string(),
            LensFilter::Tag(t) => format!("#{t}"),
        }
    }

    /// Whether a node group is selected by it.
    pub fn matches(&self, g: &Group) -> bool {
        g.kind == GroupKind::Node
            && match self {
                LensFilter::Kind(k) => g.node_kind == Some(*k),
                LensFilter::Tag(t) => g.tags.iter().any(|x| x == t),
            }
    }
}

/// One choice of the lens control: a filter and how many nodes it selects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LensOption {
    pub filter: LensFilter,
    pub nodes: usize,
}

/// The filters a scene offers: its node kinds, then its tags, each most-populated first (ties by
/// label). Kinds and tags with no node are not offered.
pub fn options(scene: &Scene) -> Vec<LensOption> {
    let mut kinds: BTreeMap<Kind, usize> = BTreeMap::new();
    let mut tags: BTreeMap<&str, usize> = BTreeMap::new();
    for g in scene.groups.iter().filter(|g| g.kind == GroupKind::Node) {
        if let Some(k) = g.node_kind {
            *kinds.entry(k).or_default() += 1;
        }
        for t in &g.tags {
            *tags.entry(t.as_str()).or_default() += 1;
        }
    }
    let mut out: Vec<LensOption> = kinds
        .into_iter()
        .map(|(k, nodes)| LensOption { filter: LensFilter::Kind(k), nodes })
        .collect();
    out.sort_by(|a, b| b.nodes.cmp(&a.nodes).then_with(|| a.filter.label().cmp(&b.filter.label())));
    let mut tagged: Vec<LensOption> =
        tags.into_iter().map(|(t, nodes)| LensOption { filter: LensFilter::Tag(t.to_string()), nodes }).collect();
    tagged.sort_by(|a, b| b.nodes.cmp(&a.nodes).then_with(|| a.filter.label().cmp(&b.filter.label())));
    out.extend(tagged);
    out
}

/// The next filter in the cycle `none, o0, o1, ...`, back to none after the last. A `current` that
/// is not on offer (an edit removed it) starts the cycle again.
pub fn cycle(options: &[LensOption], current: Option<&LensFilter>) -> Option<LensFilter> {
    let at = current.and_then(|c| options.iter().position(|o| &o.filter == c));
    match (current, at) {
        (None, _) | (Some(_), None) => options.first().map(|o| o.filter.clone()),
        (Some(_), Some(i)) => options.get(i + 1).map(|o| o.filter.clone()),
    }
}

/// What a lens lights.
#[derive(Clone, Debug, PartialEq)]
pub struct LensSelection {
    pub filters: Vec<LensFilter>,
    /// The nodes the filters select, in scene order.
    pub nodes: Vec<GroupId>,
    /// Nodes at the other end of a matched edge that no filter selects (single filter only).
    pub peers: Vec<GroupId>,
    /// The matched edges with how they run relative to the selection: `Out` leaves it (or goes
    /// from the first filter to the second), `In` enters it (second to first), `Within` joins two
    /// selected nodes.
    pub edges: Vec<(GroupId, Direction)>,
    /// The dim's lit set: selected nodes and matched edges (and their labels) at 1, peers at .62.
    pub lit: Vec<(GroupId, f64)>,
    /// Too many edges to animate: the dim holds, the comets do not run.
    pub quiet: bool,
}

impl LensSelection {
    /// The comets that run: none when quiet.
    pub fn flow(&self) -> &[(GroupId, Direction)] {
        if self.quiet { &[] } else { &self.edges }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Select by `filters` (the first [`MAX_FILTERS`] count; none selects nothing).
pub fn select(scene: &Scene, filters: &[LensFilter]) -> LensSelection {
    let filters: Vec<LensFilter> = filters.iter().take(MAX_FILTERS).cloned().collect();
    let mut sel = LensSelection {
        filters,
        nodes: Vec::new(),
        peers: Vec::new(),
        edges: Vec::new(),
        lit: Vec::new(),
        quiet: false,
    };
    if sel.filters.is_empty() {
        return sel;
    }
    let chosen = |g: &Group| sel.filters.iter().any(|f| f.matches(g));
    let chosen_ids: Vec<(GroupId, String)> = ids(scene)
        .filter_map(|id| {
            let g = scene.group(id)?;
            chosen(g).then(|| g.node_id.clone().map(|n| (id, n))).flatten()
        })
        .collect();
    sel.nodes = chosen_ids.iter().map(|(id, _)| *id).collect();
    let is_chosen = |node: &str| chosen_ids.iter().any(|(_, n)| n == node);
    let on = |node: &str, f: &LensFilter| scene.node(node).and_then(|id| scene.group(id)).is_some_and(|g| f.matches(g));

    let cross = sel.filters.len() == 2;
    let mut peers: Vec<GroupId> = Vec::new();
    for Link { group, from, to, .. } in links(scene) {
        let direction = if cross {
            if on(&from, &sel.filters[0]) && on(&to, &sel.filters[1]) {
                Direction::Out
            } else if on(&from, &sel.filters[1]) && on(&to, &sel.filters[0]) {
                Direction::In
            } else {
                continue;
            }
        } else {
            match (is_chosen(&from), is_chosen(&to)) {
                (true, true) => Direction::Within,
                (true, false) => Direction::Out,
                (false, true) => Direction::In,
                (false, false) => continue,
            }
        };
        sel.edges.push((group, direction));
        if !cross {
            for end in [&from, &to].into_iter().filter(|n| !is_chosen(n)) {
                if let Some(id) = scene.node(end)
                    && !peers.contains(&id)
                {
                    peers.push(id);
                }
            }
        }
    }
    sel.peers = peers;
    sel.quiet = sel.edges.len() > LENS_MAX_EDGES;

    let edge_ids: Vec<GroupId> = sel.edges.iter().map(|(g, _)| *g).collect();
    sel.lit = sel.nodes.iter().map(|g| (*g, 1.0)).collect();
    sel.lit.extend(edge_ids.iter().map(|g| (*g, 1.0)));
    sel.lit.extend(labels_of(scene, &edge_ids).into_iter().map(|g| (g, 1.0)));
    sel.lit.extend(sel.peers.iter().map(|g| (*g, DIM_LENS_PEER)));
    sel
}

#[cfg(test)]
mod tests {
    use super::super::path::fixture::{id, scene};
    use super::*;

    const NODES: [(&str, Kind); 5] = [
        ("web", Kind::Frontend),
        ("api", Kind::Backend),
        ("jobs", Kind::Backend),
        ("db", Kind::Database),
        ("lone", Kind::Cloud),
    ];

    /// web>api, api>db, jobs>db, db>api (back), api>jobs.
    fn graph() -> Scene {
        scene(
            &NODES,
            &[(1, "web", "api"), (2, "api", "db"), (3, "jobs", "db"), (4, "db", "api"), (5, "api", "jobs")],
        )
    }

    fn kind(k: Kind) -> LensFilter {
        LensFilter::Kind(k)
    }

    #[test]
    fn one_kind_lights_its_nodes_every_edge_touching_them_and_the_peers() {
        let s = graph();
        let sel = select(&s, &[kind(Kind::Database)]);
        assert_eq!(sel.nodes, [id(&s, "db")]);
        assert_eq!(sel.edges.len(), 3, "api>db, jobs>db, db>api");
        let dir = |i: usize| sel.edges[i].1;
        assert_eq!((dir(0), dir(1), dir(2)), (Direction::In, Direction::In, Direction::Out));
        assert_eq!(sel.peers, [id(&s, "api"), id(&s, "jobs")]);
        let lit = |g: GroupId| sel.lit.iter().find(|(x, _)| *x == g).map(|(_, a)| *a);
        assert_eq!(lit(id(&s, "db")), Some(1.0));
        assert_eq!(lit(id(&s, "api")), Some(DIM_LENS_PEER));
        assert_eq!(lit(id(&s, "web")), None, "web is outside the lens and dims");
        assert_eq!(lit(GroupId(7)), Some(1.0), "api>db, a matched edge");
        assert_eq!(lit(GroupId(8)), Some(1.0), "and its label");
        assert_eq!(lit(GroupId(13)), None, "api>jobs does not touch a database");
    }

    #[test]
    fn an_edge_between_two_chosen_nodes_runs_within() {
        let s = graph();
        let sel = select(&s, &[kind(Kind::Backend)]);
        let within: Vec<_> = sel.edges.iter().filter(|(_, d)| *d == Direction::Within).collect();
        assert_eq!(within.len(), 1, "api>jobs joins two backends");
        assert!(sel.peers.contains(&id(&s, "web")) && sel.peers.contains(&id(&s, "db")));
    }

    #[test]
    fn two_kinds_compare_only_the_direct_relationships_between_them() {
        let s = graph();
        let sel = select(&s, &[kind(Kind::Backend), kind(Kind::Database)]);
        let runs: Vec<_> = sel.edges.iter().map(|(_, d)| *d).collect();
        assert_eq!(runs, [Direction::Out, Direction::Out, Direction::In], "api>db, jobs>db forward; db>api back");
        assert!(sel.peers.is_empty(), "a comparison keeps the rest as a dim reference");
        assert_eq!(sel.nodes.len(), 3);
        let none = select(&s, &[kind(Kind::Frontend), kind(Kind::Database)]);
        assert!(none.edges.is_empty() && !none.is_empty(), "no direct relationship, the nodes still light");
    }

    #[test]
    fn a_third_filter_is_ignored() {
        let s = graph();
        let two = select(&s, &[kind(Kind::Backend), kind(Kind::Database)]);
        let three = select(&s, &[kind(Kind::Backend), kind(Kind::Database), kind(Kind::Cloud)]);
        assert_eq!(two, three);
    }

    #[test]
    fn nothing_selected_lights_nothing() {
        let s = graph();
        assert!(select(&s, &[]).is_empty());
        let sel = select(&s, &[kind(Kind::External)]);
        assert!(sel.is_empty() && sel.edges.is_empty() && sel.lit.is_empty());
    }

    #[test]
    fn tags_select_like_kinds() {
        let mut s = graph();
        for g in s.groups.iter_mut().filter(|g| matches!(g.node_id.as_deref(), Some("web" | "api"))) {
            g.tags.push("edge-tier".into());
        }
        let sel = select(&s, &[LensFilter::Tag("edge-tier".into())]);
        assert_eq!(sel.nodes, [id(&s, "web"), id(&s, "api")]);
        let opts = options(&s);
        assert_eq!(opts.last().unwrap(), &LensOption { filter: LensFilter::Tag("edge-tier".into()), nodes: 2 });
        assert_eq!(LensFilter::Tag("x".into()).label(), "#x");
    }

    #[test]
    fn more_than_24_edges_is_quiet_density_and_runs_no_comets() {
        let edges: Vec<(u32, &str, &str)> = (0..30).map(|i| (i, "web", "api")).collect();
        let s = scene(&NODES, &edges);
        let sel = select(&s, &[kind(Kind::Frontend)]);
        assert_eq!(sel.edges.len(), 30);
        assert!(sel.quiet && sel.flow().is_empty());
        assert_eq!(sel.nodes.len(), 1, "the dim still shows the selection");
        let edges: Vec<(u32, &str, &str)> = (0..24).map(|i| (i, "web", "api")).collect();
        let sel = select(&scene(&NODES, &edges), &[kind(Kind::Frontend)]);
        assert!(!sel.quiet && sel.flow().len() == 24, "exactly the cap still runs");
    }

    #[test]
    fn options_list_kinds_most_populated_first() {
        let s = graph();
        let o = options(&s);
        let kinds: Vec<_> = o.iter().map(|x| (x.filter.label(), x.nodes)).collect();
        assert_eq!(kinds[0], ("backend".to_string(), 2));
        assert_eq!(kinds.len(), 4);
        assert_eq!(kinds[1].0, "cloud", "ties by label");
    }

    #[test]
    fn the_cycle_walks_the_options_and_ends_on_none() {
        let s = graph();
        let o = options(&s);
        let first = cycle(&o, None).unwrap();
        assert_eq!(first, o[0].filter);
        let mut at = Some(first);
        let mut seen = 0;
        while let Some(f) = at {
            seen += 1;
            at = cycle(&o, Some(&f));
        }
        assert_eq!(seen, o.len(), "every option once, then off");
        assert_eq!(cycle(&o, Some(&LensFilter::Tag("gone".into()))), Some(o[0].filter.clone()));
        assert_eq!(cycle(&[], None), None);
    }
}
