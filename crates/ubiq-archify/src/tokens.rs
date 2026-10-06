//! The colour vocabulary of a scene, as data (P2.3, D11): a [`Token`] enum, the seven node
//! [`Kind`]s, the relationship [`EdgeVariant`]s, and Archify's *classic* palette for both modes
//! (`03` §4.4, `assets/template.html:172-263`). No GPUI colour here: `crates/archify/src/theme.rs`
//! resolves a `Token` to a real colour, taking the chrome tokens from the active Ubiq palette and
//! the kind tokens from [`classic`] (D11). [`classic`] also carries the chrome values, so the
//! alternative of D11 (full Archify parity) needs no data change.
//!
//! Alpha lives in the value, not in the scene: `Token::KindFill(k)` is already translucent
//! (`rgba(8,51,68,.4)`), exactly as the CSS custom property is.

use serde::{Deserialize, Serialize};

use crate::scene::{Cmd, Item, Layer, PathShape, Scene, Shape, Stroke};

/// The seven node kinds (`03` §4.3). Lifecycle types map onto them with [`Kind::from_state_type`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Frontend,
    Backend,
    Database,
    Cloud,
    Security,
    Messagebus,
    External,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Frontend,
        Kind::Backend,
        Kind::Database,
        Kind::Cloud,
        Kind::Security,
        Kind::Messagebus,
        Kind::External,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Frontend => "frontend",
            Kind::Backend => "backend",
            Kind::Database => "database",
            Kind::Cloud => "cloud",
            Kind::Security => "security",
            Kind::Messagebus => "messagebus",
            Kind::External => "external",
        }
    }

    /// An authored node `type`. Unknown values are `None` (the renderers fall back to `external`).
    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Lifecycle state `type` to kind (`render-lifecycle.mjs:175-195`): start and active are
    /// frontend, waiting cloud, decision database, success backend, failure security, everything
    /// else external. The node kinds themselves map to themselves.
    pub fn from_state_type(s: &str) -> Kind {
        match s {
            "start" | "active" => Kind::Frontend,
            "waiting" => Kind::Cloud,
            "decision" => Kind::Database,
            "success" => Kind::Backend,
            "failure" => Kind::Security,
            "neutral" | "external" => Kind::External,
            other => Kind::parse(other).unwrap_or(Kind::External),
        }
    }
}

/// A named colour. Resolved per frame by the theme adapter, never cached across frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Token {
    /// `--bg`, the canvas ground.
    Bg,
    /// `--grid`, the 40x40 background grid line (hidden in the classic preset).
    Grid,
    /// `--canvas-dot`.
    CanvasDot,
    /// `--text`, class `t-primary`.
    Text,
    /// `--text-muted`, class `t-muted`.
    TextMuted,
    /// `--text-dim`, class `t-dim`.
    TextDim,
    /// `--text-faint`, UI hints.
    TextFaint,
    /// `--panel`.
    Panel,
    /// `--panel-border`.
    PanelBorder,
    /// `--lane-fill`, lanes, stages, segments, groups.
    LaneFill,
    /// `--lane-stroke`.
    LaneStroke,
    /// `--arrow`, default edge stroke and label text.
    Arrow,
    /// `--arrow-emphasis`.
    ArrowEmphasis,
    /// `--mask`, the opaque colour under node fills, edge labels, titles and crossover halos.
    Mask,
    /// `--<kind>-fill`, translucent.
    KindFill(Kind),
    /// `--<kind>-stroke`: the node stroke, its text accent, its sigil, and the security/dashed
    /// edge variants.
    KindStroke(Kind),
    /// `.c-region` fill (`rgba(251,191,36,.05)`); the stroke is `KindStroke(Cloud)`.
    RegionFill,
    /// Brand badge plate (`#fff`).
    BrandBadge,
    /// Brand badge frame (`#cbd5e1`) and fallback glyph (`#475569`).
    BrandFrame,
    BrandGlyph,
    /// A built-in brand mark's own colour, `0xRRGGBB` (the vendor's `hex`, `brand.rs`). The same in
    /// every preset and mode: it sits inside the white badge plate.
    BrandInk(u32),
}

/// Light or dark, picked from the active Ubiq palette by the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Dark,
    Light,
}

/// A colour with straight alpha, as written in the CSS.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f64,
}

impl Rgba {
    pub const fn rgb(hex: u32) -> Rgba {
        Rgba { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8, a: 1.0 }
    }

