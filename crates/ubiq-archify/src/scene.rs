//! The scene graph the UI paints (P2.3). `00` §5.3-5.4, `architecture.md` §3.
//!
//! # Frozen for the P2.6 painter
//!
//! A [`Scene`] is a `view_box`, a flat, already z-ordered `Vec<Item>` and a `Vec<Group>`. The
//! painter walks `items` front to back and never sorts; layout builds with [`SceneBuilder`], whose
//! `build` does the one stable sort by [`Layer`]. Everything is serde (the `scene` CLI dump, goldens).
//!
//! | Type | Role |
//! |---|---|
//! | [`Scene`] `{view_box, items, groups}` | the whole picture; `hit_test`, `group`, `items_of`, `edges_of`, `tooltip` |
//! | [`Item`] `{layer, group, shape}` | one painted primitive and the [`GroupId`] it belongs to |
//! | [`Shape`] | `Rect(`[`RectShape`]`)`, `Polyline(`[`PolylineShape`]`)`, `Text(`[`TextShape`]`)`, `Path(`[`PathShape`]`)` |
//! | [`Layer`] | z-order: Background, Frames, Edges, Overlay, Nodes, EdgeLabels, FrameTitles, Legend |
//! | [`Stroke`] `{token, width, dash}`, [`Fill`] `{token, alpha}` | styling; [`Token`] comes from `tokens.rs` |
//! | [`Bounds`] `{x, y, width, height}` | a rect with serde (`From<geom::Rect>`) |
//! | [`Cmd`] | `M L Q C Z` path commands (sigils, brand marks), `From<geom::PathCmd>` |
//! | [`Arrow`] `{tip, dir, sw}` | the arrowhead, derived from a polyline: `PolylineShape::arrow` |
//! | [`Group`] `{kind, id, node_id, node_kind, edge, label, sublabel, context, tags, bounds, hit}` | semantics for hit-testing, tooltips, focus/dim and the `render` summary; never painted |
//! | [`GroupKind`], [`GroupId`], [`EdgeRef`], [`Hit`] | group parts |
//! | [`Anchor`], [`Detail`], [`Tier`] | text alignment, reading-depth class (D12) and the tier from zoom / fit |
//! | [`SceneBuilder`] | `group`, `push`, `push_in`, `build` |
//!
//! Units are viewBox units (vu), except halos and comets, which the painter draws in screen px
//! (`04` §6.4). Points are `[x, y]` ([`Pt`]). The origin is `(0, 0)`, as Archify always emits.
//!
//! ## Decisions worth knowing
//!
//! - The crossover halo is **not** a layer. It is a flag on the edge's polyline: the painter strokes
//!   `halo_width()` in `Token::Mask` with round caps and joins, then the edge. It has to sit directly
//!   under *its own* edge in paint order (it bridges the edges painted before it), which a layer
//!   below all edges would lose.
//! - The opaque `c-mask` under a node is an ordinary [`RectShape`] with `Token::Mask` fill and no
//!   stroke, pushed first in the node's group. Edge labels and frame titles do the same.
//! - A polyline keeps the un-rounded canonical points (`data-composition-points`) and the corner
//!   radius; `cmds()`/`flattened()` give the rounded outline (the same corners as `roundedPath`).

use serde::{Deserialize, Serialize};

use crate::geom::{self, Pt, Rect};
use crate::tokens::{Kind, Preset, Token};

/// A rect with serde. `geom::Rect` is the computation type; this is the scene's.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.height)
    }

    /// Inside the rect grown by `slop` on every side.
    pub fn contains(&self, p: Pt, slop: f64) -> bool {
        p[0] >= self.x - slop
            && p[0] <= self.x + self.width + slop
            && p[1] >= self.y - slop
            && p[1] <= self.y + self.height + slop
    }
}

impl From<Rect> for Bounds {
    fn from(r: Rect) -> Self {
        Bounds::new(r.x, r.y, r.width, r.height)
    }
}

