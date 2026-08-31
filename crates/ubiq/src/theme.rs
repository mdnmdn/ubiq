use gpui::Rgba;
use std::cell::RefCell;

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
    pub on_accent: Rgba,
}

#[derive(Clone, Copy, Debug)]
pub struct AccentColors {
    pub primary: Rgba,
    pub muted: Rgba,
}

#[derive(Clone, Copy, Debug)]
pub struct BorderColors {
    pub default: Rgba,
    pub focus: Rgba,
}

#[derive(Clone, Copy, Debug)]
pub struct StatusColors {
    pub danger: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub info: Rgba,
}

// ── Palette ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub surface: SurfaceColors,
    pub text: TextColors,
    pub accent: AccentColors,
    pub border: BorderColors,
    pub status: StatusColors,
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
        CURRENT.with(|c| c.borrow().clone())
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
}

thread_local! {
    static CURRENT: RefCell<Theme> = RefCell::new(dark());
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

pub fn on_accent() -> Rgba {
    Theme::current().palette.text.on_accent
}

pub fn accent() -> Rgba {
    Theme::current().palette.accent.primary
}

pub fn accent_muted() -> Rgba {
    Theme::current().palette.accent.muted
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

pub fn success() -> Rgba {
    Theme::current().palette.status.success
}

pub fn warning() -> Rgba {
    Theme::current().palette.status.warning
}

pub fn info() -> Rgba {
    Theme::current().palette.status.info
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
                on_accent: rgba_hex(0xffffff),
            },
            accent: AccentColors {
                primary: rgba_hex(0x5b8def),
                muted: rgba_hex(0x3d5f9e),
            },
            border: BorderColors {
                default: rgba_hex(0x2c2c34),
                focus: rgba_hex(0x5b8def),
            },
            status: StatusColors {
                danger: rgba_hex(0xe5484d),
                success: rgba_hex(0x46a758),
                warning: rgba_hex(0xef9f2a),
                info: rgba_hex(0x4a9eff),
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
                on_accent: rgba_hex(0xffffff),
            },
            accent: AccentColors {
                primary: rgba_hex(0x3b6fd4),
                muted: rgba_hex(0x5a8ad4),
            },
            border: BorderColors {
                default: rgba_hex(0xd4d4dc),
                focus: rgba_hex(0x3b6fd4),
            },
            status: StatusColors {
                danger: rgba_hex(0xd1353b),
                success: rgba_hex(0x3a9148),
                warning: rgba_hex(0xd48a1e),
                info: rgba_hex(0x0066ff),
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

// ── Helper ──────────────────────────────────────────────────────────

const fn rgba_hex(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}