    pub const fn new(r: u8, g: u8, b: u8, a: f64) -> Rgba {
        Rgba { r, g, b, a }
    }

    /// `#rrggbb` for opaque colours, `rgba(r,g,b,a)` otherwise: the CSS spelling.
    pub fn css(&self) -> String {
        if self.a >= 1.0 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("rgba({}, {}, {}, {})", self.r, self.g, self.b, self.a)
        }
    }
}

/// The fill and stroke of one kind.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KindColors {
    pub fill: Rgba,
    pub stroke: Rgba,
}

/// A flat token table (`03` §4.4: "theme = {Theme, Preset} to a flat token struct").
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    pub bg: Rgba,
    pub grid: Rgba,
    pub canvas_dot: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_dim: Rgba,
    pub text_faint: Rgba,
    pub panel: Rgba,
    pub panel_border: Rgba,
    pub lane_fill: Rgba,
    pub lane_stroke: Rgba,
    pub arrow: Rgba,
    pub arrow_emphasis: Rgba,
    pub mask: Rgba,
    pub region_fill: Rgba,
    /// Indexed by [`Kind::ALL`] order.
    pub kinds: [KindColors; 7],
}

impl Palette {
    pub fn kind(&self, kind: Kind) -> &KindColors {
        &self.kinds[Kind::ALL.iter().position(|k| *k == kind).unwrap_or(6)]
    }

    pub fn resolve(&self, token: Token) -> Rgba {
        match token {
            Token::Bg => self.bg,
            Token::Grid => self.grid,
            Token::CanvasDot => self.canvas_dot,
            Token::Text => self.text,
            Token::TextMuted => self.text_muted,
            Token::TextDim => self.text_dim,
            Token::TextFaint => self.text_faint,
            Token::Panel => self.panel,
            Token::PanelBorder => self.panel_border,
            Token::LaneFill => self.lane_fill,
            Token::LaneStroke => self.lane_stroke,
            Token::Arrow => self.arrow,
            Token::ArrowEmphasis => self.arrow_emphasis,
            Token::Mask => self.mask,
            Token::RegionFill => self.region_fill,
            Token::KindFill(k) => self.kind(k).fill,
            Token::KindStroke(k) => self.kind(k).stroke,
            Token::BrandBadge => Rgba::rgb(0xffffff),
            Token::BrandFrame => Rgba::rgb(0xcbd5e1),
            Token::BrandGlyph => Rgba::rgb(0x475569),
            Token::BrandInk(hex) => Rgba::rgb(hex),
        }
    }
}

const fn kc(fill: Rgba, stroke: u32) -> KindColors {
    KindColors { fill, stroke: Rgba::rgb(stroke) }
}

/// Classic dark (`template.html:172-216`).
pub static CLASSIC_DARK: Palette = Palette {
    bg: Rgba::rgb(0x020617),
    grid: Rgba::rgb(0x1e293b),
    canvas_dot: Rgba::new(148, 163, 184, 0.16),
    text: Rgba::rgb(0xffffff),
    text_muted: Rgba::rgb(0x94a3b8),
    text_dim: Rgba::rgb(0x475569),
    text_faint: Rgba::rgb(0x7d8da1),
    panel: Rgba::new(15, 23, 42, 0.5),
    panel_border: Rgba::rgb(0x1e293b),
    lane_fill: Rgba::new(15, 23, 42, 0.22),
    lane_stroke: Rgba::rgb(0x334155),
    arrow: Rgba::rgb(0x64748b),
    arrow_emphasis: Rgba::rgb(0x34d399),
    mask: Rgba::rgb(0x0f172a),
    region_fill: Rgba::new(251, 191, 36, 0.05),
    kinds: [
        kc(Rgba::new(8, 51, 68, 0.4), 0x22d3ee),
        kc(Rgba::new(6, 78, 59, 0.4), 0x34d399),
        kc(Rgba::new(76, 29, 149, 0.4), 0xa78bfa),
        kc(Rgba::new(120, 53, 15, 0.3), 0xfbbf24),
        kc(Rgba::new(136, 19, 55, 0.4), 0xfb7185),
        kc(Rgba::new(251, 146, 60, 0.3), 0xfb923c),
        kc(Rgba::new(30, 41, 59, 0.5), 0x94a3b8),
    ],
};

