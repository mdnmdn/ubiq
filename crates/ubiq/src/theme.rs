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

/// The terminal body: type size, the inset its output is drawn inside, and how many lines of
/// scrollback an emulator keeps.
pub const TERMINAL_FONT_SIZE: f32 = 13.0;
pub const TERMINAL_PADDING: f32 = 8.0;
pub const TERMINAL_SCROLLBACK: usize = 10_000;

/// The base point size a file editor draws its text at, and the anchor a project's zoom nudges.
/// The component library draws editors at the theme's mono size (13px); this is Ubiq's own name
/// for that floor so a per-project font size has a known start rather than an ever-tallied one.
pub const EDITOR_FONT_SIZE: f32 = 13.0;
/// The range a project's editor zoom is allowed to live in, in whole points.
pub const EDITOR_FONT_MIN: f32 = 8.0;
pub const EDITOR_FONT_MAX: f32 = 36.0;

/// Fixed chrome heights, in pixels.
pub const TITLEBAR_HEIGHT: f32 = 34.0;
pub const STATUS_BAR_HEIGHT: f32 = 30.0;
pub const RAIL_WIDTH: f32 = 56.0;

/// The size each of the dock's three edge regions opens at, in pixels. What a drag will not pass
/// is the dock's own, so a region is one number rather than a triple; what the user drags one to
/// is remembered inside the arrangement blob and is what a restored window opens on.
pub const EXPLORER_WIDTH: f32 = 300.0;
pub const CHAT_WIDTH: f32 = 420.0;
pub const DOCK_HEIGHT: f32 = 300.0;

/// The orchestration screen: the inspector beside the graph, the tasks drawer under it, and the
/// pitch of the graph's dotted ground at 100% zoom.
pub const INSPECTOR_WIDTH: f32 = 420.0;
pub const TASKS_HEIGHT: f32 = 220.0;
pub const GRAPH_DOT_PITCH: f32 = 28.0;

/// The agents screen: the sidebar listing every agent, and the strip that drops a dragged tab into
/// a column of its own. The columns themselves share the row and are sized by
/// `state::agents::COLUMN_MIN_WIDTH`, because how narrow a conversation may get is a fact about
/// the conversation rather than about this window.
pub const AGENT_SIDEBAR_WIDTH: f32 = 300.0;
pub const NEW_COLUMN_STRIP: f32 = 28.0;

/// A modal: how wide it is drawn, and the most of the window's height it may take before its body
/// scrolls inside it. A modal is one question, so it is one width rather than a per-caller size.
pub const MODAL_WIDTH: f32 = 460.0;
pub const MODAL_MAX_HEIGHT: f32 = 0.8;

/// The one modal that is not one question: a running harness login for a full-screen TUI
/// (`opencode auth login`, bare `grok`). Those measure the box they are given and redraw for it,
/// so a one-question width and a squeezed height is what garbles their output — see
/// `_docs/tech/ui-and-design.md`. `LOGIN_MODAL_WIDTH` and `LOGIN_MODAL_HEIGHT` size the panel
/// itself (through `kit::modal_sized`'s fill mode, not the scroll-and-hug a normal modal uses),
/// generous enough for a real TUI while still clamped to the viewport the same way
/// `MODAL_MAX_HEIGHT` clamps an ordinary modal. `LOGIN_MODAL_HEIGHT` leaves the terminal box
/// itself at roughly 30 rows once the note above it and the header/footer chrome are subtracted.
pub const LOGIN_MODAL_WIDTH: f32 = 960.0;
pub const LOGIN_MODAL_HEIGHT: f32 = 720.0;

/// Application settings: a page overlay with a nav, not a one-question modal. Fixed size so
/// switching sections does not resize the panel. Same width as project settings.
pub const SETTINGS_WIDTH: f32 = 820.0;
pub const SETTINGS_HEIGHT: f32 = 560.0;

/// The tasks board: a column's width open and shut, and the panel the selected task opens in.
pub const COLUMN_WIDTH: f32 = 320.0;
pub const COLUMN_SHUT: f32 = 44.0;
pub const TASK_PANEL_WIDTH: f32 = 420.0;

// ── Palette groups ──────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct SurfaceColors {
    pub app_bg: Rgba,
    pub pane_bg: Rgba,
    pub base: Rgba,
    pub raised: Rgba,
    pub hover: Rgba,
    pub selected: Rgba,
    /// What a modal lays over the window it took the keyboard from. Its own token rather than a
    /// `fade` at the call site, because the amount a palette has to dim by is not the same in both.
    pub scrim: Rgba,
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
    pub swatches: [Rgba; 16],
    /// The tint a temporary project takes instead of a swatch. Grey rather than a hue, because a
    /// folder that is not in the catalogue should not look like one that is.
    pub temporary: Rgba,
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

