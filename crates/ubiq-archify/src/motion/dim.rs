//! Fades: M12 dim/focus (180 ms ease) and M11 LOD detail (160 ms ease) (`00` §6.2, `04` §1.1).
//!
//! A fade is a multiplier on a group's alpha, never a restyle: the overlay's `group_alpha` carries
//! it and the painter multiplies. Under reduced motion (or a hidden window) the fade is instant.

use serde::{Deserialize, Serialize};

use super::overlay::DetailAlpha;
use super::timeline::{Easing, Ms, lerp};
use crate::scene::{Detail, GroupId, GroupKind, Scene, Tier};

/// M12: opacity transition on nodes and edges.
pub const DIM_FADE_MS: f64 = 180.0;
/// M11: `[data-detail]` opacity.
pub const LOD_FADE_MS: f64 = 160.0;

/// Dim targets (`00` §6.3 `Dim`, `focus.js`): the alpha of everything not lit.
pub const DIM_INTENT: f64 = 0.2;
pub const DIM_FOCUS: f64 = 0.13;
pub const DIM_LENS: f64 = 0.11;
/// A lens peer (a node of a related kind) stays at .62.
pub const DIM_LENS_PEER: f64 = 0.62;
pub const DIM_ROUTE: f64 = 0.11;
/// A relationship shown between two picked nodes (`relationship-direct-active`, `viewer.css:4228`).
pub const DIM_RELATIONSHIP: f64 = 0.16;
pub const DIM_ROUTE_PAST: f64 = 0.62;
pub const DIM_ROUTE_FUTURE: f64 = 0.34;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DimKind {
    Intent,
    Focus,
    Lens,
    Route,
    Relationship,
}

impl DimKind {
    pub fn rest(self) -> f64 {
        match self {
            DimKind::Intent => DIM_INTENT,
            DimKind::Focus => DIM_FOCUS,
            DimKind::Lens => DIM_LENS,
            DimKind::Route => DIM_ROUTE,
            DimKind::Relationship => DIM_RELATIONSHIP,
        }
    }
}

/// Only nodes, edges and edge labels dim (`[data-node-id]`, `[data-edge-from]`).
pub fn dimmable(kind: GroupKind) -> bool {
    matches!(kind, GroupKind::Node | GroupKind::Edge | GroupKind::Label)
}

/// One dim: everything dimmable that is not in `lit` sits at `rest`; a lit group at its own alpha
/// (1 for a neighbourhood, .62/.34 for route past/future, .62 for lens peers).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dim {
    pub kind: DimKind,
    pub rest: f64,
    /// Sorted by group id.
    pub lit: Vec<(GroupId, f64)>,
    pub since_ms: Ms,
    /// `(when, depth at that moment)` once released.
    pub released: Option<(Ms, f64)>,
    /// 0 when instant.
    pub fade_ms: f64,
}

impl Dim {
    pub fn new(kind: DimKind, rest: f64, mut lit: Vec<(GroupId, f64)>, since_ms: Ms, instant: bool) -> Self {
        lit.sort_by_key(|(g, _)| *g);
        lit.dedup_by_key(|(g, _)| *g);
        Self {
            kind,
            rest,
            lit,
            since_ms,
            released: None,
            fade_ms: if instant { 0.0 } else { DIM_FADE_MS },
        }
    }

    /// The neighbourhood of a node: the node, its neighbours, their incident edges and those
    /// edges' labels stay at 1 (`focus.js`, `intent-trace.js`). `None` for an unknown node.
    pub fn neighbourhood(kind: DimKind, scene: &Scene, node_id: &str, since_ms: Ms, instant: bool) -> Option<Dim> {
        let me = scene.node(node_id)?;
        let mut lit = vec![(me, 1.0)];
        let mut incident: Vec<u32> = Vec::new();
        for (edge, other) in scene.edges_of(node_id) {
            lit.push((edge, 1.0));
            if let Some(n) = scene.node(other) {
                lit.push((n, 1.0));
            }
            incident.extend(scene.group(edge).and_then(|g| g.edge.as_ref()).map(|e| e.key));
        }
        for (i, g) in scene.groups.iter().enumerate() {
            if g.kind == GroupKind::Label && g.edge.as_ref().is_some_and(|e| incident.contains(&e.key)) {
                lit.push((GroupId(i as u32), 1.0));
            }
        }
        Some(Dim::new(kind, kind.rest(), lit, since_ms, instant))
    }

