//! Ambient `--step` per edge and node (`04` §1.3, `00` §6.3): the delay of the one-shot trace is
//! `min(12, step) * 160 ms`, so the whole pass is bounded (last start 1.92 s, ends <= 5.52 s).
//!
//! - **A** (architecture): an edge's step is its connection index. A node's step is the index of
//!   the first connection naming it `from`, else the first naming it `to` plus one, else its own
//!   index (`render-architecture.mjs:97-105`).
//! - **W** (workflow): the `mainPath` position (`workflow-compiler.mjs:978-997`).
//!
//! The rules take plain id lists so any model can feed them; [`scene_steps`] derives rule A from a
//! [`Scene`] alone, which is what the other types (D, S, L) use too.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::scene::{GroupKind, Scene};

/// `min(12, step)`: the cap that keeps the pass inside the 6 s capture budget.
pub const MAX_STEP: u32 = 12;

/// Per-step delay of the ambient trace.
pub const STEP_DELAY_MS: f64 = 160.0;

/// Ambient steps. `edges[i]` is the step of the `i`-th edge group in scene order; nodes by id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Steps {
    pub edges: Vec<u32>,
    pub nodes: BTreeMap<String, u32>,
}

impl Steps {
    pub fn edge(&self, ordinal: usize) -> u32 {
        self.edges.get(ordinal).copied().unwrap_or(0).min(MAX_STEP)
    }

    pub fn node(&self, id: &str) -> u32 {
        self.nodes.get(id).copied().unwrap_or(0).min(MAX_STEP)
    }

    /// `min(12, step) * 160 ms`.
    pub fn edge_delay_ms(&self, ordinal: usize) -> f64 {
        f64::from(self.edge(ordinal)) * STEP_DELAY_MS
    }

    pub fn node_delay_ms(&self, id: &str) -> f64 {
        f64::from(self.node(id)) * STEP_DELAY_MS
    }
}

/// Rule A over `connections` as `(from, to)` and `nodes` in authored order.
pub fn architecture_steps(connections: &[(&str, &str)], nodes: &[&str]) -> Steps {
    let mut node_step: BTreeMap<String, u32> = BTreeMap::new();
    for (i, (from, to)) in connections.iter().enumerate() {
        node_step.entry((*from).to_string()).or_insert(i as u32);
        node_step.entry((*to).to_string()).or_insert(i as u32 + 1);
    }
    for (i, id) in nodes.iter().enumerate() {
        node_step.entry((*id).to_string()).or_insert(i as u32);
    }
    Steps { edges: (0..connections.len() as u32).collect(), nodes: node_step }
}

/// Rule W: `main_path` ids, `edges` as `(from, to)`, `nodes` in authored order. An edge sits at
/// the position of its source when it steps along the path (`to == from + 1`), otherwise at
/// `len(main_path) + index`; a node off the path at `len(main_path) + its index`.
pub fn workflow_steps(main_path: &[&str], edges: &[(&str, &str)], nodes: &[&str]) -> Steps {
    let pos = |id: &str| main_path.iter().position(|p| *p == id);
    let n = main_path.len() as u32;
    let edge_steps = edges
        .iter()
        .enumerate()
        .map(|(i, (from, to))| match (pos(from), pos(to)) {
            (Some(f), Some(t)) if t == f + 1 => f as u32,
            _ => n + i as u32,
        })
        .collect();
    let node_steps = nodes
        .iter()
        .enumerate()
        .map(|(i, id)| ((*id).to_string(), pos(id).map_or(n + i as u32, |p| p as u32)))
        .collect();
    Steps { edges: edge_steps, nodes: node_steps }
}

/// Rule A from a scene: edge groups in group order are the connections, node groups in group
/// order the components.
pub fn scene_steps(scene: &Scene) -> Steps {
    let mut connections: Vec<(&str, &str)> = Vec::new();
    let mut nodes: Vec<&str> = Vec::new();
    for g in &scene.groups {
        match g.kind {
            GroupKind::Edge => {
                if let Some(e) = &g.edge {
                    connections.push((e.from.as_str(), e.to.as_str()));
                }
            }
            GroupKind::Node => nodes.extend(g.node_id.as_deref()),
            _ => {}
        }
    }
    architecture_steps(&connections, &nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_rule_first_from_then_first_to_plus_one_then_own_index() {
        let s = architecture_steps(&[("a", "b"), ("b", "c"), ("a", "c")], &["a", "b", "c", "lonely"]);
        assert_eq!(s.edges, vec![0, 1, 2]);
        assert_eq!(s.node("a"), 0);
        assert_eq!(s.node("b"), 1, "first seen as `to` of edge 0: 0 + 1; a later `from` never overrides");
        assert_eq!(s.node("c"), 2, "first seen as `to` of edge 1: 1 + 1");
        assert_eq!(s.node("lonely"), 3);
    }

    #[test]
    fn workflow_rule_uses_the_main_path_position() {
        let s = workflow_steps(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("a", "c"), ("c", "x")], &["x", "a", "b", "c"]);
        assert_eq!(s.edges, vec![0, 1, 3 + 2, 3 + 3], "path hops are the source position, the rest follow the path");
        assert_eq!((s.node("a"), s.node("b"), s.node("c")), (0, 1, 2));
        assert_eq!(s.node("x"), 3);
    }

    #[test]
    fn steps_are_capped_at_twelve_so_the_pass_is_bounded() {
        let s = Steps { edges: vec![0, 12, 13, 400], ..Steps::default() };
        assert_eq!(s.edge(3), MAX_STEP);
        assert_eq!(s.edge_delay_ms(3), 1920.0, "last start 12 x 160 ms");
        assert_eq!(s.edge(99), 0, "unknown ordinal");
        assert_eq!(s.node_delay_ms("nope"), 0.0);
    }
}
