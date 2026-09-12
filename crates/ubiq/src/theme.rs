//! The colour palette, its tokens, and the design constants the shell is laid out with.
//!
//! Every colour in the UI comes from an accessor here. A literal colour anywhere else is a defect
//! — see `_docs/tech/ui-and-design.md`, which owns the token set.

use gpui::{App, Pixels, Rgba, px};
use std::cell::RefCell;

// ── Design constants ────────────────────────────────────────────────

/// The monospace family used for code, terminal chrome and mono labels: the mono that ships
/// with the OS, so the text system resolves it instead of falling back to a proportional face
/// (which renders unevenly and measures wrong per cell).
#[cfg(target_os = "macos")]
pub const MONO_FONT: &str = "Menlo";
#[cfg(target_os = "windows")]
pub const MONO_FONT: &str = "Cascadia Mono";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const MONO_FONT: &str = "DejaVu Sans Mono";

// The grid half of the table. These are the sizes density scales, so each is a function rather
// than a `pub const`: a factor cannot apply to a const. Everything below the "Dragged regions"
// heading stays a const and does not move — see the note there.

/// The width of the coloured edge that marks a surface. Ubiq's surfaces are square; the left
/// border is what identifies them.
const ACCENT_EDGE: f32 = 2.0;

pub fn accent_edge() -> f32 {
    scaled(ACCENT_EDGE)
}

/// The terminal body: type size, the inset its output is drawn inside, and how many lines of
/// scrollback an emulator keeps.
pub const TERMINAL_FONT_SIZE: f32 = 13.0;
const TERMINAL_PADDING: f32 = 8.0;
pub const TERMINAL_SCROLLBACK: usize = 10_000;

/// The inset a pane's output is drawn inside. Density reaches the pseudo-terminal through this:
/// the emulator re-measures its cell grid from the bounds minus the padding, and its resize
/// callback is what tells the harness.
pub fn terminal_padding() -> f32 {
    scaled(TERMINAL_PADDING)
}

/// The base point size a file editor draws its text at, and the anchor a project's zoom nudges.
/// The component library draws editors at the theme's mono size (13px); this is Ubiq's own name
/// for that floor so a per-project font size has a known start rather than an ever-tallied one.
pub const EDITOR_FONT_SIZE: f32 = 13.0;
/// The range a project's editor zoom is allowed to live in, in whole points.
pub const EDITOR_FONT_MIN: f32 = 8.0;
pub const EDITOR_FONT_MAX: f32 = 36.0;

/// Chrome heights, in pixels, at `Density::Regular`.
const TITLEBAR_HEIGHT: f32 = 34.0;
const STATUS_BAR_HEIGHT: f32 = 30.0;
const RAIL_WIDTH: f32 = 56.0;

pub fn titlebar_height() -> f32 {
    scaled(TITLEBAR_HEIGHT)
}

pub fn status_bar_height() -> f32 {
    scaled(STATUS_BAR_HEIGHT)
}

pub fn rail_width() -> f32 {
    scaled(RAIL_WIDTH)
}

// ── Dragged regions ─────────────────────────────────────────────────
//
// Density does **not** touch anything from here to the end of the table. These are what a *fresh*
// window opens at; what the user then drags is remembered per project inside the arrangement blob,
// so scaling them would fight a value the user already set.

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

/// The start control on an empty chat panel: about three times a chrome `kit::icon_button`,
/// because it is the page's whole subject rather than one control among a row of them.
pub const EMPTY_START_SIZE: f32 = 90.0;
pub const EMPTY_START_ICON: f32 = 48.0;

/// An A2UI surface's two sizes. The catalog carries no width, no height and no padding — it names
/// a discrete `variant` per component and leaves the measurements to the renderer — so these are
/// Ubiq's answer to "how big is a `smallFeature` image", and the only sizes a drawn surface has.
/// `A2UI_SVG_MAX` is the box an agent-authored picture is fitted into, aspect preserved.
pub const A2UI_IMAGE_ICON: f32 = 20.0;
pub const A2UI_IMAGE_AVATAR: f32 = 32.0;
pub const A2UI_IMAGE_SMALL: f32 = 96.0;
pub const A2UI_IMAGE_MEDIUM: f32 = 160.0;
pub const A2UI_IMAGE_LARGE: f32 = 240.0;
pub const A2UI_IMAGE_HEADER_H: f32 = 120.0;
pub const A2UI_SVG_MAX: f32 = 320.0;

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
    /// What the cursor bar looks like once the list itself holds the keyboard. The same colour as
    /// `selected`, a shade deeper: which row the next key lands on has to read at a glance from
    /// the row a list merely left its cursor on when the keyboard went elsewhere.
    pub selected_focus: Rgba,
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

/// The six hued tokens, all of them derived from one seed by [`with_accent`].
///
/// `primary` as written in a palette definition **is** that palette's seed — the one hue it
/// declares. `muted` and `soft`, and `border.focus` and the two `terminal.link_underline*`
/// alongside them, are consequences of it and are overwritten when the theme resolves, so what a
/// definition writes there is documentation of the default rather than a seventh decision.
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
    /// The build-channel ribbon in the window's bottom-left corner: its two bands, and the ink
    /// its word is written in. Fixed in both palettes — the ribbon marks the build, not the mood.
    pub ribbon_alpha: Rgba,
    pub ribbon_beta: Rgba,
    pub ribbon_ink: Rgba,
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

/// Which ground a palette is. What the component library and the syntax highlighter are told —
/// they only know these two — and the axis the titlebar's one control flips along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

/// How tight the grid is drawn: one factor over the grid constants, and nothing else.
///
/// It is resolved into the [`Theme`] like the palette and the accent are, so a call site reads a
/// scaled size the same way it reads a colour and no call site computes a factor of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Density {
    Compact,
    #[default]
    Regular,
    Comfortable,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Compact, Density::Regular, Density::Comfortable];

    pub fn factor(self) -> f32 {
        match self {
            Density::Compact => 0.9,
            Density::Regular => 1.0,
            Density::Comfortable => 1.15,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Regular => "Regular",
            Density::Comfortable => "Comfortable",
        }
    }
}

/// The current density.
pub fn density() -> Density {
    Theme::current().density
}

/// A grid constant at the current density, rounded to whole pixels — a chrome row that is a
/// fraction of a pixel tall is a seam in the rule under it.
fn scaled(base: f32) -> f32 {
    (base * Theme::current().density.factor()).round()
}

// ── The type scale ──────────────────────────────────────────────────
//
// Every type size in the UI comes from [`font`]. A `text_size(px(N))` anywhere else is the same
// defect a literal colour is: this is the one file allowed to name a size.