/// Classic light (`template.html:218-263`).
pub static CLASSIC_LIGHT: Palette = Palette {
    bg: Rgba::rgb(0xf4f5f7),
    grid: Rgba::rgb(0xe9edf2),
    canvas_dot: Rgba::rgb(0xd9dee5),
    text: Rgba::rgb(0x111827),
    text_muted: Rgba::rgb(0x5b6474),
    text_dim: Rgba::rgb(0x9aa3b2),
    text_faint: Rgba::rgb(0x6b7280),
    panel: Rgba::rgb(0xffffff),
    panel_border: Rgba::rgb(0xe5e7eb),
    lane_fill: Rgba::new(248, 250, 252, 0.65),
    lane_stroke: Rgba::rgb(0xcbd5e1),
    arrow: Rgba::rgb(0x94a3b8),
    arrow_emphasis: Rgba::rgb(0x059669),
    mask: Rgba::rgb(0xffffff),
    region_fill: Rgba::new(251, 191, 36, 0.05),
    kinds: [
        kc(Rgba::new(34, 211, 238, 0.15), 0x0891b2),
        kc(Rgba::new(52, 211, 153, 0.18), 0x059669),
        kc(Rgba::new(167, 139, 250, 0.2), 0x7c3aed),
        kc(Rgba::new(251, 191, 36, 0.18), 0xd97706),
        kc(Rgba::new(251, 113, 133, 0.15), 0xe11d48),
        kc(Rgba::new(251, 146, 60, 0.15), 0xea580c),
        kc(Rgba::new(148, 163, 184, 0.18), 0x64748b),
    ],
};

/// The classic palette for a mode.
pub fn classic(mode: Mode) -> &'static Palette {
    match mode {
        Mode::Dark => &CLASSIC_DARK,
        Mode::Light => &CLASSIC_LIGHT,
    }
}

// SIGNAL_FLOW_*, BLUEPRINT_*, EDITORIAL_*: generated by `gen-palettes.mjs`.
include!("data/palettes.rs");

/// A visual preset (`meta.visual_preset`, `00` §5.2): the colour table and the frame style.
/// Archify's viewer cycles them at run time; the document picks the one it opens in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    #[default]
    Classic,
    SignalFlow,
    Blueprint,
    Editorial,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::Classic, Preset::SignalFlow, Preset::Blueprint, Preset::Editorial];

    /// `data-preset`.
    pub fn as_str(self) -> &'static str {
        match self {
            Preset::Classic => "classic",
            Preset::SignalFlow => "signal-flow",
            Preset::Blueprint => "blueprint",
            Preset::Editorial => "editorial",
        }
    }

    pub fn parse(s: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.as_str() == s)
    }

    pub fn is_classic(&self) -> bool {
        *self == Preset::Classic
    }

    /// The preset a picture takes: the settings' palette when it names one, else (`document`, or a
    /// value this build does not know) the preset the document opens in.
    pub fn pick(palette: &str, authored: Option<crate::model::common::VisualPreset>) -> Preset {
        Preset::parse(palette).unwrap_or_else(|| Preset::of(authored))
    }

    /// The preset a document opens in.
    pub fn of(authored: Option<crate::model::common::VisualPreset>) -> Preset {
        use crate::model::common::VisualPreset as V;
        match authored {
            None | Some(V::Classic) => Preset::Classic,
            Some(V::SignalFlow) => Preset::SignalFlow,
            Some(V::Blueprint) => Preset::Blueprint,
            Some(V::Editorial) => Preset::Editorial,
        }
    }
}

/// The full Archify palette of a preset and mode: classic's table, or the preset's block over it.
pub fn palette(preset: Preset, mode: Mode) -> &'static Palette {
    match (preset, mode) {
        (Preset::Classic, m) => classic(m),
        (Preset::SignalFlow, Mode::Dark) => &SIGNAL_FLOW_DARK,
        (Preset::SignalFlow, Mode::Light) => &SIGNAL_FLOW_LIGHT,
        (Preset::Blueprint, Mode::Dark) => &BLUEPRINT_DARK,
        (Preset::Blueprint, Mode::Light) => &BLUEPRINT_LIGHT,
        (Preset::Editorial, Mode::Dark) => &EDITORIAL_DARK,
        (Preset::Editorial, Mode::Light) => &EDITORIAL_LIGHT,
    }
}

/// The background grid (`.c-grid`, `utils.mjs:26-28`): a 40 x 40 tile of `M 40 0 L 0 0 0 40`, stroke
/// 0.5 in `--grid`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridStyle {
    /// `stroke-dasharray`; empty is solid.
    pub dash: &'static [f64],
    pub opacity: f64,
}

