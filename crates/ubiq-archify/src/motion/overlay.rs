//! What [`sample`](super::sample) returns: primitives to paint over the static scene. No GPUI, no
//! colour values (tokens), serde for the goldens.
//!
//! Z-order (`04` §6.4): the `strokes` go above the edges and **below the nodes** (the scene's
//! `Layer::Overlay`), so comets pass beneath the node masks; the `highlights` are node outlines
//! and glows and sit above the node they belong to. `group_alpha` multiplies every item of the
//! group and `detail_alpha` replaces the tier visibility of context/fine texts while a LOD fade
//! runs. An empty [`Overlay`] means "paint the static scene": authored shapes are never mutated.

use serde::{Deserialize, Serialize};

use super::comet::CometKind;
use crate::geom::Pt;
use crate::scene::{Bounds, GroupId};
use crate::tokens::Token;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrokeKind {
    /// A moving (or static-fallback) comet.
    Comet(CometKind),
    /// A comet that finished and stays parked (route probe).
    Parked(CometKind),
    /// One dash of the ambient edge trace, already cut out of the polyline.
    AmbientDash,
}

/// An open polyline stroke with round caps. `points` are already sliced: the painter never needs
/// dash support, it strokes the sub-path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OverlayStroke {
    pub kind: StrokeKind,
    /// The edge group whose geometry this is a clone of.
    pub group: GroupId,
    pub points: Vec<Pt>,
    pub token: Token,
    /// Screen px when `non_scaling` (comets: `vector-effect: non-scaling-stroke`), vu otherwise.
    pub width: f64,
    pub non_scaling: bool,
    pub alpha: f64,
    /// Glow radius in screen px (the CSS `drop-shadow`); 0 for none. A painter without blur
    /// strokes a wider low-alpha copy or skips it.
    pub glow: f64,
}

/// A rounded outline around a node with an optional glow: the ambient pulse and the intent
/// trace's selected node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub group: GroupId,
    pub rect: Bounds,
    pub radius: f64,
    pub token: Token,
    pub width: f64,
    pub alpha: f64,
    pub glow: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupAlpha {
    pub group: GroupId,
    pub alpha: f64,
}

/// Visibility of the two fade-able reading depths (`Detail::Anchor` is always 1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetailAlpha {
    pub context: f64,
    pub fine: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    pub strokes: Vec<OverlayStroke>,
    pub highlights: Vec<Highlight>,
    /// Only groups whose multiplier is not 1, ascending by id.
    pub group_alpha: Vec<GroupAlpha>,
    /// `Some` only while a tier fade runs; otherwise the painter uses `Detail::visible(tier)`.
    pub detail_alpha: Option<DetailAlpha>,
}

impl Overlay {
    /// Nothing to paint over the static scene.
    pub fn is_identity(&self) -> bool {
        self.strokes.is_empty()
            && self.highlights.is_empty()
            && self.group_alpha.is_empty()
            && self.detail_alpha.is_none()
    }

    /// The multiplier of a group (1 when absent).
    pub fn alpha_of(&self, group: GroupId) -> f64 {
        self.group_alpha
            .binary_search_by_key(&group, |g| g.group)
            .map_or(1.0, |i| self.group_alpha[i].alpha)
    }
}
