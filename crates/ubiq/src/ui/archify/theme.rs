//! The theme adapter (P2.7, D11): a scene [`Token`] to a GPUI colour.
//!
//! Chrome tokens (ground, text tiers, lanes, the default edge) come from the active Ubiq palette
//! through `crate::theme`, so a diagram sits in whichever palette is on. The kind fills and strokes,
//! the region fill and the brand plate are *data* from `ubiq_archify::tokens::classic`, converted
//! here at runtime; they are semantic series colours and do not follow the palette, only its
//! light/dark axis ([`mode`]). No colour literal lives in this file.
//!
//! **Presets (P9.1, D38).** `classic` is the above. `signal-flow`, `blueprint` and `editorial` are
//! Archify's own tables ([`tokens::palette`]) and take the whole picture: ground, text tiers,
//! lanes, edges, masks and kinds. A preset is an identity (the blueprint's blue ground and grid, the
//! editorial paper), so it does not borrow the Ubiq chrome; only light/dark still follows the
//! active Ubiq palette. The preset in force is the scene's ([`set_preset`], called by the viewer
//! before it paints a scene).
//!
//! **Chart theme.** The diagram has its own light/dark choice ([`ChartTheme`]: Auto follows Ubiq,
//! Dark and Light pin it) that never touches Ubiq's theme. [`chart_mode`] is the axis every paint
//! resolves on; for the classic preset's Ubiq chrome tokens the opposite ground is taken from the
//! active theme's counterpart palette. [`chart_palette`] is the entry point for anything outside
//! paint (an export) that needs the active chart theme's colours.
//!
//! Resolved per frame: a palette switch redraws the next frame ([`begin_frame`] drops the one
//! cached chrome palette).

use std::cell::{Cell, RefCell};

use ubiq_archify::tokens::{self, Mode, Preset, Token};
use gpui::Rgba;
use crate::theme as ubiq_theme;

thread_local! {
    /// The preset the next paint resolves in. GPUI renders and paints on the main thread, in turn.
    static ACTIVE: Cell<Preset> = const { Cell::new(Preset::Classic) };
    /// The app-wide chart theme.
    static CHART: Cell<ChartTheme> = const { Cell::new(ChartTheme::Auto) };
    /// The Ubiq palette the classic chrome tokens read, for one mode, for one frame.
    static CHROME: RefCell<Option<(Mode, ubiq_theme::Theme)>> = const { RefCell::new(None) };
}

/// How the diagram canvas picks light or dark, independent of the Ubiq theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartTheme {
    /// Follow Ubiq's own theme.
    Auto,
    Dark,
    Light,
}

impl ChartTheme {
    pub const ALL: [ChartTheme; 3] = [ChartTheme::Auto, ChartTheme::Dark, ChartTheme::Light];

    pub fn label(self) -> &'static str {
        match self {
            ChartTheme::Auto => "Auto",
            ChartTheme::Dark => "Dark",
            ChartTheme::Light => "Light",
        }
    }

    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            ChartTheme::Auto => "auto",
            ChartTheme::Dark => "dark",
            ChartTheme::Light => "light",
        }
    }

    /// Anything unknown is `Auto`.
    pub fn parse(s: &str) -> ChartTheme {
        Self::ALL.into_iter().find(|t| t.as_str() == s).unwrap_or(ChartTheme::Auto)
    }
}

/// The chart theme in force.
pub fn chart_theme() -> ChartTheme {
    CHART.with(Cell::get)
}

/// Choose the chart theme (app-wide, this thread).
pub fn set_chart_theme(theme: ChartTheme) {
    CHART.with(|c| c.set(theme));
    begin_frame();
}

/// Drop the per-frame cache; the viewer calls it before it paints a scene.
pub fn begin_frame() {
    CHROME.with(|c| *c.borrow_mut() = None);
}

/// The ground the diagram is painted on: the chart theme, or Ubiq's when Auto.
pub fn chart_mode() -> Mode {
    match chart_theme() {
        ChartTheme::Auto => mode(),
        ChartTheme::Dark => Mode::Dark,
        ChartTheme::Light => Mode::Light,
    }
}

/// The colours of the active chart theme, for code outside paint (the export). `resolve` is
/// [`resolve_in`] at [`chart_mode`] in `preset`; use the scene's own preset.
#[derive(Clone, Copy, Debug)]
pub struct ChartPalette {
    pub mode: Mode,
    pub preset: Preset,
}

impl ChartPalette {
    pub fn resolve(&self, token: Token) -> Rgba {
        resolve_in(token, self.mode, self.preset)
    }
}