/// What a preset changes in the static picture besides colour (`T:4036`, `T:4654-4697`). Dash arrays
/// are `None` where the preset keeps the attribute Archify writes (`8,4` region, `6,6` lane and
/// stage, `4,4` security group).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameStyle {
    /// `None`: no grid (classic hides it).
    pub grid: Option<GridStyle>,
    pub region_dash: Option<&'static [f64]>,
    pub lane_dash: Option<&'static [f64]>,
    pub security_group_dash: Option<&'static [f64]>,
    /// Radius of the brand badge plate and its frame (`rx 1px` for blueprint).
    pub badge_radius: f64,
}

pub const GRID_TILE: f64 = 40.0;
pub const GRID_STROKE: f64 = 0.5;

pub fn frame_style(preset: Preset) -> FrameStyle {
    let base = FrameStyle {
        grid: None,
        region_dash: None,
        lane_dash: None,
        security_group_dash: None,
        badge_radius: crate::brand::RADIUS,
    };
    match preset {
        Preset::Classic => base,
        Preset::SignalFlow => FrameStyle { grid: Some(GridStyle { dash: &[], opacity: 1.0 }), ..base },
        Preset::Editorial => FrameStyle {
            grid: Some(GridStyle { dash: &[1.0, 4.0], opacity: 0.48 }),
            region_dash: Some(&[9.0, 4.0]),
            ..base
        },
        Preset::Blueprint => FrameStyle {
            grid: Some(GridStyle { dash: &[1.0, 3.0], opacity: 0.78 }),
            region_dash: Some(&[12.0, 4.0, 2.0, 4.0]),
            lane_dash: Some(&[7.0, 3.0]),
            security_group_dash: Some(&[7.0, 3.0]),
            badge_radius: 1.0,
        },
    }
}

/// Applies `preset` to a scene fresh from layout (which draws Archify's attribute values): the
/// frame dashes, the brand badge radius and the background grid. The colour is resolved at paint
/// time from [`palette`]. Call once per scene; `scene.preset` records the result.
pub fn restyle(scene: &mut Scene, preset: Preset) {
    scene.preset = preset;
    let style = frame_style(preset);
    for item in &mut scene.items {
        match &mut item.shape {
            Shape::Rect(r) if item.layer == Layer::Frames => {
                let Some(stroke) = r.stroke.as_mut().filter(|s| !s.dash.is_empty()) else { continue };
                let dash = |d: &[f64]| stroke.dash == d;
                let new = if stroke.token == Token::KindStroke(Kind::Cloud) && dash(&[8.0, 4.0]) {
                    style.region_dash
                } else if stroke.token == Token::LaneStroke && dash(&[6.0, 6.0]) {
                    style.lane_dash
                } else if stroke.token == Token::KindStroke(Kind::Security) && dash(&[4.0, 4.0]) {
                    style.security_group_dash
                } else {
                    None
                };
                if let Some(new) = new {
                    stroke.dash = new.to_vec();
                }
            }
            Shape::Rect(r) if r.fill.is_some_and(|f| f.token == Token::BrandBadge) => r.radius = style.badge_radius,
            Shape::Path(p) if p.stroke.as_ref().is_some_and(|s| s.token == Token::BrandFrame) => {
                p.cmds = crate::brand::frame_cmds(style.badge_radius);
            }
            _ => {}
        }
    }
    let Some(grid) = style.grid else { return };
    let [w, h] = scene.view_box;
    let mut cmds = Vec::new();
    let mut y = 0.0;
    while y < h {
        let mut x = 0.0;
        while x < w {
            // The tile's path runs from its top right, left along the top, then down its left edge;
            // the last tile of a row or column is cut at the picture's edge.
            cmds.push(Cmd::M([(x + GRID_TILE).min(w), y]));
            cmds.push(Cmd::L([x, y]));
            cmds.push(Cmd::L([x, (y + GRID_TILE).min(h)]));
            x += GRID_TILE;
        }
        y += GRID_TILE;
    }
    let shape = Shape::Path(PathShape {
        cmds,
        transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        stroke: Some(Stroke { token: Token::Grid, width: GRID_STROKE, dash: grid.dash.to_vec() }),
        fill: None,
        opacity: grid.opacity,
        non_scaling: false,
    });
    // After the ground, before everything else: the items are already in layer order.
    let at = scene.items.iter().position(|i| i.layer != Layer::Background).unwrap_or(scene.items.len());
    scene.items.insert(at, Item { layer: Layer::Background, group: None, shape });
}