impl From<Bounds> for Rect {
    fn from(b: Bounds) -> Self {
        b.rect()
    }
}

/// A path command. Superset of `geom::PathCmd` (sigils need cubics and close).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Cmd {
    M(Pt),
    L(Pt),
    /// Control, end.
    Q(Pt, Pt),
    /// Control 1, control 2, end.
    C(Pt, Pt, Pt),
    Z,
}

impl From<geom::PathCmd> for Cmd {
    fn from(c: geom::PathCmd) -> Self {
        match c {
            geom::PathCmd::M(p) => Cmd::M(p),
            geom::PathCmd::L(p) => Cmd::L(p),
            geom::PathCmd::Q(c, e) => Cmd::Q(c, e),
        }
    }
}

/// A stroke. `dash` empty is solid. Width is in vu unless the shape says `non_scaling`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub token: Token,
    pub width: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dash: Vec<f64>,
}

impl Stroke {
    pub fn solid(token: Token, width: f64) -> Self {
        Self { token, width, dash: Vec::new() }
    }

    pub fn dashed(token: Token, width: f64, dash: &[f64]) -> Self {
        Self { token, width, dash: dash.to_vec() }
    }
}

/// A fill: token plus an extra alpha multiplier (the translucency of kind fills is already in the
/// token's value, so this is `1.0` for them).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub token: Token,
    pub alpha: f64,
}

impl Fill {
    pub const fn new(token: Token) -> Self {
        Self { token, alpha: 1.0 }
    }
}

/// `Rect`/`RoundedRect`: radius 0 is a plain rect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RectShape {
    pub rect: Bounds,
    pub radius: f64,
    pub fill: Option<Fill>,
    pub stroke: Option<Stroke>,
}

impl RectShape {
    /// The opaque `c-mask` rect: `Token::Mask` fill, no stroke.
    pub fn mask(rect: Bounds, radius: f64) -> Self {
        Self { rect, radius, fill: Some(Fill::new(Token::Mask)), stroke: None }
    }
}

/// The arrowhead marker of a polyline (`marker-end`): only the fill token (the edge's colour).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub fill: Token,
}

/// An arrowhead: the SVG marker `10 x 7`, ref `(9, 3.5)`, `markerUnits = strokeWidth` (`00` §5.2).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arrow {
    /// The polyline's last point; the head's tip sits `1 * sw` beyond it.
    pub end: Pt,
    /// Unit direction of the last segment.
    pub dir: Pt,
    pub sw: f64,
}

impl Arrow {
    /// The filled triangle `0,0 10,3.5 0,7` mapped through the marker: base, tip, base.
    pub fn triangle(&self) -> [Pt; 3] {
        let [dx, dy] = self.dir;
        let (nx, ny) = (-dy, dx);
        let at = |x: f64, y: f64| {
            let (u, v) = ((x - 9.0) * self.sw, (y - 3.5) * self.sw);
            [self.end[0] + u * dx + v * nx, self.end[1] + u * dy + v * ny]
        };
        [at(0.0, 0.0), at(10.0, 3.5), at(0.0, 7.0)]
    }

    /// The tip, `1 * sw` past the end.
    pub fn tip(&self) -> Pt {
        self.triangle()[1]
    }
}

/// `Polyline`: an edge. `points` are the canonical un-rounded route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolylineShape {
    pub points: Vec<Pt>,
    /// Corner radius (A 8, L 10, D/S/W 0 = sharp).
    pub radius: f64,
    pub stroke: Stroke,
    pub marker: Option<Marker>,
    /// Draw the mask-coloured underlay `stroke.width + 4` (round caps and joins) first.
    pub halo: bool,
}

impl PolylineShape {
    /// Width of the crossover underlay.
    pub fn halo_width(&self) -> f64 {
        self.stroke.width + 4.0
    }

