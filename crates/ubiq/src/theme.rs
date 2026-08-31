//! The colour palette, its tokens, and the design constants the shell is laid out with.
//!
//! Every colour in the UI comes from an accessor here. A literal colour anywhere else is a defect
//! — see `_docs/tech/ui-and-design.md`, which owns the token set.

use gpui::{App, Rgba};
use std::cell::RefCell;

// ── Design constants ────────────────────────────────────────────────

/// The monospace family used for code, terminal chrome and mono labels.
pub const MONO_FONT: &str = "Menlo";

/// The width of the coloured edge that marks a surface. Ubiq's surfaces are square; the left
/// border is what identifies them.
pub const ACCENT_EDGE: f32 = 2.0;

/// Fixed chrome heights, in pixels.
pub const TITLEBAR_HEIGHT: f32 = 44.0;
pub const STATUS_BAR_HEIGHT: f32 = 30.0;
pub const RAIL_WIDTH: f32 = 72.0;

/// Default and permitted sizes for the three resizable panels, in pixels.
pub const EXPLORER_WIDTH: f32 = 300.0;
pub const EXPLORER_MIN: f32 = 200.0;
pub const EXPLORER_MAX: f32 = 480.0;
pub const CHAT_WIDTH: f32 = 420.0;
pub const CHAT_MIN: f32 = 320.0;
pub const CHAT_MAX: f32 = 640.0;
pub const DOCK_HEIGHT: f32 = 300.0;
pub const DOCK_MIN: f32 = 120.0;
pub const DOCK_MAX: f32 = 600.0;

// ── Palette groups ──────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct SurfaceColors {
    pub app_bg: Rgba,
    pub pane_bg: Rgba,
    pub base: Rgba,
    pub raised: Rgba,
    pub hover: Rgba,
    pub selected: Rgba,
}

#[derive(Clone, Copy, Debug)]
pub struct TextColors {
    pub primary: Rgba,
    pub muted: Rgba,
    pub faint: Rgba,
    pub on_accent: Rgba,
}

#[derive(Clone, Copy, Debug)]
pub struct AccentColors {
    pub primary: Rgba,
    pub muted: Rgba,
    pub soft: Rgba,
}

#[derive(Clone, Copy, Debug)]
pub struct BorderColors {
    pub default: Rgba,
    pub focus: Rgba,
}

/// The swatches projects are identified by. A project keeps the same one wherever it appears — its
/// dot in the picker, the fill behind its name, the mark, and the window's left edge.
#[derive(Clone, Copy, Debug)]
pub struct ProjectColors {
    pub swatches: [Rgba; 6],
}

#[derive(Clone, Copy, Debug)]
pub struct StatusColors {
    pub danger: Rgba,
    pub danger_soft: Rgba,
    pub success: Rgba,
    pub success_soft: Rgba,
    pub warning: Rgba,
    pub warning_soft: Rgba,
    pub info: Rgba,
    pub info_soft: Rgba,
}

// ── Palette ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub surface: SurfaceColors,
    pub text: TextColors,
    pub accent: AccentColors,
    pub border: BorderColors,
    pub status: StatusColors,
    pub project: ProjectColors,
}

// ── Theme ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeId {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub palette: Palette,
    pub id: ThemeId,
}

impl Theme {
    pub fn current() -> Theme {
        CURRENT.with(|c| *c.borrow())
    }

    pub fn set(theme: Theme) {
        CURRENT.with(|c| *c.borrow_mut() = theme);
    }

    pub fn is_dark(&self) -> bool {
        self.id == ThemeId::Dark
    }
}