    /// Start fading back to 1. A second release keeps the first.
    pub fn release(&mut self, t: Ms) {
        if self.released.is_none() {
            // A release mid fade-in starts from where the fade got to, not from the target.
            self.released = Some((t, self.depth(t)));
        }
    }

    /// How far the dim has taken effect, in `[0, 1]`.
    pub fn depth(&self, t: Ms) -> f64 {
        let run = |from: Ms| {
            if self.fade_ms <= 0.0 { 1.0 } else { Easing::Ease.apply((t - from) / self.fade_ms) }
        };
        match self.released {
            None => run(self.since_ms),
            Some((r, from)) => from * (1.0 - run(r)),
        }
    }

    /// The multiplier for a group: `lerp(1, target, depth)`; non-dimmable kinds stay at 1.
    pub fn alpha(&self, scene: &Scene, group: GroupId, t: Ms) -> f64 {
        let Some(g) = scene.group(group) else { return 1.0 };
        if !dimmable(g.kind) {
            return 1.0;
        }
        let target = self
            .lit
            .binary_search_by_key(&group, |(id, _)| *id)
            .map_or(self.rest, |i| self.lit[i].1);
        lerp(1.0, target, self.depth(t))
    }

    /// Still fading in or out.
    pub fn is_active(&self, t: Ms) -> bool {
        match self.released {
            None => t < self.since_ms + self.fade_ms,
            Some((r, _)) => t < r + self.fade_ms,
        }
    }

    /// Fully faded back to 1: drop it.
    pub fn is_gone(&self, t: Ms) -> bool {
        self.released.is_some_and(|(r, _)| t >= r + self.fade_ms)
    }
}

/// A reading-depth fade after a tier change.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LodFade {
    pub from: DetailAlpha,
    pub to: Tier,
    pub since_ms: Ms,
    pub fade_ms: f64,
}

/// The visibility of the two fade-able depths at a tier.
pub fn tier_alpha(tier: Tier) -> DetailAlpha {
    let v = |d: Detail| if d.visible(tier) { 1.0 } else { 0.0 };
    DetailAlpha { context: v(Detail::Context), fine: v(Detail::Fine) }
}

impl LodFade {
    /// A fade from whatever is visible at `t` (`current`) to `to`.
    pub fn new(current: DetailAlpha, to: Tier, since_ms: Ms, instant: bool) -> Self {
        Self { from: current, to, since_ms, fade_ms: if instant { 0.0 } else { LOD_FADE_MS } }
    }

    pub fn at(&self, t: Ms) -> DetailAlpha {
        let k = if self.fade_ms <= 0.0 { 1.0 } else { Easing::Ease.apply((t - self.since_ms) / self.fade_ms) };
        let to = tier_alpha(self.to);
        DetailAlpha { context: lerp(self.from.context, to.context, k), fine: lerp(self.from.fine, to.fine, k) }
    }

    pub fn is_active(&self, t: Ms) -> bool {
        t < self.since_ms + self.fade_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Bounds, EdgeRef, Group};
    use crate::tokens::Kind;

    fn scene() -> Scene {
        let mut s = Scene::default();
        let b = Bounds::new(0.0, 0.0, 10.0, 10.0);
        for id in ["a", "b", "c"] {
            s.groups.push(Group::node(id, Kind::Backend, id, b));
        }
        let e = |key, from: &str, to: &str| EdgeRef { key, from: from.into(), to: to.into(), id: None };
        s.groups.push(Group::edge(e(0, "a", "b"), "", &[[0.0, 0.0], [5.0, 0.0]], 4.0));
        s.groups.push(Group::edge(e(1, "b", "c"), "", &[[0.0, 0.0], [5.0, 0.0]], 4.0));
        let mut l = Group::new(GroupKind::Label, "label-1", b);
        l.edge = Some(e(1, "b", "c"));
        s.groups.push(l);
        s.groups.push(Group::new(GroupKind::Frame, "frame-x", b));
        s
    }