/// Which surface family a size belongs to.
///
/// Three, each with a base size of its own, each set independently — the three are read
/// differently and resize for different reasons. A fourth would be arguing about where a boundary
/// falls; these three fall on boundaries the code already draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Titlebar, status bar, rail, tabs, menus, modals, settings, pickers, notifications,
    /// dialogs. Furniture: growing it reflows the window, so it moves on its own and rarely.
    Chrome,
    /// Editor, viewer, explorer tree, search results, terminal panes. **Per project** — a zoom
    /// travels with the project it was chosen for, and the panes are sized in whole points
    /// because a terminal is a cell grid.
    Content,
    /// The chat transcript, tool blocks, the composer, the agents columns and their sidebar. Read
    /// as prose, at a size that has nothing to do with the size code is read at.
    Conversation,
}

/// A named step on a family's scale, as a ratio over its base.
///
/// Five, mapped onto the size clusters already in the tree, so a call site names what a run *is*
/// rather than how many points it happens to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A page or section heading.
    Title,
    /// The family's base — running text.
    Body,
    /// A control's label, a row, a tab.
    Label,
    /// A timestamp, a count, a hint.
    Meta,
    /// A badge, a chip, a superscript.
    Micro,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::Title,
        Role::Body,
        Role::Label,
        Role::Meta,
        Role::Micro,
    ];

    pub fn ratio(self) -> f32 {
        match self {
            Role::Title => 1.15,
            Role::Body => 1.00,
            Role::Label => 0.92,
            Role::Meta => 0.85,
            Role::Micro => 0.80,
        }
    }
}

/// What [`Family::Chrome`] and [`Family::Conversation`] draw [`Role::Body`] at. Interface-scoped:
/// both are written into `InterfacePrefs`. `Family::Content`'s base is [`EDITOR_FONT_SIZE`] and is
/// the project's, which is why it is not a constant pair with these.
pub const CHROME_FONT_SIZE: f32 = 12.5;
pub const CONVERSATION_FONT_SIZE: f32 = 12.5;

/// A one-off display size: the device-login user code, which is a number to be read off a screen
/// and typed into a phone rather than a heading. It is named here rather than left as a literal at
/// the call site for the same reason a colour is.
const DISPLAY_FONT_SIZE: f32 = 22.0;

pub fn font_display() -> Pixels {
    px(DISPLAY_FONT_SIZE)
}

/// The three base sizes, one per family — the whole of the text axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextScale {
    pub chrome: f32,
    pub content: f32,
    pub conversation: f32,
}

impl Default for TextScale {
    fn default() -> Self {
        Self {
            chrome: CHROME_FONT_SIZE,
            content: EDITOR_FONT_SIZE,
            conversation: CONVERSATION_FONT_SIZE,
        }
    }
}

impl TextScale {
    pub fn base(&self, family: Family) -> f32 {
        match family {
            Family::Chrome => self.chrome,
            Family::Content => self.content,
            Family::Conversation => self.conversation,
        }
    }
}

thread_local! {
    /// The text axis. Its own cell rather than a field on the resolved [`Theme`], because it is
    /// the one axis with nothing to resolve: a base size is a number the user set, not something
    /// derived from the palette in hand. Keeping it here is also what stops switching palette,
    /// accent or density from undoing it — there is no resolution for it to be dropped by.
    // Already `const`: allowed because the lint fires on this toolchain regardless.
    #[allow(clippy::missing_const_for_thread_local)]
    static TEXT: std::cell::Cell<TextScale> = const {
        std::cell::Cell::new(TextScale {
            chrome: CHROME_FONT_SIZE,
            content: EDITOR_FONT_SIZE,
            conversation: CONVERSATION_FONT_SIZE,
        })
    };
}

/// The size one family draws one role at.
///
/// Rounded to the nearest half point, which is the grid the hand-picked sizes this replaced were
/// already on — a run at a third of a point renders unevenly for no gain.
pub fn font(family: Family, role: Role) -> Pixels {
    let base = text_scale().base(family);
    px((base * role.ratio() * 2.0).round() / 2.0)
}

/// The three bases as they stand.
pub fn text_scale() -> TextScale {
    TEXT.get()
}

/// Set the three bases, leaving the palette, the accent and the density as they stand.
pub fn set_text_scale(scale: TextScale) {
    TEXT.set(scale);
}

/// A palette in the registry: its key, what to call it, its ground, the slug of the palette the
/// theme toggle reaches from it, and the colours themselves.
///
/// The tokens are one `Palette` value rather than a builder, which is what keeps a definition
/// complete by construction — a family cannot ship a palette missing a token.
pub struct PaletteDef {
    pub slug: &'static str,
    pub name: &'static str,
    pub mode: Mode,
    /// The same family's other ground. A warm dark toggles to the warm light, not to the built-in
    /// one.
    pub counterpart: &'static str,
    pub palette: Palette,
}

/// A palette, named by its slug.
///
/// Not an enum: palettes are registry rows, so a family is an entry in [`PALETTES`] rather than a
/// variant and a match arm in every file that reads one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeId(pub &'static str);

/// An accent in the registry: its key, what to call it, and the one hue it is.
///
/// One `Rgba`, because every hued token derives from it — see [`with_accent`]. An accent is an
/// axis of its own, so N palettes and M accents cost N + M declarations.
pub struct AccentDef {
    pub slug: &'static str,
    pub name: &'static str,
    pub seed: Rgba,
}

/// An accent, named by its slug. `None` anywhere one is optional means the palette's own seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccentId(pub &'static str);

impl AccentId {
    /// Every accent the build ships, in registry order.
    pub fn all() -> impl Iterator<Item = AccentId> {
        ACCENTS.iter().map(|a| AccentId(a.slug))
    }

    pub fn slug(self) -> &'static str {
        self.def().slug
    }

    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn seed(self) -> Rgba {
        self.def().seed
    }

    /// The registry row, falling back to the first entry so an id from anywhere still resolves.
    fn def(self) -> &'static AccentDef {
        ACCENTS
            .iter()
            .find(|a| a.slug == self.0)
            .unwrap_or(&ACCENTS[0])
    }

    /// A slug read from outside the build; unknown falls back to the first entry rather than
    /// failing the whole blob, the same posture [`ThemeId::from_slug`] takes.
    fn from_slug(s: &str) -> AccentId {
        let key = s.to_ascii_lowercase();
        AccentId(
            ACCENTS
                .iter()
                .find(|a| a.slug == key)
                .unwrap_or(&ACCENTS[0])
                .slug,
        )
    }

    fn resolved(self) -> AccentId {
        AccentId(self.def().slug)
    }
}

impl serde::Serialize for AccentId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.resolved().0)
    }
}

impl<'de> serde::Deserialize<'de> for AccentId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(d)?;
        Ok(AccentId::from_slug(&s))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub palette: Palette,
    pub id: ThemeId,
    pub mode: Mode,
    /// The accent this palette was resolved with. `None` is the palette's own seed.
    pub accent: Option<AccentId>,
    /// How tight the grid constants are drawn.
    pub density: Density,
}

impl Theme {
    pub fn current() -> Theme {
        CURRENT.with(|c| *c.borrow())
    }