/// A GPUI colour as CSS: `#rrggbb` when opaque, `rgba(r, g, b, a)` otherwise (the SVG export).
pub fn css(c: Rgba) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if c.a >= 1.0 {
        format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
    } else {
        format!("rgba({}, {}, {}, {})", byte(c.r), byte(c.g), byte(c.b), c.a)
    }
}

/// The active chart theme's palette for a diagram in `preset`.
pub fn chart_palette(preset: Preset) -> ChartPalette {
    ChartPalette { mode: chart_mode(), preset }
}

/// The Ubiq palette the classic chrome tokens read in `mode`: the current theme, or its
/// counterpart (same family, other ground) when the chart theme pins the other one.
fn chrome(mode: Mode) -> ubiq_theme::Theme {
    let current = ubiq_theme::Theme::current();
    if current.is_dark() == (mode == Mode::Dark) {
        return current;
    }
    CHROME.with(|c| {
        let mut c = c.borrow_mut();
        match *c {
            Some((m, t)) if m == mode => t,
            _ => {
                let t = ubiq_theme::resolve(current.id.counterpart(), current.accent);
                *c = Some((mode, t));
                t
            }
        }
    })
}

/// Make `preset` the one [`rgba`] and [`resolve`] answer in, on this thread.
pub fn set_preset(preset: Preset) {
    ACTIVE.with(|a| a.set(preset));
}

/// The preset in force.
pub fn preset() -> Preset {
    ACTIVE.with(Cell::get)
}

/// Light or dark, from the active Ubiq palette.
pub fn mode() -> Mode {
    if ubiq_theme::Theme::current().is_dark() {
        Mode::Dark
    } else {
        Mode::Light
    }
}

/// The classic palette's colour as a GPUI one (straight alpha).
fn data(c: tokens::Rgba) -> Rgba {
    Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: c.a as f32,
    }
}

/// `token` in the active palette.
pub fn rgba(token: Token) -> Rgba {
    resolve(token, chart_mode())
}

/// `token` in `mode`. Chrome follows the Ubiq palette of that mode; the rest is core data for `mode`.
///
/// `Mask` is the opaque plate under node fills, edge labels, titles and crossover halos. It is the
/// ground itself (`pane_bg`), not a second shade: a halo has to vanish into whatever the edge
/// crosses, and a node's translucent kind fill tints over the ground.
pub fn resolve(token: Token, mode: Mode) -> Rgba {
    resolve_in(token, mode, preset())
}

/// [`resolve`] in `preset`. A non-classic preset is its own table; classic is the Ubiq chrome over
/// the classic kind data.
pub fn resolve_in(token: Token, mode: Mode, preset: Preset) -> Rgba {
    if !preset.is_classic() {
        return data(tokens::palette(preset, mode).resolve(token));
    }
    let palette = tokens::classic(mode);
    let p = chrome(mode).palette;
    match token {
        Token::Bg | Token::Mask => p.surface.pane_bg,
        Token::Grid => p.border.default,
        Token::CanvasDot => ubiq_theme::fade(p.text.faint, 0.35),
        Token::Text => p.text.primary,
        Token::TextMuted => p.text.muted,
        Token::TextDim => p.text.faint,
        Token::TextFaint => ubiq_theme::fade(p.text.faint, 0.7),
        Token::Panel => p.surface.raised,
        Token::PanelBorder | Token::LaneStroke => p.border.default,
        Token::LaneFill => ubiq_theme::fade(p.surface.raised, 0.5),
        Token::Arrow => p.text.faint,
        Token::ArrowEmphasis => p.accent.primary,
        Token::KindFill(kind) => data(palette.resolve(Token::KindFill(kind))),
        Token::KindStroke(kind) => data(palette.resolve(Token::KindStroke(kind))),
        Token::RegionFill => data(palette.resolve(Token::RegionFill)),
        Token::BrandBadge => data(palette.resolve(Token::BrandBadge)),
        Token::BrandFrame => data(palette.resolve(Token::BrandFrame)),
        Token::BrandGlyph => data(palette.resolve(Token::BrandGlyph)),
        Token::BrandInk(hex) => data(palette.resolve(Token::BrandInk(hex))),
    }
}