    #[test]
    fn intent_dims_the_rest_to_point_two_and_keeps_the_neighbourhood() {
        let s = scene();
        let d = Dim::neighbourhood(DimKind::Intent, &s, "a", 0.0, false).unwrap();
        let end = 180.0;
        let at = |i: u32| d.alpha(&s, GroupId(i), end);
        assert_eq!((at(0), at(1)), (1.0, 1.0), "a and its neighbour b");
        assert!((at(2) - DIM_INTENT).abs() < 1e-9, "c is outside the neighbourhood");
        assert_eq!(at(3), 1.0, "edge a-b is incident");
        assert!((at(4) - DIM_INTENT).abs() < 1e-9, "edge b-c is not");
        assert!((at(5) - DIM_INTENT).abs() < 1e-9, "its label dims with it");
        assert_eq!(at(6), 1.0, "frames never dim");
        assert!(Dim::neighbourhood(DimKind::Intent, &s, "zzz", 0.0, false).is_none());
    }

    #[test]
    fn the_fade_takes_180_ms_and_reverses_on_release() {
        let s = scene();
        let mut d = Dim::neighbourhood(DimKind::Focus, &s, "a", 1000.0, false).unwrap();
        assert_eq!(d.alpha(&s, GroupId(2), 1000.0), 1.0);
        let half = d.alpha(&s, GroupId(2), 1090.0);
        assert!(half < 1.0 && half > DIM_FOCUS);
        assert!((d.alpha(&s, GroupId(2), 1180.0) - DIM_FOCUS).abs() < 1e-9);
        assert!(d.is_active(1179.0) && !d.is_active(1180.0));
        d.release(2000.0);
        assert!(d.is_active(2179.0) && !d.is_gone(2179.0));
        assert!(d.is_gone(2180.0));
        assert_eq!(d.alpha(&s, GroupId(2), 2180.0), 1.0, "back to the static scene");
    }

    #[test]
    fn a_release_mid_fade_starts_from_where_it_got_to() {
        let s = scene();
        let mut d = Dim::neighbourhood(DimKind::Intent, &s, "a", 0.0, false).unwrap();
        let mid = d.alpha(&s, GroupId(2), 60.0);
        d.release(60.0);
        let just_after = d.alpha(&s, GroupId(2), 60.0);
        assert!((mid - just_after).abs() < 0.02, "no jump: {mid} vs {just_after}");
        assert!(d.is_gone(60.0 + 180.0 + 1.0));
    }

    #[test]
    fn instant_dims_apply_at_once() {
        let s = scene();
        let d = Dim::neighbourhood(DimKind::Intent, &s, "a", 5.0, true).unwrap();
        assert!((d.alpha(&s, GroupId(2), 5.0) - DIM_INTENT).abs() < 1e-9);
        assert!(!d.is_active(5.0));
    }

    #[test]
    fn lod_fade_crossfades_context_and_fine_over_160_ms() {
        let f = LodFade::new(tier_alpha(Tier::Map), Tier::Read, 0.0, false);
        assert_eq!(f.at(0.0), DetailAlpha { context: 0.0, fine: 0.0 });
        let m = f.at(80.0);
        assert!(m.context > 0.0 && m.context < 1.0 && m.fine == 0.0);
        assert_eq!(f.at(160.0), DetailAlpha { context: 1.0, fine: 0.0 });
        assert!(f.is_active(159.0) && !f.is_active(160.0));
        assert_eq!(tier_alpha(Tier::Full), DetailAlpha { context: 1.0, fine: 1.0 });
    }
}