    /// The rounded outline as `M L Q` commands (identical corners to Archify's `roundedPath`).
    pub fn cmds(&self) -> Vec<Cmd> {
        geom::rounded_cmds(&self.points, self.radius).into_iter().map(Cmd::from).collect()
    }

    /// The rounded outline flattened to points, for painters that only stroke lines.
    pub fn flattened(&self) -> Vec<Pt> {
        geom::flatten_rounded(&self.points, self.radius)
    }

    /// The arrowhead, if the polyline has a marker and a non-degenerate last segment.
    pub fn arrow(&self) -> Option<Arrow> {
        self.marker?;
        let end = *self.points.last()?;
        let prev = self.points.iter().rev().skip(1).find(|p| *p != &end)?;
        let (dx, dy) = (end[0] - prev[0], end[1] - prev[1]);
        let len = dx.hypot(dy);
        (len > 0.0).then(|| Arrow { end, dir: [dx / len, dy / len], sw: self.stroke.width })
    }
}

/// Text alignment (`text-anchor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Anchor {
    Start,
    Middle,
}

/// Reading-depth class of a text (`data-detail`, D12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Detail {
    /// The node label, titles, legend: always shown.
    Anchor,
    /// Sublabels, edge labels, notes: hidden at `Tier::Map`.
    Context,
    /// Tags, step, classification: shown only at `Tier::Full`.
    Fine,
}

/// The reading-depth tier from `zoom / fit_scale` (`00` §5.9, D12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Map,
    Read,
    Full,
}

impl Tier {
    /// `< 1` map, `[1, 1.75)` read, `>= 1.75` full.
    pub fn from_scale(relative: f64) -> Tier {
        if relative < 1.0 {
            Tier::Map
        } else if relative < 1.75 {
            Tier::Read
        } else {
            Tier::Full
        }
    }
}

impl Detail {
    pub fn visible(self, tier: Tier) -> bool {
        match self {
            Detail::Anchor => true,
            Detail::Context => tier >= Tier::Read,
            Detail::Fine => tier >= Tier::Full,
        }
    }
}

/// A text run. `at` is the SVG baseline point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextShape {
    pub at: Pt,
    pub text: String,
    /// Font size in vu.
    pub size: f64,
    pub weight: u16,
    pub anchor: Anchor,
    pub token: Token,
    pub detail: Detail,
}

/// `Path`: sigil and brand glyphs. `transform` is the SVG matrix `[a b c d e f]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathShape {
    pub cmds: Vec<Cmd>,
    pub transform: [f64; 6],
    pub stroke: Option<Stroke>,
    pub fill: Option<Fill>,
    pub opacity: f64,
    /// `vector-effect: non-scaling-stroke`: `stroke.width` is screen px, not vu.
    pub non_scaling: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    Rect(RectShape),
    Polyline(PolylineShape),
    Text(TextShape),
    Path(PathShape),
}

/// Z-order (`00` §5.4, must be kept). Workflow lanes, phases and groups are `Frames`. `Overlay`
/// is where motion would sit; the static scene leaves it empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layer {
    Background,
    Frames,
    /// Per edge, in order: optional halo underlay, then the stroke (the shape's `halo` flag).
    Edges,
    Overlay,
    /// Per node: mask, fill, sigil, brand, texts.
    Nodes,
    EdgeLabels,
    FrameTitles,
    Legend,
}

/// Index into `Scene::groups`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroupId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupKind {
    Node,
    Edge,
    Frame,
    Label,
    Legend,
}

impl GroupKind {
    /// The layer a group of this kind is hit-tested at (its main item's layer).
    pub fn layer(self) -> Layer {
        match self {
            GroupKind::Node => Layer::Nodes,
            GroupKind::Edge => Layer::Edges,
            GroupKind::Frame => Layer::Frames,
            GroupKind::Label => Layer::EdgeLabels,
            GroupKind::Legend => Layer::Legend,
        }
    }
}