    pub fn set(theme: Theme) {
        CURRENT.with(|c| *c.borrow_mut() = theme);
    }

    pub fn is_dark(&self) -> bool {
        self.mode == Mode::Dark
    }
}

impl ThemeId {
    pub const DARK: ThemeId = ThemeId("dark");
    pub const LIGHT: ThemeId = ThemeId("light");

    /// Every palette the build ships, in registry order.
    pub fn all() -> impl Iterator<Item = ThemeId> {
        PALETTES.iter().map(|p| ThemeId(p.slug))
    }

    /// The slug, as it is written to prefs.
    pub fn slug(self) -> &'static str {
        self.def().slug
    }

    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn mode(self) -> Mode {
        self.def().mode
    }

    /// The same family's other ground — what the theme toggle switches to.
    pub fn counterpart(self) -> ThemeId {
        ThemeId(self.def().counterpart).resolved()
    }

    /// The registry row, falling back to the first entry so an id from anywhere still resolves.
    fn def(self) -> &'static PaletteDef {
        PALETTES
            .iter()
            .find(|p| p.slug == self.0)
            .unwrap_or(&PALETTES[0])
    }

    /// This id with its slug pinned to a registry row.
    fn resolved(self) -> ThemeId {
        ThemeId(self.def().slug)
    }

    /// A slug read from anywhere outside the build. Case-insensitive, which is also how the old
    /// `"Dark"`/`"Light"` enum spellings still read as `dark`/`light`; anything unknown falls back
    /// to the default rather than failing the whole blob.
    fn from_slug(s: &str) -> ThemeId {
        let key = s.to_ascii_lowercase();
        PALETTES
            .iter()
            .find(|p| p.slug == key)
            .map(|p| ThemeId(p.slug))
            .unwrap_or(ThemeId::DARK)
    }
}

impl serde::Serialize for ThemeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.resolved().0)
    }
}

impl<'de> serde::Deserialize<'de> for ThemeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(d)?;
        Ok(ThemeId::from_slug(&s))
    }
}

thread_local! {
    static CURRENT: RefCell<Theme> = RefCell::new(palette_for(ThemeId::DARK));
}

/// Switch palettes.
///
/// Sets Ubiq's own tokens *and* the component library's theme, so the editor, the textarea and the
/// markdown view follow the rest of the window instead of staying in whichever mode they booted in.
/// The library resolves its own palette from the mode first; then the tokens below overwrite the
/// ones that have a counterpart here, so the two cannot drift and a third family is expressible.
pub fn set_mode(id: ThemeId, cx: &mut App) {
    set_theme(id, Theme::current().accent, Theme::current().density, cx);
}

/// Switch density, leaving the palette and the accent as they stand.
pub fn set_density(density: Density, cx: &mut App) {
    let current = Theme::current();
    set_theme(current.id, current.accent, density, cx);
}

/// Switch palette, accent and density together.
///
/// The one place the axes are resolved into the `Theme` every accessor reads, so no call site
/// outside this file learns that any of them is separable. [`set_mode`] and [`set_density`] are
/// this with the other axes left as they stand.
pub fn set_theme(id: ThemeId, accent: Option<AccentId>, density: Density, cx: &mut App) {
    let theme = resolve(id, accent, density);
    Theme::set(theme);
    let mode = match theme.mode {
        Mode::Dark => gpui_component::ThemeMode::Dark,
        Mode::Light => gpui_component::ThemeMode::Light,
    };
    gpui_component::Theme::change(mode, None, cx);
    dress_component_library(&theme.palette, cx);
}