/// `token` with an extra alpha multiplier (`Fill::alpha`, a shape's opacity).
pub fn rgba_alpha(token: Token, alpha: f64) -> Rgba {
    ubiq_theme::fade(rgba(token), alpha as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_archify::tokens::Kind;

    /// Every variant of [`Token`]. `resolve` matches exhaustively, so a new variant fails to
    /// compile there; this list is what the test below walks.
    fn all_tokens() -> Vec<Token> {
        let mut all = vec![
            Token::Bg,
            Token::Grid,
            Token::CanvasDot,
            Token::Text,
            Token::TextMuted,
            Token::TextDim,
            Token::TextFaint,
            Token::Panel,
            Token::PanelBorder,
            Token::LaneFill,
            Token::LaneStroke,
            Token::Arrow,
            Token::ArrowEmphasis,
            Token::Mask,
            Token::RegionFill,
            Token::BrandBadge,
            Token::BrandFrame,
            Token::BrandGlyph,
            Token::BrandInk(0x18bfff),
        ];
        for kind in Kind::ALL {
            all.push(Token::KindFill(kind));
            all.push(Token::KindStroke(kind));
        }
        all
    }

    fn valid(c: Rgba) -> bool {
        [c.r, c.g, c.b, c.a]
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }

    #[test]
    fn every_token_maps_in_both_modes_and_every_preset() {
        assert_eq!(all_tokens().len(), 19 + 14);
        for preset in Preset::ALL {
            for mode in [Mode::Dark, Mode::Light] {
                for token in all_tokens() {
                    let c = resolve_in(token, mode, preset);
                    assert!(valid(c), "{token:?} in {mode:?}/{preset:?} gave {c:?}");
                }
            }
        }
    }

    #[test]
    fn a_preset_is_its_own_table_and_classic_stays_the_chrome() {
        let ground = |p| resolve_in(Token::Mask, Mode::Dark, p);
        assert_eq!(ground(Preset::Blueprint), data(tokens::BLUEPRINT_DARK.mask));
        assert_eq!(ground(Preset::Editorial), data(tokens::EDITORIAL_DARK.mask));
        assert_ne!(ground(Preset::Blueprint), ground(Preset::Editorial));
        let text = |p| resolve_in(Token::Text, Mode::Light, p);
        assert_eq!(text(Preset::SignalFlow), data(tokens::SIGNAL_FLOW_LIGHT.text));
        // Classic takes the Ubiq ground, whatever the table says.
        assert_eq!(resolve_in(Token::Bg, Mode::Dark, Preset::Classic), ubiq_theme::pane_bg());
        // The brand ink is the vendor's colour in every preset.
        let ink = Token::BrandInk(0x18bfff);
        assert_eq!(resolve_in(ink, Mode::Dark, Preset::Blueprint), resolve_in(ink, Mode::Dark, Preset::Classic));
    }

    #[test]
    fn the_chart_theme_pins_the_ground_and_round_trips() {
        for t in ChartTheme::ALL {
            assert_eq!(ChartTheme::parse(t.as_str()), t);
        }
        assert_eq!(ChartTheme::parse("nonsense"), ChartTheme::Auto);
        set_chart_theme(ChartTheme::Light);
        assert_eq!(chart_mode(), Mode::Light);
        set_chart_theme(ChartTheme::Dark);
        assert_eq!(chart_mode(), Mode::Dark);
        let p = chart_palette(Preset::Classic);
        assert_eq!(p.resolve(Token::Bg), resolve_in(Token::Bg, Mode::Dark, Preset::Classic));
        set_chart_theme(ChartTheme::Auto);
    }

    #[test]
    fn the_preset_in_force_round_trips() {
        for p in Preset::ALL {
            set_preset(p);
            assert_eq!(preset(), p);
        }
        set_preset(Preset::Classic);
    }

    #[test]
    fn kind_colours_are_the_core_data_and_follow_the_mode() {
        for kind in Kind::ALL {
            let dark = resolve(Token::KindStroke(kind), Mode::Dark);
            let light = resolve(Token::KindStroke(kind), Mode::Light);
            assert_ne!(dark, light, "{kind:?} is the same in both modes");
            assert_eq!(dark, data(tokens::CLASSIC_DARK.kind(kind).stroke));
            assert!(
                resolve(Token::KindFill(kind), Mode::Dark).a < 1.0,
                "kind fills are translucent"
            );
        }
    }

    #[test]
    fn the_core_to_gpui_conversion_keeps_channels_and_alpha() {
        let c = data(tokens::Rgba::new(255, 0, 51, 0.4));
        assert_eq!((c.r, c.g, c.a), (1.0, 0.0, 0.4));
        assert!((c.b - 0.2).abs() < 1e-6);
    }

    #[test]
    fn alpha_multiplies_rather_than_replaces() {
        let base = rgba(Token::KindFill(Kind::Cloud));
        let half = rgba_alpha(Token::KindFill(Kind::Cloud), 0.5);
        assert!((half.a - base.a * 0.5).abs() < 1e-6);
    }
}