/// `data-edge-*` of an edge or label group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeRef {
    pub key: u32,
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// How a group is hit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hit {
    /// Inside `Group::bounds`.
    Rect,
    /// Within `half_width` of the route (the un-rounded points).
    Polyline { points: Vec<Pt>, half_width: f64 },
}

/// Semantics of a set of items: not painted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub kind: GroupKind,
    /// `node-{id}`, `edge-{key}`, `frame-{kind}-{id}`, `label-{key}`, `legend`.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<EdgeRef>,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sublabel: Option<String>,
    /// `data-node-context`: the container path joined by ` › `.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub bounds: Bounds,
    pub hit: Hit,
}

impl Group {
    pub fn new(kind: GroupKind, id: impl Into<String>, bounds: Bounds) -> Self {
        Self {
            kind,
            id: id.into(),
            node_id: None,
            node_kind: None,
            edge: None,
            label: String::new(),
            sublabel: None,
            context: None,
            tags: Vec::new(),
            bounds,
            hit: Hit::Rect,
        }
    }

    /// A node group.
    pub fn node(id: &str, kind: Kind, label: &str, bounds: Bounds) -> Self {
        let mut g = Group::new(GroupKind::Node, format!("node-{id}"), bounds);
        g.node_id = Some(id.to_string());
        g.node_kind = Some(kind);
        g.label = label.to_string();
        g
    }

    /// An edge group hit along its route.
    pub fn edge(edge: EdgeRef, label: &str, points: &[Pt], half_width: f64) -> Self {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in points {
            for i in 0..2 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let bounds = if points.is_empty() {
            Bounds::new(0.0, 0.0, 0.0, 0.0)
        } else {
            Bounds::new(lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1])
        };
        let mut g = Group::new(GroupKind::Edge, format!("edge-{}", edge.key), bounds);
        g.edge = Some(edge);
        g.label = label.to_string();
        g.hit = Hit::Polyline { points: points.to_vec(), half_width };
        g
    }

    /// `label · sublabel · context [· tag]`, the node `<title>` (`03` §5.2).
    pub fn tooltip(&self) -> String {
        let mut parts: Vec<&str> = vec![self.label.as_str()];
        parts.extend(self.sublabel.as_deref());
        parts.extend(self.context.as_deref());
        parts.extend(self.tags.iter().map(String::as_str));
        parts.retain(|p| !p.is_empty());
        parts.join(" · ")
    }

    fn contains(&self, p: Pt, slop: f64) -> bool {
        match &self.hit {
            Hit::Rect => self.bounds.contains(p, slop),
            Hit::Polyline { points, half_width } => {
                if !self.bounds.contains(p, slop + half_width) {
                    return false;
                }
                let limit = half_width + slop;
                match points.as_slice() {
                    [] => false,
                    [only] => (p[0] - only[0]).hypot(p[1] - only[1]) <= limit,
                    _ => points.windows(2).any(|w| geom::point_segment_distance(p, w[0], w[1]) <= limit),
                }
            }
        }
    }
}

/// One painted primitive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub layer: Layer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupId>,
    pub shape: Shape,
}

/// The scene: items already in z-order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// `(width, height)`; the origin is `(0, 0)`.
    pub view_box: [f64; 2],
    pub items: Vec<Item>,
    pub groups: Vec<Group>,
    /// The preset the colours resolve in and the frames are styled for ([`crate::tokens::restyle`]).
    /// Layout draws classic; the interface restyles for the effective preset.
    #[serde(default, skip_serializing_if = "Preset::is_classic")]
    pub preset: Preset,
}