/// Hand the component library Ubiq's palette.
///
/// Only the fields that plainly correspond are written; the rest stay as the library resolved them
/// from the mode. `sync_base` is what pushes the result down to the layer that paints scrollbars
/// and resize handles without going through the widget set.
fn dress_component_library(p: &Palette, cx: &mut App) {
    // Through `DerefMut`, so `t.list` names `ThemeColor`'s row colour rather than the `Theme`
    // field of the same name that holds the list's geometry.
    let t: &mut gpui_component::ThemeColor = gpui_component::Theme::global_mut(cx);

    // Grounds and ink.
    t.background = p.surface.base.into();
    t.foreground = p.text.primary.into();
    t.popover = p.surface.raised.into();
    t.popover_foreground = p.text.primary.into();
    t.secondary = p.surface.raised.into();
    t.secondary_foreground = p.text.primary.into();
    t.muted = p.surface.raised.into();
    t.muted_foreground = p.text.muted.into();
    t.skeleton = p.surface.raised.into();
    t.caret = p.text.primary.into();
    t.overlay = p.surface.scrim.into();

    // Borders and focus.
    t.border = p.border.default.into();
    t.input = p.border.default.into();
    t.ring = p.border.focus.into();
    t.switch = p.border.default.into();

    // The library's `accent` is the hover ground, not a brand colour.
    t.accent = p.surface.hover.into();
    t.accent_foreground = p.text.primary.into();
    t.drop_target = p.accent.soft.into();

    // The brand colour is `primary`.
    t.primary = p.accent.primary.into();
    t.primary_foreground = p.text.on_accent.into();
    t.primary_hover = p.accent.muted.into();
    t.primary_active = p.accent.muted.into();

    // Status.
    t.danger = p.status.danger.into();
    t.danger_foreground = p.text.on_accent.into();
    t.success = p.status.success.into();
    t.success_foreground = p.text.on_accent.into();
    t.warning = p.status.warning.into();
    t.warning_foreground = p.text.on_accent.into();
    t.info = p.status.info.into();
    t.info_foreground = p.text.on_accent.into();

    // Selection and links.
    t.selection = p.terminal.selection.into();
    t.link = p.terminal.link_underline.into();
    t.link_hover = p.terminal.link_underline_hover.into();

    // Rows.
    t.list = p.surface.base.into();
    t.list_hover = p.surface.hover.into();
    t.list_active = p.surface.selected.into();
    t.list_head = p.surface.raised.into();
    t.table = p.surface.base.into();
    t.table_head = p.surface.raised.into();
    t.table_head_foreground = p.text.muted.into();
    t.table_hover = p.surface.hover.into();
    t.table_active = p.surface.selected.into();
    t.table_row_border = p.border.default.into();

    // Chrome the library draws for its dock.
    t.title_bar = p.surface.base.into();
    t.title_bar_border = p.border.default.into();
    t.status_bar = p.surface.base.into();
    t.status_bar_border = p.border.default.into();
    t.tab = p.surface.pane_bg.into();
    t.tab_active = p.surface.base.into();
    t.tab_bar = p.surface.app_bg.into();
    t.scrollbar = p.surface.pane_bg.into();
    t.scrollbar_thumb = p.border.default.into();
    t.scrollbar_thumb_hover = p.text.faint.into();

    gpui_component::Theme::sync_base(cx);
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

pub fn selected_focus() -> Rgba {
    Theme::current().palette.surface.selected_focus
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

/// The accent the window is dressed in, or `None` for the palette's own seed. What prefs write
/// down; every colour it decides is already resolved into the tokens above.
pub fn accent_id() -> Option<AccentId> {
    Theme::current().accent
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

/// How full a plan reads, as a colour: fine under 75%, warning from 75, danger from 90.
///
/// A status colour rather than the accent, because a quota is something *reported* about the
/// account and not something the user is acting in — and because two accent rings beside each
/// other in the conversation footer would read as one fact drawn twice. One accessor so the ring
/// in the footer and the meter in the settings section can never disagree about where the line
/// falls.
pub fn usage_tone(pct: u8) -> Rgba {
    match pct {
        90.. => danger(),
        75..=89 => warning(),
        _ => success(),
    }
}

pub fn ribbon_alpha() -> Rgba {
    Theme::current().palette.status.ribbon_alpha
}

pub fn ribbon_beta() -> Rgba {
    Theme::current().palette.status.ribbon_beta
}

pub fn ribbon_ink() -> Rgba {
    Theme::current().palette.status.ribbon_ink
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

const DARK: Palette = Palette {
    surface: SurfaceColors {
        app_bg: rgba_hex(0x0d0d11),
        pane_bg: rgba_hex(0x121216),
        base: rgba_hex(0x1c1c22),
        raised: rgba_hex(0x25252c),
        hover: rgba_hex(0x2a2a33),
        selected: rgba_hex(0x1c2a44),
        selected_focus: rgba_hex(0x27405f),
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
        ribbon_alpha: rgba_hex(0xf5c518),
        ribbon_beta: rgba_hex(0xf5a04a),
        ribbon_ink: rgba_hex(0x1b1b1b),
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
};

const LIGHT: Palette = Palette {
    surface: SurfaceColors {
        app_bg: rgba_hex(0xfafafc),
        pane_bg: rgba_hex(0xf8f8fa),
        base: rgba_hex(0xffffff),
        raised: rgba_hex(0xf0f0f4),
        hover: rgba_hex(0xe8e8ec),
        selected: rgba_hex(0xd4e4ff),
        selected_focus: rgba_hex(0xb3d0ff),
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
        ribbon_alpha: rgba_hex(0xf5c518),
        ribbon_beta: rgba_hex(0xf5a04a),
        ribbon_ink: rgba_hex(0x1b1b1b),
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
};

/// The build-channel ribbon marks the build, not the mood, so every palette carries the same three
/// values — named once rather than retyped per family.
const RIBBON_ALPHA: Rgba = rgba_hex(0xf5c518);
const RIBBON_BETA: Rgba = rgba_hex(0xf5a04a);
const RIBBON_INK: Rgba = rgba_hex(0x1b1b1b);

/// The swatch wheel a high-contrast palette uses: the same sixteen identities, taken to full
/// strength so each one still separates from the others at maximum ground contrast. `D19` — a
/// swatch is identity, not role, so these are literals and no accent choice reaches them.
const VIVID_SWATCHES: ProjectColors = ProjectColors {
    swatches: [
        rgba_hex(0x4d9dff),
        rgba_hex(0xb98cff),
        rgba_hex(0x2fd6b4),
        rgba_hex(0xffc043),
        rgba_hex(0xff7d9e),
        rgba_hex(0x6fdd52),
        rgba_hex(0xff5c5c),
        rgba_hex(0xff9330),
        rgba_hex(0xa8e02a),
        rgba_hex(0x1fc8e8),
        rgba_hex(0x8189ff),
        rgba_hex(0xf95cf0),
        rgba_hex(0xc98a55),
        rgba_hex(0x9aa8c0),
        rgba_hex(0xc8bf2e),
        rgba_hex(0xff4d7e),
    ],
    temporary: rgba_hex(0x9a9aa4),
};

const VIVID_SWATCHES_LIGHT: ProjectColors = ProjectColors {
    swatches: [
        rgba_hex(0x0040c0),
        rgba_hex(0x6a1fc0),
        rgba_hex(0x006b5a),
        rgba_hex(0x8a5a00),
        rgba_hex(0xaa1140),
        rgba_hex(0x2a6b00),
        rgba_hex(0xb00000),
        rgba_hex(0x9a4400),
        rgba_hex(0x4a6b00),
        rgba_hex(0x006080),
        rgba_hex(0x2f2fb0),
        rgba_hex(0x9a008f),
        rgba_hex(0x6b3d1f),
        rgba_hex(0x3f4a5c),
        rgba_hex(0x5c5500),
        rgba_hex(0xa00040),
    ],
    temporary: rgba_hex(0x5a5a62),
};

/// Ember — a warm, low-contrast dark. Browned grounds and an amber accent, with the ink pulled in
/// from white so a long session on it reads softer than the built-in pair.
const EMBER_DARK: Palette = Palette {
    surface: SurfaceColors {
        app_bg: rgba_hex(0x171310),
        pane_bg: rgba_hex(0x1d1815),
        base: rgba_hex(0x26201b),
        raised: rgba_hex(0x302823),
        hover: rgba_hex(0x3a312a),
        selected: rgba_hex(0x3f3121),
        selected_focus: rgba_hex(0x523d22),
        scrim: rgba_hex_a(0x0c0806, 0.60),
    },
    text: TextColors {
        primary: rgba_hex(0xe3d7c8),
        muted: rgba_hex(0xa89583),
        faint: rgba_hex(0x7c6c5c),
        on_accent: rgba_hex(0x1b1409),
    },
    accent: AccentColors {
        primary: rgba_hex(0xd9a05b),
        muted: rgba_hex(0x9c7440),
        soft: rgba_hex_a(0xd9a05b, 0.16),
    },
    border: BorderColors {
        default: rgba_hex(0x3a312a),
        focus: rgba_hex(0xd9a05b),
    },
    status: StatusColors {
        danger: rgba_hex(0xd4635f),
        danger_soft: rgba_hex_a(0xd4635f, 0.16),
        success: rgba_hex(0x8ba85f),
        success_soft: rgba_hex_a(0x8ba85f, 0.16),
        warning: rgba_hex(0xe0a94a),
        warning_soft: rgba_hex_a(0xe0a94a, 0.16),
        info: rgba_hex(0x86a6bf),
        info_soft: rgba_hex_a(0x86a6bf, 0.16),
        ribbon_alpha: RIBBON_ALPHA,
        ribbon_beta: RIBBON_BETA,
        ribbon_ink: RIBBON_INK,
    },
    project: DARK.project,
    terminal: TerminalColors {
        selection: rgba_hex(0x3f3121),
        link_underline: rgba_hex(0xd9a05b),
        link_underline_hover: rgba_hex(0xe8bb82),
    },
};

/// Ember's light ground: paper rather than white, with the same amber accent taken deep enough to
/// read on it.
const EMBER_LIGHT: Palette = Palette {
    surface: SurfaceColors {
        app_bg: rgba_hex(0xf6f0e6),
        pane_bg: rgba_hex(0xf2ebdf),
        base: rgba_hex(0xfdfaf3),
        raised: rgba_hex(0xece3d4),
        hover: rgba_hex(0xe3d8c6),
        selected: rgba_hex(0xf0dfbd),
        selected_focus: rgba_hex(0xe3c793),
        scrim: rgba_hex_a(0x3a2c1c, 0.34),
    },
    text: TextColors {
        primary: rgba_hex(0x3b3229),
        muted: rgba_hex(0x7a6b58),
        faint: rgba_hex(0xa2947f),
        on_accent: rgba_hex(0xfdfaf3),
    },
    accent: AccentColors {
        primary: rgba_hex(0xa9702a),
        muted: rgba_hex(0xc0925a),
        soft: rgba_hex_a(0xa9702a, 0.12),
    },
    border: BorderColors {
        default: rgba_hex(0xdcd0bb),
        focus: rgba_hex(0xa9702a),
    },
    status: StatusColors {
        danger: rgba_hex(0xb04a44),
        danger_soft: rgba_hex_a(0xb04a44, 0.12),
        success: rgba_hex(0x5f7d3a),
        success_soft: rgba_hex_a(0x5f7d3a, 0.12),
        warning: rgba_hex(0xb27d1c),
        warning_soft: rgba_hex_a(0xb27d1c, 0.14),
        info: rgba_hex(0x3d7594),
        info_soft: rgba_hex_a(0x3d7594, 0.10),
        ribbon_alpha: RIBBON_ALPHA,
        ribbon_beta: RIBBON_BETA,
        ribbon_ink: RIBBON_INK,
    },
    project: LIGHT.project,
    terminal: TerminalColors {
        selection: rgba_hex(0xf0dfbd),
        link_underline: rgba_hex(0xa9702a),
        link_underline_hover: rgba_hex(0xc0925a),
    },
};

/// Contrast — a black ground, white ink, and a `faint` that is still legible. The built-in dark's
/// `text_faint` sits at `0x5c5c68`, which is the case this family exists for: nothing here drops
/// below a grey a reader can resolve, and borders are drawn rather than implied.
const CONTRAST_DARK: Palette = Palette {
    surface: SurfaceColors {
        app_bg: rgba_hex(0x000000),
        pane_bg: rgba_hex(0x000000),
        base: rgba_hex(0x0a0a0a),
        raised: rgba_hex(0x171717),
        hover: rgba_hex(0x262626),
        selected: rgba_hex(0x00335c),
        selected_focus: rgba_hex(0x004b86),
        scrim: rgba_hex_a(0x000000, 0.80),
    },
    text: TextColors {
        primary: rgba_hex(0xffffff),
        muted: rgba_hex(0xd4d4d4),
        faint: rgba_hex(0xa8a8a8),
        on_accent: rgba_hex(0x000000),
    },
    accent: AccentColors {
        primary: rgba_hex(0x6cb6ff),
        muted: rgba_hex(0x3d8ee0),
        soft: rgba_hex_a(0x6cb6ff, 0.24),
    },
    border: BorderColors {
        default: rgba_hex(0x6e6e6e),
        focus: rgba_hex(0xffd400),
    },
    status: StatusColors {
        danger: rgba_hex(0xff6b6b),
        danger_soft: rgba_hex_a(0xff6b6b, 0.22),
        success: rgba_hex(0x54e07f),
        success_soft: rgba_hex_a(0x54e07f, 0.22),
        warning: rgba_hex(0xffd400),
        warning_soft: rgba_hex_a(0xffd400, 0.22),
        info: rgba_hex(0x6cb6ff),
        info_soft: rgba_hex_a(0x6cb6ff, 0.22),
        ribbon_alpha: RIBBON_ALPHA,
        ribbon_beta: RIBBON_BETA,
        ribbon_ink: RIBBON_INK,
    },
    project: VIVID_SWATCHES,
    terminal: TerminalColors {
        selection: rgba_hex(0x00335c),
        link_underline: rgba_hex(0x6cb6ff),
        link_underline_hover: rgba_hex(0xa8d6ff),
    },
};

/// Contrast on white: black ink, drawn borders, and the same refusal to fade anything below
/// legibility.
const CONTRAST_LIGHT: Palette = Palette {
    surface: SurfaceColors {
        app_bg: rgba_hex(0xffffff),
        pane_bg: rgba_hex(0xffffff),
        base: rgba_hex(0xffffff),
        raised: rgba_hex(0xf0f0f0),
        hover: rgba_hex(0xe0e0e0),
        selected: rgba_hex(0xcce0ff),
        selected_focus: rgba_hex(0xa6c8f5),
        scrim: rgba_hex_a(0x000000, 0.50),
    },
    text: TextColors {
        primary: rgba_hex(0x000000),
        muted: rgba_hex(0x333333),
        faint: rgba_hex(0x555555),
        on_accent: rgba_hex(0xffffff),
    },
    accent: AccentColors {
        primary: rgba_hex(0x0040c0),
        muted: rgba_hex(0x2a5fd0),
        soft: rgba_hex_a(0x0040c0, 0.16),
    },
    border: BorderColors {
        default: rgba_hex(0x555555),
        focus: rgba_hex(0x0040c0),
    },
    status: StatusColors {
        danger: rgba_hex(0xb00000),
        danger_soft: rgba_hex_a(0xb00000, 0.16),
        success: rgba_hex(0x006622),
        success_soft: rgba_hex_a(0x006622, 0.16),
        warning: rgba_hex(0x8a5a00),
        warning_soft: rgba_hex_a(0x8a5a00, 0.16),
        info: rgba_hex(0x0040c0),
        info_soft: rgba_hex_a(0x0040c0, 0.16),
        ribbon_alpha: RIBBON_ALPHA,
        ribbon_beta: RIBBON_BETA,
        ribbon_ink: RIBBON_INK,
    },
    project: VIVID_SWATCHES_LIGHT,
    terminal: TerminalColors {
        selection: rgba_hex(0xcce0ff),
        link_underline: rgba_hex(0x0040c0),
        link_underline_hover: rgba_hex(0x2a5fd0),
    },
};

/// Navy — a cool dark with blue grounds rather than a grey one. The built-in dark is near-black
/// with a blue accent; this family is the accent's hue taken into the surface stack, so a long
/// session on it reads as night sky rather than charcoal.
const NAVY_DARK: Palette = Palette {
    surface: SurfaceColors {
        app_bg: rgba_hex(0x0a121c),
        pane_bg: rgba_hex(0x0f1824),
        base: rgba_hex(0x15202e),
        raised: rgba_hex(0x1c2a3c),
        hover: rgba_hex(0x24364c),
        selected: rgba_hex(0x1a3a5c),
        selected_focus: rgba_hex(0x245278),
        scrim: rgba_hex_a(0x050a12, 0.62),
    },
    text: TextColors {
        primary: rgba_hex(0xd8e4f0),
        muted: rgba_hex(0x7e90a6),
        faint: rgba_hex(0x546478),
        on_accent: rgba_hex(0xffffff),
    },
    accent: AccentColors {
        primary: rgba_hex(0x5ba8f5),
        muted: rgba_hex(0x3d74b0),
        soft: rgba_hex_a(0x5ba8f5, 0.16),
    },
    border: BorderColors {
        default: rgba_hex(0x2a3c52),
        focus: rgba_hex(0x5ba8f5),
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
        ribbon_alpha: RIBBON_ALPHA,
        ribbon_beta: RIBBON_BETA,
        ribbon_ink: RIBBON_INK,
    },
    project: DARK.project,
    terminal: TerminalColors {
        selection: rgba_hex(0x1a3a5c),
        link_underline: rgba_hex(0x5ba8f5),
        link_underline_hover: rgba_hex(0x7ebcf8),
    },
};

/// Navy's light ground: paper with a blue cast, and the same sky-blue seed taken deep enough to
/// read on it.
const NAVY_LIGHT: Palette = Palette {
    surface: SurfaceColors {
        app_bg: rgba_hex(0xeef3f8),
        pane_bg: rgba_hex(0xe6eef6),
        base: rgba_hex(0xf7fafd),
        raised: rgba_hex(0xdde6f0),
        hover: rgba_hex(0xd0dce8),
        selected: rgba_hex(0xc5d8f0),
        selected_focus: rgba_hex(0xa8c6ea),
        scrim: rgba_hex_a(0x15202e, 0.38),
    },
    text: TextColors {
        primary: rgba_hex(0x15202e),
        muted: rgba_hex(0x4a5d73),
        faint: rgba_hex(0x7a8ca0),
        on_accent: rgba_hex(0xffffff),
    },
    accent: AccentColors {
        primary: rgba_hex(0x2a6ec8),
        muted: rgba_hex(0x4a88d4),
        soft: rgba_hex_a(0x2a6ec8, 0.12),
    },
    border: BorderColors {
        default: rgba_hex(0xc5d0dc),
        focus: rgba_hex(0x2a6ec8),
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
        ribbon_alpha: RIBBON_ALPHA,
        ribbon_beta: RIBBON_BETA,
        ribbon_ink: RIBBON_INK,
    },
    project: LIGHT.project,
    terminal: TerminalColors {
        selection: rgba_hex(0xc5d8f0),
        link_underline: rgba_hex(0x2a6ec8),
        link_underline_hover: rgba_hex(0x4a88d4),
    },
};

/// Violet — aubergine grounds and a lilac seed. The built-in dark is grey; this family is the
/// purple of the project swatches taken into the surface stack, so chrome, panes and selected
/// rows stay in one hue rather than wearing a purple accent on charcoal.
const VIOLET_DARK: Palette = Palette {
    surface: SurfaceColors {
        app_bg: rgba_hex(0x120e18),
        pane_bg: rgba_hex(0x17121f),
        base: rgba_hex(0x1e1728),
        raised: rgba_hex(0x271f34),
        hover: rgba_hex(0x322843),
        selected: rgba_hex(0x3a2460),
        selected_focus: rgba_hex(0x4e3080),
        scrim: rgba_hex_a(0x0a0710, 0.62),
    },
    text: TextColors {
        primary: rgba_hex(0xece6f4),
        muted: rgba_hex(0x9a8cad),
        faint: rgba_hex(0x6c5e80),
        on_accent: rgba_hex(0xffffff),
    },
    accent: AccentColors {
        primary: rgba_hex(0xb794f6),
        muted: rgba_hex(0x7a5cb8),
        soft: rgba_hex_a(0xb794f6, 0.16),
    },
    border: BorderColors {
        default: rgba_hex(0x322a42),
        focus: rgba_hex(0xb794f6),
    },
    status: StatusColors {
        danger: rgba_hex(0xe5484d),
        danger_soft: rgba_hex_a(0xe5484d, 0.16),
        success: rgba_hex(0x46a758),
        success_soft: rgba_hex_a(0x46a758, 0.16),
        warning: rgba_hex(0xef9f2a),
        warning_soft: rgba_hex_a(0xef9f2a, 0.16),
        info: rgba_hex(0x82a8e8),
        info_soft: rgba_hex_a(0x82a8e8, 0.16),
        ribbon_alpha: RIBBON_ALPHA,
        ribbon_beta: RIBBON_BETA,
        ribbon_ink: RIBBON_INK,
    },
    project: DARK.project,
    terminal: TerminalColors {
        selection: rgba_hex(0x3a2460),
        link_underline: rgba_hex(0xb794f6),
        link_underline_hover: rgba_hex(0xcdb0f8),
    },
};

/// Violet's light ground: paper with a lilac cast, and the same seed taken deep enough to read
/// on it.
const VIOLET_LIGHT: Palette = Palette {
    surface: SurfaceColors {
        app_bg: rgba_hex(0xf4eef8),
        pane_bg: rgba_hex(0xefe8f5),
        base: rgba_hex(0xfbf8fd),
        raised: rgba_hex(0xe8def0),
        hover: rgba_hex(0xddd0e8),
        selected: rgba_hex(0xe4d0f5),
        selected_focus: rgba_hex(0xd0b4ec),
        scrim: rgba_hex_a(0x2a1f38, 0.34),
    },
    text: TextColors {
        primary: rgba_hex(0x2a1f38),
        muted: rgba_hex(0x6a5a7c),
        faint: rgba_hex(0x9488a4),
        on_accent: rgba_hex(0xffffff),
    },
    accent: AccentColors {
        primary: rgba_hex(0x7a45c8),
        muted: rgba_hex(0x9a70d8),
        soft: rgba_hex_a(0x7a45c8, 0.12),
    },
    border: BorderColors {
        default: rgba_hex(0xd4c8e0),
        focus: rgba_hex(0x7a45c8),
    },
    status: StatusColors {
        danger: rgba_hex(0xd1353b),
        danger_soft: rgba_hex_a(0xd1353b, 0.12),
        success: rgba_hex(0x3a9148),
        success_soft: rgba_hex_a(0x3a9148, 0.12),
        warning: rgba_hex(0xd48a1e),
        warning_soft: rgba_hex_a(0xd48a1e, 0.14),
        info: rgba_hex(0x3d5fb0),
        info_soft: rgba_hex_a(0x3d5fb0, 0.10),
        ribbon_alpha: RIBBON_ALPHA,
        ribbon_beta: RIBBON_BETA,
        ribbon_ink: RIBBON_INK,
    },
    project: LIGHT.project,
    terminal: TerminalColors {
        selection: rgba_hex(0xe4d0f5),
        link_underline: rgba_hex(0x7a45c8),
        link_underline_hover: rgba_hex(0x9a70d8),
    },
};

/// Every palette the build ships. The first entry is the default and the fallback for a slug
/// nothing here answers to.
pub static PALETTES: &[PaletteDef] = &[
    PaletteDef {
        slug: "dark",
        name: "Dark",
        mode: Mode::Dark,
        counterpart: "light",
        palette: DARK,
    },
    PaletteDef {
        slug: "light",
        name: "Light",
        mode: Mode::Light,
        counterpart: "dark",
        palette: LIGHT,
    },
    PaletteDef {
        slug: "ember-dark",
        name: "Ember Dark",
        mode: Mode::Dark,
        counterpart: "ember-light",
        palette: EMBER_DARK,
    },
    PaletteDef {
        slug: "ember-light",
        name: "Ember Light",
        mode: Mode::Light,
        counterpart: "ember-dark",
        palette: EMBER_LIGHT,
    },
    PaletteDef {
        slug: "contrast-dark",
        name: "Contrast Dark",
        mode: Mode::Dark,
        counterpart: "contrast-light",
        palette: CONTRAST_DARK,
    },
    PaletteDef {
        slug: "contrast-light",
        name: "Contrast Light",
        mode: Mode::Light,
        counterpart: "contrast-dark",
        palette: CONTRAST_LIGHT,
    },
    PaletteDef {
        slug: "navy-dark",
        name: "Navy",
        mode: Mode::Dark,
        counterpart: "navy-light",
        palette: NAVY_DARK,
    },
    PaletteDef {
        slug: "navy-light",
        name: "Navy",
        mode: Mode::Light,
        counterpart: "navy-dark",
        palette: NAVY_LIGHT,
    },
    PaletteDef {
        slug: "violet-dark",
        name: "Violet",
        mode: Mode::Dark,
        counterpart: "violet-light",
        palette: VIOLET_DARK,
    },
    PaletteDef {
        slug: "violet-light",
        name: "Violet",
        mode: Mode::Light,
        counterpart: "violet-dark",
        palette: VIOLET_LIGHT,
    },
];

/// Every accent the build ships, in registry order. The first entry is the fallback for a slug
/// nothing here answers to.
pub static ACCENTS: &[AccentDef] = &[
    AccentDef {
        slug: "blue",
        name: "Blue",
        seed: rgba_hex(0x5b8def),
    },
    AccentDef {
        slug: "green",
        name: "Green",
        seed: rgba_hex(0x3fa563),
    },
    AccentDef {
        slug: "purple",
        name: "Purple",
        seed: rgba_hex(0x9b7cf0),
    },
    AccentDef {
        slug: "amber",
        name: "Amber",
        seed: rgba_hex(0xd9a05b),
    },
    AccentDef {
        slug: "rose",
        name: "Rose",
        seed: rgba_hex(0xe0496e),
    },
    AccentDef {
        slug: "teal",
        name: "Teal",
        seed: rgba_hex(0x2fb0c7),
    },
];

/// How far `accent_muted` is pulled toward the ground, and how far `link_underline_hover` is
/// lifted off the seed. Fixed ratios: the palette supplies the ground, the accent the hue.
const ACCENT_MUTED_MIX: f32 = 0.4;
const ACCENT_LIFT: f32 = 0.22;
/// The alpha every `_soft` token is drawn at, declared once here rather than per palette.
const ACCENT_SOFT_ALPHA: f32 = 0.16;
/// The contrast an accent has to reach against the surface it sits on before it stops being
/// corrected — WCAG's floor for a non-text mark.
const ACCENT_MIN_CONTRAST: f64 = 3.0;

const WHITE: Rgba = rgba_hex(0xffffff);
const BLACK: Rgba = rgba_hex(0x000000);

/// Resolve a palette and an accent into the theme every accessor reads. `None` is the palette's
/// own seed, which is the `accent.primary` its definition declares.
pub fn resolve(id: ThemeId, accent: Option<AccentId>, density: Density) -> Theme {
    let def = id.def();
    let seed = accent.map_or(def.palette.accent.primary, AccentId::seed);
    Theme {
        id: ThemeId(def.slug),
        mode: def.mode,
        palette: with_accent(def.palette, seed),
        accent,
        density,
    }
}

/// The palette with its six hued tokens derived from `seed`.
///
/// This is the whole of the accent axis. It lives here because the derivation is a colour
/// decision: a `.alpha()` or a hand-mixed shade at a call site is the defect `D10` names.
fn with_accent(mut p: Palette, seed: Rgba) -> Palette {
    let primary = readable_on(seed, p.surface.base);
    p.accent = AccentColors {
        primary,
        muted: mix(seed, p.surface.base, ACCENT_MUTED_MIX),
        soft: fade(seed, ACCENT_SOFT_ALPHA),
    };
    p.border.focus = seed;
    p.terminal.link_underline = seed;
    p.terminal.link_underline_hover = mix(seed, WHITE, ACCENT_LIFT);
    p.text.on_accent = if mark_dark(primary) { WHITE } else { BLACK };
    p
}

/// A seed pushed away from the ground until it reads on it — a pale accent darkened on a light
/// palette, a dark one lifted on a dark palette. A seed that already clears the floor is
/// untouched, which is why every built-in palette resolves to the accent it declares.
fn readable_on(seed: Rgba, ground: Rgba) -> Rgba {
    let toward = if relative_luminance(ground) < 0.5 {
        WHITE
    } else {
        BLACK
    };
    let mut out = seed;
    for _ in 0..10 {
        if contrast(out, ground) >= ACCENT_MIN_CONTRAST {
            break;
        }
        out = mix(out, toward, 0.1);
    }
    out
}

/// WCAG contrast ratio between two colours, either way round.
fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (a, b) = (relative_luminance(a), relative_luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// `t` of the way from `a` to `b`, keeping `a`'s alpha.
fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a,
    }
}

/// The palette on its own accent — what a window boots on and what the toggle lands on.
pub fn palette_for(id: ThemeId) -> Theme {
    resolve(id, None, Density::Regular)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every family is a closed pair: the toggle from any palette lands on a registered slug of
    /// the other ground and comes straight back. A typo in a `counterpart` would otherwise show up
    /// only as a click that silently falls back to `dark`.
    #[test]
    fn every_counterpart_is_the_other_ground_of_the_same_family() {
        for id in ThemeId::all() {
            let other = id.counterpart();
            assert_eq!(other.slug(), other.0, "{} has an unknown counterpart", id.0);
            assert_ne!(other.mode(), id.mode(), "{} toggles to its own mode", id.0);
            assert_eq!(other.counterpart(), id, "{} does not toggle back", id.0);
        }
    }

    /// Settings lists one pill per family on the ground in use, labelled by `name()`. Two
    /// families sharing a name on the same ground would draw as one choice.
    #[test]
    fn each_ground_lists_distinct_family_names() {
        for mode in [Mode::Dark, Mode::Light] {
            let mut names: Vec<&str> = ThemeId::all()
                .filter(|id| id.mode() == mode)
                .map(ThemeId::name)
                .collect();
            let count = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(
                names.len(),
                count,
                "{mode:?} has two families sharing a pill label"
            );
        }
    }

    /// Where the two lines fall, in both grounds. The quota ring and the accounts meter read this
    /// one accessor, so a threshold moving in one surface and not the other is not expressible —
    /// but a threshold moving *at all* is a change to what "nearly out" means, which is why the
    /// boundaries are asserted rather than left to the caller that happens to be looking.
    #[test]
    fn how_full_a_plan_reads_turns_at_seventy_five_and_ninety() {
        for id in [ThemeId::DARK, ThemeId::LIGHT] {
            Theme::set(resolve(id, None, Density::default()));
            assert_eq!(usage_tone(0), success());
            assert_eq!(usage_tone(74), success());
            assert_eq!(usage_tone(75), warning());
            assert_eq!(usage_tone(89), warning());
            assert_eq!(usage_tone(90), danger());
            assert_eq!(usage_tone(100), danger());
        }
    }

    /// The slug is what prefs carry, and the two enum spellings a blob written before the registry
    /// used still read.
    #[test]
    fn a_slug_round_trips_and_the_old_spellings_still_read() {
        for id in ThemeId::all() {
            let json = serde_json::to_string(&id).expect("serialises");
            assert_eq!(json, format!("\"{}\"", id.0));
            let back: ThemeId = serde_json::from_str(&json).expect("deserialises");
            assert_eq!(back, id);
        }
        assert_eq!(ThemeId::from_slug("Dark"), ThemeId::DARK);
        assert_eq!(ThemeId::from_slug("Light"), ThemeId::LIGHT);
        assert_eq!(ThemeId::from_slug("no-such-palette"), ThemeId::DARK);
    }

    /// The six hued tokens come from the seed, and the ink on the accent flips with it: a pale
    /// accent takes black text, a dark one white. The rest of the palette is untouched.
    #[test]
    fn the_hued_tokens_derive_from_the_seed() {
        let pale = rgba_hex(0xffe08a);
        let dark = rgba_hex(0x102080);

        let p = with_accent(DARK, pale);
        assert_eq!(p.border.focus, pale, "focus is the seed, unmodified");
        assert_eq!(p.terminal.link_underline, pale);
        assert_eq!(p.accent.soft.a, ACCENT_SOFT_ALPHA);
        assert_eq!(p.text.on_accent, BLACK, "black ink on a pale accent");
        assert_eq!(
            p.surface.base, DARK.surface.base,
            "the ground is the palette's"
        );

        let p = with_accent(LIGHT, dark);
        assert_eq!(p.text.on_accent, WHITE, "white ink on a dark accent");

        // A seed too close to the ground is pushed off it until it reads.
        let washed = with_accent(LIGHT, rgba_hex(0xf5f5c0));
        assert!(contrast(washed.accent.primary, LIGHT.surface.base) >= ACCENT_MIN_CONTRAST);

        // Every shipped accent resolves on every shipped palette.
        for id in ThemeId::all() {
            for accent in AccentId::all() {
                let theme = resolve(id, Some(accent), Density::Regular);
                assert_eq!(theme.accent, Some(accent));
                assert!(
                    contrast(theme.palette.accent.primary, theme.palette.surface.base)
                        >= ACCENT_MIN_CONTRAST,
                    "{} on {} does not read",
                    accent.0,
                    id.0
                );
            }
            // No accent chosen means the palette's own seed, unmodified.
            let own = resolve(id, None, Density::Regular);
            assert_eq!(own.palette.border.focus, id.def().palette.accent.primary);
        }
    }

    /// An accent slug round-trips, and an unknown one falls back rather than failing the blob.
    #[test]
    fn an_accent_slug_round_trips() {
        for accent in AccentId::all() {
            let json = serde_json::to_string(&accent).expect("serialises");
            let back: AccentId = serde_json::from_str(&json).expect("deserialises");
            assert_eq!(back, accent);
        }
        assert_eq!(AccentId::from_slug("Blue"), AccentId("blue"));
        assert_eq!(AccentId::from_slug("no-such-accent"), AccentId("blue"));
    }

    /// Density is one factor over the grid constants, and over nothing else: a dragged region is
    /// what a fresh window opens at and what the user then drags, so it does not move.
    #[test]
    fn density_scales_the_grid_and_not_the_dragged_regions() {
        Theme::set(resolve(ThemeId::DARK, None, Density::Regular));
        assert_eq!(titlebar_height(), TITLEBAR_HEIGHT);

        Theme::set(resolve(ThemeId::DARK, None, Density::Compact));
        assert_eq!(titlebar_height(), (TITLEBAR_HEIGHT * 0.9).round());
        assert_eq!(EXPLORER_WIDTH, 300.0, "a dragged region is not scaled");

        Theme::set(resolve(ThemeId::DARK, None, Density::Comfortable));
        assert_eq!(titlebar_height(), (TITLEBAR_HEIGHT * 1.15).round());
        assert!(titlebar_height() > TITLEBAR_HEIGHT);
        assert_eq!(EXPLORER_WIDTH, 300.0, "a dragged region is not scaled");

        Theme::set(resolve(ThemeId::DARK, None, Density::Regular));
    }

    /// Three families, three independent bases, and a role's ratio applied to its own family's
    /// base and to nobody else's.
    #[test]
    fn each_family_scales_from_its_own_base() {
        Theme::set(resolve(ThemeId::DARK, None, Density::Regular));
        set_text_scale(TextScale::default());

        // The defaults land where the hand-picked sizes they replaced were.
        assert_eq!(font(Family::Content, Role::Body), px(13.0));
        assert_eq!(font(Family::Content, Role::Meta), px(11.0));
        assert_eq!(font(Family::Chrome, Role::Body), px(12.5));
        assert_eq!(font(Family::Conversation, Role::Body), px(12.5));

        // One family moves and the other two do not.
        set_text_scale(TextScale {
            chrome: 12.5,
            content: 20.0,
            conversation: 16.0,
        });
        assert_eq!(font(Family::Content, Role::Body), px(20.0));
        assert_eq!(font(Family::Conversation, Role::Body), px(16.0));
        assert_eq!(font(Family::Chrome, Role::Body), px(12.5));

        // A ratio is over the family's own base, to the nearest half point.
        for family in [Family::Chrome, Family::Content, Family::Conversation] {
            let base = text_scale().base(family);
            for role in Role::ALL {
                let want = (base * role.ratio() * 2.0).round() / 2.0;
                assert_eq!(font(family, role), px(want), "{family:?} {role:?}");
            }
        }

        // The scale survives a palette switch, and a density switch, because it is an axis of its
        // own with nothing to resolve — see the `TEXT` cell.
        Theme::set(resolve(ThemeId::LIGHT, None, Density::Comfortable));
        assert_eq!(font(Family::Content, Role::Body), px(20.0));
        assert_eq!(font(Family::Conversation, Role::Body), px(16.0));

        set_text_scale(TextScale::default());
        Theme::set(resolve(ThemeId::DARK, None, Density::Regular));
    }
}