impl ThemeId {
    pub fn all() -> &'static [ThemeId] {
        &[ThemeId::Dark, ThemeId::Light]
    }

    /// The other palette — what a theme toggle switches to.
    pub fn toggled(self) -> ThemeId {
        match self {
            ThemeId::Dark => ThemeId::Light,
            ThemeId::Light => ThemeId::Dark,
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Theme> = RefCell::new(dark());
}

/// Switch palettes.
///
/// Sets Ubiq's own tokens *and* the component library's theme, so the editor, the textarea and the
/// markdown view follow the rest of the window instead of staying in whichever mode they booted in.
pub fn set_mode(id: ThemeId, cx: &mut App) {
    Theme::set(palette_for(id));
    let mode = match id {
        ThemeId::Dark => gpui_component::ThemeMode::Dark,
        ThemeId::Light => gpui_component::ThemeMode::Light,
    };
    gpui_component::Theme::change(mode, None, cx);
}

// ── Accessor functions ──────────────────────────────────────────────

pub fn app_bg() -> Rgba {
    Theme::current().palette.surface.app_bg
}

pub fn pane_bg() -> Rgba {
    Theme::current().palette.surface.pane_bg
}

pub fn surface() -> Rgba {
    Theme::current().palette.surface.base
}

pub fn surface_raised() -> Rgba {
    Theme::current().palette.surface.raised
}

pub fn hover() -> Rgba {
    Theme::current().palette.surface.hover
}

pub fn selected() -> Rgba {
    Theme::current().palette.surface.selected
}

pub fn text() -> Rgba {
    Theme::current().palette.text.primary
}

pub fn text_muted() -> Rgba {
    Theme::current().palette.text.muted
}

pub fn text_faint() -> Rgba {
    Theme::current().palette.text.faint
}

pub fn on_accent() -> Rgba {
    Theme::current().palette.text.on_accent
}

pub fn accent() -> Rgba {
    Theme::current().palette.accent.primary
}

pub fn accent_muted() -> Rgba {
    Theme::current().palette.accent.muted
}

pub fn accent_soft() -> Rgba {
    Theme::current().palette.accent.soft
}

pub fn border() -> Rgba {
    Theme::current().palette.border.default
}

pub fn border_focus() -> Rgba {
    Theme::current().palette.border.focus
}

pub fn danger() -> Rgba {
    Theme::current().palette.status.danger
}

pub fn danger_soft() -> Rgba {
    Theme::current().palette.status.danger_soft
}

pub fn success() -> Rgba {
    Theme::current().palette.status.success
}

pub fn success_soft() -> Rgba {
    Theme::current().palette.status.success_soft
}

pub fn warning() -> Rgba {
    Theme::current().palette.status.warning
}

pub fn warning_soft() -> Rgba {
    Theme::current().palette.status.warning_soft
}

pub fn info() -> Rgba {
    Theme::current().palette.status.info
}

pub fn info_soft() -> Rgba {
    Theme::current().palette.status.info_soft
}

/// The swatch a project is identified by. Wraps, so any number of projects is colourable.
pub fn project_colour(index: usize) -> Rgba {
    let swatches = Theme::current().palette.project.swatches;
    swatches[index % swatches.len()]
}

/// The number of distinct project swatches before they repeat.
pub fn project_colour_count() -> usize {
    Theme::current().palette.project.swatches.len()
}

// ── Built-in palettes ──────────────────────────────────────────────

pub fn dark() -> Theme {
    Theme {
        id: ThemeId::Dark,
        palette: Palette {
            surface: SurfaceColors {
                app_bg: rgba_hex(0x0d0d11),
                pane_bg: rgba_hex(0x121216),
                base: rgba_hex(0x1c1c22),
                raised: rgba_hex(0x25252c),
                hover: rgba_hex(0x2a2a33),
                selected: rgba_hex(0x1c2a44),
            },
            text: TextColors {
                primary: rgba_hex(0xe8e8ed),
                muted: rgba_hex(0x8f8f9a),
                faint: rgba_hex(0x5c5c68),
                on_accent: rgba_hex(0xffffff),
            },
            accent: AccentColors {
                primary: rgba_hex(0x5b8def),
                muted: rgba_hex(0x3d5f9e),
                soft: rgba_hex_a(0x5b8def, 0.16),
            },
            border: BorderColors {
                default: rgba_hex(0x2c2c34),
                focus: rgba_hex(0x5b8def),
            },
            status: StatusColors {
                danger: rgba_hex(0xe5484d),
                danger_soft: rgba_hex_a(0xe5484d, 0.16),
                success: rgba_hex(0x46a758),
                success_soft: rgba_hex_a(0x46a758, 0.16),
                warning: rgba_hex(0xef9f2a),
                warning_soft: rgba_hex_a(0xef9f2a, 0.16),
                info: rgba_hex(0x4a9eff),
                info_soft: rgba_hex_a(0x4a9eff, 0.16),
            },
            project: ProjectColors {
                swatches: [
                    rgba_hex(0x5b8def),
                    rgba_hex(0x9b7cf0),
                    rgba_hex(0x3fbfa8),
                    rgba_hex(0xe0a94a),
                    rgba_hex(0xe06c8a),
                    rgba_hex(0x6fbf5b),
                ],
            },
        },
    }
}

pub fn light() -> Theme {
    Theme {
        id: ThemeId::Light,
        palette: Palette {
            surface: SurfaceColors {
                app_bg: rgba_hex(0xfafafc),
                pane_bg: rgba_hex(0xf8f8fa),
                base: rgba_hex(0xffffff),
                raised: rgba_hex(0xf0f0f4),
                hover: rgba_hex(0xe8e8ec),
                selected: rgba_hex(0xd4e4ff),
            },
            text: TextColors {
                primary: rgba_hex(0x1a1a2e),
                muted: rgba_hex(0x6b6b80),
                faint: rgba_hex(0x9a9aac),
                on_accent: rgba_hex(0xffffff),
            },
            accent: AccentColors {
                primary: rgba_hex(0x3b6fd4),
                muted: rgba_hex(0x5a8ad4),
                soft: rgba_hex_a(0x3b6fd4, 0.12),
            },
            border: BorderColors {
                default: rgba_hex(0xd4d4dc),
                focus: rgba_hex(0x3b6fd4),
            },
            status: StatusColors {
                danger: rgba_hex(0xd1353b),
                danger_soft: rgba_hex_a(0xd1353b, 0.12),
                success: rgba_hex(0x3a9148),
                success_soft: rgba_hex_a(0x3a9148, 0.12),
                warning: rgba_hex(0xd48a1e),
                warning_soft: rgba_hex_a(0xd48a1e, 0.14),
                info: rgba_hex(0x0066ff),
                info_soft: rgba_hex_a(0x0066ff, 0.10),
            },
            project: ProjectColors {
                swatches: [
                    rgba_hex(0x3b6fd4),
                    rgba_hex(0x7a55d8),
                    rgba_hex(0x2a9d8a),
                    rgba_hex(0xc4881f),
                    rgba_hex(0xc44f6d),
                    rgba_hex(0x4e9b3c),
                ],
            },
        },
    }
}

pub fn palette_for(id: ThemeId) -> Theme {
    match id {
        ThemeId::Dark => dark(),
        ThemeId::Light => light(),
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

const fn rgba_hex(hex: u32) -> Rgba {
    rgba_hex_a(hex, 1.0)
}

const fn rgba_hex_a(hex: u32, a: f32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a,
    }
}