impl Scene {
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.get(id.0 as usize)
    }

    /// The items of one group, in paint order.
    pub fn items_of(&self, id: GroupId) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(move |i| i.group == Some(id))
    }

    pub fn node(&self, node_id: &str) -> Option<GroupId> {
        self.group_ids().find(|id| {
            let g = &self.groups[id.0 as usize];
            g.kind == GroupKind::Node && g.node_id.as_deref() == Some(node_id)
        })
    }

    /// The edge groups touching a node, as `(group, other end)`: the neighbourhood focus keeps lit.
    pub fn edges_of(&self, node_id: &str) -> Vec<(GroupId, &str)> {
        self.group_ids()
            .filter_map(|id| {
                let e = self.groups[id.0 as usize].edge.as_ref()?;
                if self.groups[id.0 as usize].kind != GroupKind::Edge {
                    return None;
                }
                if e.from == node_id {
                    Some((id, e.to.as_str()))
                } else if e.to == node_id {
                    Some((id, e.from.as_str()))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn tooltip(&self, id: GroupId) -> Option<String> {
        self.group(id).map(Group::tooltip)
    }

    /// The topmost group under `p` (viewBox coordinates): highest [`Layer`] first, later groups
    /// over earlier ones within a layer. `slop` widens every hit area (a screen-px tolerance
    /// divided by the zoom, supplied by the painter).
    pub fn hit_test(&self, p: Pt, slop: f64) -> Option<GroupId> {
        let mut order: Vec<GroupId> = self.group_ids().collect();
        order.sort_by_key(|id| (self.groups[id.0 as usize].kind.layer(), *id));
        order.into_iter().rev().find(|id| self.groups[id.0 as usize].contains(p, slop))
    }

    fn group_ids(&self) -> impl Iterator<Item = GroupId> + use<> {
        (0..self.groups.len() as u32).map(GroupId)
    }
}

/// Collects groups and items in any order and sorts the items into z-order once.
#[derive(Debug)]
pub struct SceneBuilder {
    view_box: [f64; 2],
    items: Vec<Item>,
    groups: Vec<Group>,
}

impl SceneBuilder {
    pub fn new(width: f64, height: f64) -> Self {
        Self { view_box: [width, height], items: Vec::new(), groups: Vec::new() }
    }

    /// Register a group and return its id.
    pub fn group(&mut self, group: Group) -> GroupId {
        self.groups.push(group);
        GroupId(self.groups.len() as u32 - 1)
    }

    /// An item outside any group (background, grid).
    pub fn push(&mut self, layer: Layer, shape: Shape) {
        self.items.push(Item { layer, group: None, shape });
    }

    pub fn push_in(&mut self, group: GroupId, layer: Layer, shape: Shape) {
        self.items.push(Item { layer, group: Some(group), shape });
    }

    /// Stable sort by layer: insertion order is kept within a layer.
    pub fn build(mut self) -> Scene {
        self.items.sort_by_key(|i| i.layer);
        Scene { view_box: self.view_box, items: self.items, groups: self.groups, preset: Preset::Classic }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Shape {
        Shape::Text(TextShape {
            at: [0.0, 0.0],
            text: s.into(),
            size: 10.0,
            weight: 600,
            anchor: Anchor::Middle,
            token: Token::Text,
            detail: Detail::Anchor,
        })
    }

    fn line(points: &[Pt], halo: bool, marker: bool) -> PolylineShape {
        PolylineShape {
            points: points.to_vec(),
            radius: 0.0,
            stroke: Stroke::solid(Token::Arrow, 1.4),
            marker: marker.then_some(Marker { fill: Token::Arrow }),
            halo,
        }
    }

    #[test]
    fn layers_are_ordered_as_the_spec() {
        use Layer::*;
        let order = [Background, Frames, Edges, Overlay, Nodes, EdgeLabels, FrameTitles, Legend];
        assert!(order.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn build_sorts_stably_by_layer() {
        let mut b = SceneBuilder::new(100.0, 50.0);
        let n = b.group(Group::node("a", Kind::Backend, "A", Bounds::new(0.0, 0.0, 10.0, 10.0)));
        b.push_in(n, Layer::Nodes, Shape::Rect(RectShape::mask(Bounds::new(0.0, 0.0, 10.0, 10.0), 6.0)));
        b.push(Layer::Legend, text("legend"));
        b.push(Layer::Edges, Shape::Polyline(line(&[[0.0, 0.0], [5.0, 0.0]], true, true)));
        b.push_in(n, Layer::Nodes, text("A"));
        b.push(Layer::Background, text("bg"));
        b.push(Layer::Edges, Shape::Polyline(line(&[[0.0, 1.0], [5.0, 1.0]], false, true)));
        let s = b.build();
        let layers: Vec<Layer> = s.items.iter().map(|i| i.layer).collect();
        use Layer::*;
        assert_eq!(layers, [Background, Edges, Edges, Nodes, Nodes, Legend]);
        // Insertion order survives inside a layer: the halo edge first, and the mask before the text.
        assert!(matches!(&s.items[1].shape, Shape::Polyline(p) if p.halo));
        assert!(matches!(&s.items[2].shape, Shape::Polyline(p) if !p.halo));
        assert!(matches!(&s.items[3].shape, Shape::Rect(r) if r.fill.map(|f| f.token) == Some(Token::Mask)));
        assert_eq!(s.items_of(n).count(), 2);
    }

    #[test]
    fn halo_is_stroke_plus_four() {
        assert_eq!(line(&[[0.0, 0.0], [9.0, 0.0]], true, false).halo_width(), 5.4);
    }

    #[test]
    fn arrow_matches_the_marker() {
        // sw 2 pointing +x at the origin: tip 1*sw past the end, base 9*sw behind, half width 3.5*sw.
        let a = line(&[[-30.0, 0.0], [0.0, 0.0]], false, true);
        let a = Arrow { sw: 2.0, ..a.arrow().unwrap() };
        let [b0, tip, b1] = a.triangle();
        assert_eq!(tip, [2.0, 0.0]);
        assert_eq!(b0, [-18.0, -7.0]);
        assert_eq!(b1, [-18.0, 7.0]);
        // Pointing +y.
        let down = Arrow { end: [10.0, 10.0], dir: [0.0, 1.0], sw: 1.0 }.triangle();
        assert_eq!(down[1], [10.0, 11.0]);
        assert_eq!(down[0], [13.5, 1.0]);
        assert_eq!(down[2], [6.5, 1.0]);
        // No marker, or a zero-length route, has no head; trailing duplicates are skipped.
        assert!(line(&[[0.0, 0.0], [5.0, 0.0]], false, false).arrow().is_none());
        assert!(line(&[[1.0, 1.0], [1.0, 1.0]], false, true).arrow().is_none());
        let d = line(&[[0.0, 0.0], [0.0, 5.0], [0.0, 5.0]], false, true).arrow().unwrap();
        assert_eq!(d.dir, [0.0, 1.0]);
    }

    #[test]
    fn rounded_outline_uses_geom_corners() {
        let mut p = line(&[[0.0, 0.0], [40.0, 0.0], [40.0, 40.0]], false, true);
        p.radius = 8.0;
        let cmds = p.cmds();
        assert_eq!(cmds[0], Cmd::M([0.0, 0.0]));
        assert_eq!(cmds[1], Cmd::L([32.0, 0.0]));
        assert_eq!(cmds[2], Cmd::Q([40.0, 0.0], [40.0, 8.0]));
        assert_eq!(*p.flattened().last().unwrap(), [40.0, 40.0]);
    }

    #[test]
    fn hit_test_picks_topmost_and_follows_routes() {
        let mut b = SceneBuilder::new(200.0, 100.0);
        let frame = b.group(Group::new(GroupKind::Frame, "frame-lane-0", Bounds::new(0.0, 0.0, 200.0, 100.0)));
        let edge = b.group(Group::edge(
            EdgeRef { key: 0, from: "a".into(), to: "b".into(), id: None },
            "go",
            &[[40.0, 20.0], [120.0, 20.0], [120.0, 80.0]],
            0.7,
        ));
        let a = b.group(Group::node("a", Kind::Frontend, "A", Bounds::new(0.0, 0.0, 40.0, 40.0)));
        let bb = b.group(Group::node("b", Kind::Backend, "B", Bounds::new(100.0, 80.0, 40.0, 40.0)));
        let s = b.build();
        assert_eq!(s.hit_test([10.0, 10.0], 0.0), Some(a));
        assert_eq!(s.hit_test([80.0, 20.0], 0.0), Some(edge));
        assert_eq!(s.hit_test([80.0, 23.0], 0.0), Some(frame), "3 px off the route misses the edge");
        assert_eq!(s.hit_test([80.0, 23.0], 3.0), Some(edge), "slop widens the rail");
        assert_eq!(s.hit_test([120.0, 60.0], 0.0), Some(edge));
        assert_eq!(s.hit_test([110.0, 90.0], 0.0), Some(bb));
        assert_eq!(s.hit_test([500.0, 500.0], 0.0), None);
        let near: Vec<&str> = s.edges_of("a").into_iter().map(|(_, o)| o).collect();
        assert_eq!(near, ["b"]);
        assert_eq!(s.node("b"), Some(bb));
    }

    #[test]
    fn tooltip_joins_the_parts() {
        let mut g = Group::node("a", Kind::Backend, "Billing API", Bounds::new(0.0, 0.0, 1.0, 1.0));
        g.sublabel = Some("payment producer".into());
        g.context = Some("01 / Producers".into());
        g.tags = vec!["team money".into()];
        assert_eq!(g.tooltip(), "Billing API · payment producer · 01 / Producers · team money");
        g.sublabel = None;
        g.tags.clear();
        assert_eq!(g.tooltip(), "Billing API · 01 / Producers");
    }

    #[test]
    fn tiers_follow_d12() {
        assert_eq!(Tier::from_scale(0.99), Tier::Map);
        assert_eq!(Tier::from_scale(1.0), Tier::Read);
        assert_eq!(Tier::from_scale(1.74), Tier::Read);
        assert_eq!(Tier::from_scale(1.75), Tier::Full);
        assert!(Detail::Anchor.visible(Tier::Map));
        assert!(!Detail::Context.visible(Tier::Map));
        assert!(Detail::Context.visible(Tier::Read));
        assert!(!Detail::Fine.visible(Tier::Read), "tags are hidden at fit");
        assert!(Detail::Fine.visible(Tier::Full));
    }

    #[test]
    fn scene_round_trips_through_json() {
        let mut b = SceneBuilder::new(1080.0, 588.0);
        let g = b.group(Group::node("a", Kind::Cloud, "A", Bounds::new(1.0, 2.0, 3.0, 4.0)));
        b.push_in(g, Layer::Nodes, Shape::Rect(RectShape {
            rect: Bounds::new(1.0, 2.0, 3.0, 4.0),
            radius: 6.0,
            fill: Some(Fill::new(Token::KindFill(Kind::Cloud))),
            stroke: Some(Stroke::dashed(Token::KindStroke(Kind::Cloud), 1.5, &[8.0, 4.0])),
        }));
        b.push_in(g, Layer::Nodes, Shape::Path(PathShape {
            cmds: vec![Cmd::M([0.0, 0.0]), Cmd::C([1.0, 1.0], [2.0, 2.0], [3.0, 3.0]), Cmd::Z],
            transform: [1.0, 0.0, 0.0, 1.0, 6.0, 6.0],
            stroke: None,
            fill: None,
            opacity: 0.76,
            non_scaling: true,
        }));
        b.push(Layer::Edges, Shape::Polyline(line(&[[0.0, 0.0], [5.0, 5.0]], true, true)));
        let s = b.build();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Scene>(&json).unwrap(), s);
    }
}