/// Colours the terminal emulator paints that are not ANSI — selection and links.
#[derive(Clone, Copy, Debug)]
pub struct TerminalColors {
    pub selection: Rgba,
    pub link_underline: Rgba,
    pub link_underline_hover: Rgba,
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
    pub terminal: TerminalColors,
}

// ── Theme ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

pub fn scrim() -> Rgba {
    Theme::current().palette.surface.scrim
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

pub fn selection_background() -> Rgba {
    Theme::current().palette.terminal.selection
}

pub fn link_underline() -> Rgba {
    Theme::current().palette.terminal.link_underline
}

pub fn link_underline_hover() -> Rgba {
    Theme::current().palette.terminal.link_underline_hover
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

/// The tint a temporary project is drawn in — one grey, not a swatch.
pub fn project_temporary() -> Rgba {
    Theme::current().palette.project.temporary
}

/// The tint a project is identified by, whichever kind it is.
///
/// One function so the grey is decided in one place: a temporary project ignores its stored colour
/// index, which the host never let it choose in the first place. `custom`, when set, wins over the
/// swatch — it is what a colour picked outside the swatch grid resolves to everywhere a project's
/// tint is drawn.
pub fn project_tint(temporary: bool, colour: usize, custom: Option<u32>) -> Rgba {
    if temporary {
        project_temporary()
    } else if let Some(rgb) = custom {
        rgba_of(rgb)
    } else {
        project_colour(colour)
    }
}

/// Unpack a colour picked outside the swatches, packed as `0x00RRGGBB`.
pub fn rgba_of(rgb: u32) -> Rgba {
    Rgba {
        r: ((rgb >> 16) & 0xff) as f32 / 255.0,
        g: ((rgb >> 8) & 0xff) as f32 / 255.0,
        b: (rgb & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

/// Whether a tint is dark enough that a white mark reads on it.
///
/// The mark is Ubiq's own logo drawn on the tint; a dark one takes the white file and a light one
/// the blue. It takes the resolved colour rather than a swatch index so the same answer covers a
/// temporary project's grey and the border a window with no project falls back to.
pub fn mark_dark(colour: Rgba) -> bool {
    relative_luminance(colour) < 0.5
}

/// WCAG relative luminance for a colour, so a swatch can say whether it is light or dark.
fn relative_luminance(colour: Rgba) -> f64 {
    let linear = |c: f32| {
        let c = c as f64;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(colour.r) + 0.7152 * linear(colour.g) + 0.0722 * linear(colour.b)
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
                scrim: rgba_hex_a(0x05050a, 0.62),
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
                    // The ten added later, appended rather than interleaved — `colour` is a stored
                    // index, so reordering the first six would recolour every existing project.
                    rgba_hex(0xe0555a), // red
                    rgba_hex(0xe08245), // orange
                    rgba_hex(0x7fbf3f), // lime
                    rgba_hex(0x2fb0c7), // cyan
                    rgba_hex(0x6c78e0), // indigo
                    rgba_hex(0xd94ad4), // magenta
                    rgba_hex(0xb07a52), // brown
                    rgba_hex(0x8a94a8), // slate
                    rgba_hex(0xaaa347), // olive
                    rgba_hex(0xe0496e), // rose
                ],
                temporary: rgba_hex(0x6e7681),
            },
            terminal: TerminalColors {
                selection: rgba_hex(0x1c2a44),
                link_underline: rgba_hex(0x5b8def),
                link_underline_hover: rgba_hex(0x7aa6f5),
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
                scrim: rgba_hex_a(0x1a1a2e, 0.38),
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
                    // Same ten, deeper and more saturated to match this palette's relationship to
                    // the dark one above.
                    rgba_hex(0xc23438), // red
                    rgba_hex(0xc4671f), // orange
                    rgba_hex(0x679c2b), // lime
                    rgba_hex(0x1e8ca0), // cyan
                    rgba_hex(0x4a54c4), // indigo
                    rgba_hex(0xb82eb2), // magenta
                    rgba_hex(0x8f5c38), // brown
                    rgba_hex(0x687487), // slate
                    rgba_hex(0x8a8329), // olive
                    rgba_hex(0xc22e4f), // rose
                ],
                temporary: rgba_hex(0x8b929c),
            },
            terminal: TerminalColors {
                selection: rgba_hex(0xd4e4ff),
                link_underline: rgba_hex(0x3b6fd4),
                link_underline_hover: rgba_hex(0x5a8ad4),
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

/// The same colour at another alpha.
///
/// Not a colour of its own: a token stays the token it was, and this is how something that has to
/// sit under, over or beside a surface — a glow, a fading grain of sand, a connector — borrows one
/// without inventing a shade.
pub fn fade(colour: Rgba, alpha: f32) -> Rgba {
    Rgba {
        a: colour.a * alpha.clamp(0.0, 1.0),
        ..colour
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