/// A relationship variant (`00` §5.2): stroke, dash and label token. Widths depend on the diagram
/// type and are the layout's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeVariant {
    Default,
    Emphasis,
    Security,
    Dashed,
    /// Sequence return messages.
    Return,
}

impl EdgeVariant {
    /// An authored `variant`; absent or unknown is `Default`.
    pub fn parse(s: Option<&str>) -> EdgeVariant {
        match s {
            Some("emphasis") => EdgeVariant::Emphasis,
            Some("security") => EdgeVariant::Security,
            Some("dashed") => EdgeVariant::Dashed,
            Some("return") => EdgeVariant::Return,
            _ => EdgeVariant::Default,
        }
    }

    /// `a-*` stroke and `m-*` arrowhead fill.
    pub fn stroke(self) -> Token {
        match self {
            EdgeVariant::Default | EdgeVariant::Return => Token::Arrow,
            EdgeVariant::Emphasis => Token::ArrowEmphasis,
            EdgeVariant::Security => Token::KindStroke(Kind::Security),
            EdgeVariant::Dashed => Token::KindStroke(Kind::Database),
        }
    }

    /// The dash array; empty is solid.
    pub fn dash(self) -> &'static [f64] {
        match self {
            EdgeVariant::Security => &[5.0, 5.0],
            EdgeVariant::Dashed => &[4.0, 4.0],
            EdgeVariant::Return => &[3.0, 5.0],
            _ => &[],
        }
    }

    /// `t-edge-*`: the label text shares the path's token; `return` labels are muted.
    pub fn label(self) -> Token {
        match self {
            EdgeVariant::Return => Token::TextMuted,
            v => v.stroke(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_table_matches_the_css() {
        let d = classic(Mode::Dark);
        assert_eq!(d.kind(Kind::Frontend).stroke.css(), "#22d3ee");
        assert_eq!(d.kind(Kind::Frontend).fill.css(), "rgba(8, 51, 68, 0.4)");
        assert_eq!(d.kind(Kind::External).stroke.css(), "#94a3b8");
        let l = classic(Mode::Light);
        assert_eq!(l.kind(Kind::Security).stroke.css(), "#e11d48");
        assert_eq!(l.kind(Kind::Messagebus).fill.css(), "rgba(251, 146, 60, 0.15)");
        assert_eq!(l.resolve(Token::Mask).css(), "#ffffff");
        assert_eq!(d.resolve(Token::Mask).css(), "#0f172a");
        assert_eq!(d.resolve(Token::KindStroke(Kind::Cloud)).css(), "#fbbf24");
    }

    #[test]
    fn the_presets_carry_the_values_of_the_map() {
        // `03` section 4.4, spot values.
        let c = |p, m, t| palette(p, m).resolve(t).css();
        assert_eq!(c(Preset::SignalFlow, Mode::Dark, Token::Mask), "#07101e");
        assert_eq!(c(Preset::SignalFlow, Mode::Light, Token::ArrowEmphasis), "#0d9488");
        assert_eq!(c(Preset::Blueprint, Mode::Dark, Token::Bg), "#06131f");
        assert_eq!(c(Preset::Blueprint, Mode::Light, Token::Grid), "#b5d5e1");
        assert_eq!(c(Preset::Blueprint, Mode::Dark, Token::KindStroke(Kind::Cloud)), "#ffd166");
        assert_eq!(c(Preset::Editorial, Mode::Dark, Token::ArrowEmphasis), "#dd6b3d");
        assert_eq!(c(Preset::Editorial, Mode::Light, Token::Mask), "#fbf8f1");
        assert_eq!(c(Preset::Editorial, Mode::Dark, Token::KindStroke(Kind::External)), "#b8ad99");
        // What a preset does not override is classic's (`canvas-dot`, `panel` of the light blocks).
        assert_eq!(palette(Preset::Blueprint, Mode::Dark).canvas_dot, CLASSIC_DARK.canvas_dot);
        // `.c-region` mixes the cloud fill with transparent: 58% and 48% of its alpha.
        let a = |p| palette(p, Mode::Dark).region_fill.a;
        assert!((a(Preset::Blueprint) - 0.0754).abs() < 1e-9, "0.13 x 0.58");
        assert!((a(Preset::Editorial) - 0.0768).abs() < 1e-9, "0.16 x 0.48");
        assert_eq!(palette(Preset::SignalFlow, Mode::Dark).region_fill, CLASSIC_DARK.region_fill);
        for p in Preset::ALL {
            for m in [Mode::Dark, Mode::Light] {
                for k in &palette(p, m).kinds {
                    assert!(k.fill.a < 1.0);
                }
            }
        }
    }

    #[test]
    fn frame_styles_follow_the_stylesheet() {
        assert_eq!(frame_style(Preset::Classic).grid, None);
        assert_eq!(frame_style(Preset::SignalFlow).grid, Some(GridStyle { dash: &[], opacity: 1.0 }));
        assert_eq!(frame_style(Preset::Editorial).region_dash, Some(&[9.0, 4.0][..]));
        let b = frame_style(Preset::Blueprint);
        assert_eq!((b.region_dash, b.lane_dash, b.security_group_dash), (Some(&[12.0, 4.0, 2.0, 4.0][..]), Some(&[7.0, 3.0][..]), Some(&[7.0, 3.0][..])));
        assert_eq!(b.badge_radius, 1.0);
        assert_eq!(frame_style(Preset::Editorial).badge_radius, 4.0);
    }

    #[test]
    fn the_palette_setting_picks_over_the_document() {
        use crate::model::common::VisualPreset as V;
        assert_eq!(Preset::pick("document", Some(V::Blueprint)), Preset::Blueprint);
        assert_eq!(Preset::pick("document", None), Preset::Classic);
        assert_eq!(Preset::pick("editorial", Some(V::Blueprint)), Preset::Editorial);
        assert_eq!(Preset::pick("classic", Some(V::Blueprint)), Preset::Classic);
        assert_eq!(Preset::pick("nonsense", Some(V::SignalFlow)), Preset::SignalFlow);
        for p in Preset::ALL {
            assert_eq!(Preset::parse(p.as_str()), Some(p));
            let s = serde_json::to_string(&p).unwrap();
            assert_eq!(s, format!("\"{}\"", p.as_str()));
        }
    }

    #[test]
    fn every_kind_has_distinct_strokes_per_mode() {
        for p in [&CLASSIC_DARK, &CLASSIC_LIGHT] {
            for (i, a) in p.kinds.iter().enumerate() {
                assert!(a.fill.a < 1.0, "kind fills are translucent");
                for b in &p.kinds[i + 1..] {
                    assert_ne!(a.stroke, b.stroke);
                }
            }
        }
    }

    #[test]
    fn lifecycle_types_map_to_kinds() {
        for (t, k) in [
            ("start", Kind::Frontend),
            ("active", Kind::Frontend),
            ("waiting", Kind::Cloud),
            ("decision", Kind::Database),
            ("success", Kind::Backend),
            ("failure", Kind::Security),
            ("neutral", Kind::External),
            ("external", Kind::External),
            ("messagebus", Kind::Messagebus),
            ("nonsense", Kind::External),
        ] {
            assert_eq!(Kind::from_state_type(t), k, "{t}");
        }
        assert_eq!(Kind::parse("cloud"), Some(Kind::Cloud));
        assert_eq!(Kind::parse("start"), None);
    }

    #[test]
    fn edge_variants() {
        assert_eq!(EdgeVariant::parse(Some("security")).dash(), &[5.0, 5.0]);
        assert_eq!(EdgeVariant::parse(Some("dashed")).stroke(), Token::KindStroke(Kind::Database));
        assert_eq!(EdgeVariant::parse(Some("return")).dash(), &[3.0, 5.0]);
        assert_eq!(EdgeVariant::parse(Some("return")).label(), Token::TextMuted);
        assert_eq!(EdgeVariant::parse(None), EdgeVariant::Default);
        assert!(EdgeVariant::Emphasis.dash().is_empty());
        assert_eq!(EdgeVariant::Emphasis.label(), Token::ArrowEmphasis);
    }

    #[test]
    fn tokens_round_trip_through_json() {
        for t in [Token::Mask, Token::KindFill(Kind::Cloud), Token::KindStroke(Kind::Messagebus)] {
            let s = serde_json::to_string(&t).unwrap();
            assert_eq!(serde_json::from_str::<Token>(&s).unwrap(), t, "{s}");
        }
    }
}
