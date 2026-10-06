//! The viewer container: one spec per contributed viewer the IDE editor can draw a file with
//! (`D204`).
//!
//! **What it is not: a conversion of [`ViewerKind`].** The built-in viewers — the editor, Markdown,
//! Mermaid, Excalidraw, draw.io, the image — stay the closed enum they were, because each one is
//! wired into something only the base has (a web tenant, the markdown view, the image editor's
//! panel focus). A contribution is the enum's one open variant, [`ViewerKind::Contributed`], and
//! every match on the enum answers it from the spec — `X16`'s recipe applied to the one arm rather
//! than to the whole set: `label` is the label arm, `layouts` the layouts arm, `language` the
//! forced-language arm, `render` the drawing arm, `live_reload` the "the tab on screen is left
//! alone" branch of the watcher.
//!
//! **The file stays the IDE's.** A contributed viewer opens nothing, watches nothing and saves
//! nothing: the tab reads, watches and writes through the host exactly as an editor tab does, and
//! the spec is handed the tab's own buffer to draw from. `Source` and the source half of `Split`
//! are drawn by the base — the same buffer every other viewer edits — and `render` draws every
//! other position the spec declares.
//!
//! **A claim is a ranking, not a boolean** — the question `extension-plan.md` §4 said would test
//! `X16`. [`Claim::Default`] opens the path in this viewer when no built-in has an opinion,
//! [`Claim::Offer`] lists it under "Open with" and nothing else, [`Claim::No`] does neither.
//!
//! The order is resolved **once** and cached for the process, never walked in a draw path.

use std::collections::HashMap;
use std::sync::OnceLock;

use gpui::{AnyElement, App, Context, Entity, SharedString, Window};
use gpui_component::input::EditorState;

use super::{Registry, SlotId, Slotted, ids};
use crate::app::AppState;
use crate::state::OpenFile;
use crate::state::editor::{FileLanguage, ViewLayout, ViewerKind};

/// The one group a viewer is registered in. A viewer is ordered by registration and nothing else:
/// the first spec to claim a path as its default is the one that opens it.
pub const GROUPS: &[SlotId] = &[ids::VIEWER];

/// How strongly a viewer wants a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// Open the path in this viewer unless a built-in viewer names its extension. The first
    /// registered spec to answer this wins; a later one is still offered under "Open with".
    Default,
    /// Offer this viewer under "Open with", and never open the path in it unasked.
    Offer,
    /// Not for this path.
    No,
}

/// What a contributed viewer is handed to draw.
pub struct ViewerCtx<'a> {
    pub app: &'a AppState,
    /// The tab: its key (`file.key()`) is what a viewer keys its own state by, its `path` is
    /// project-relative.
    pub file: &'a OpenFile,
    /// The tab's own buffer — never a copy. Read it (`buffer.read(cx).value()`) to draw; write it
    /// (`set_value`, which needs the `Window`) to edit, and the tab turns dirty and saves the way
    /// any other edit does.
    pub buffer: &'a Entity<EditorState>,
    /// The position being drawn: any of the spec's own [`ViewerSpec::layouts`] but `Source`, which
    /// the base draws.
    pub layout: ViewLayout,
}

/// One extra row a viewer contributes to the editor's `⋯` menu, below "Open with".
pub struct ViewerAction {
    pub label: SharedString,
    pub enabled: bool,
    /// Run with the tab's key when the row is picked. An `fn` pointer, for the reason every spec
    /// field is one: a contribution keeps its own state in a `Global`, keyed by the tab key.
    pub run: fn(&mut AppState, &str, &mut Window, &mut Context<AppState>),
}

/// The body a viewer draws for one position.
pub type Render = fn(&ViewerCtx<'_>, &mut Window, &mut Context<AppState>) -> AnyElement;

/// The `⋯` rows a viewer adds for one tab, asked every time the menu opens.
pub type Menu = fn(&AppState, &OpenFile, &App) -> Vec<ViewerAction>;

/// Content a viewer puts into the editor's top row: the left slot sits before the spacer, the right
/// one before the layout pills. It should be as tall as the row (`h_full`) and flush — the row has
/// no padding or gap of its own.
pub type HeaderSlot = fn(&AppState, &OpenFile, &mut Context<AppState>) -> AnyElement;

/// One contributed viewer.
///
/// Every field is one match on [`ViewerKind`] answered for [`ViewerKind::Contributed`] (`X16`
/// step 3), plus the claim that replaces the extension table for it.
pub struct ViewerSpec {
    pub id: SlotId,
    /// What "Open with", the status bar's picker and the tab's `⋯` call it.
    pub label: &'static str,
    /// Whether this viewer wants a path. Handed the project-relative path and, once the bytes are
    /// here, the head of the text (`None` before then — the explorer's menu and the first open ask
    /// with the path alone). A pure function: it opens nothing and is asked often.
    pub claims: fn(&str, Option<&str>) -> Claim,
    /// The positions the editor's top row offers, in drawn order. `Source` is the tab's buffer and
    /// the base draws it; `Split` is the buffer beside [`Self::render`]; anything else is
    /// `render` alone. Empty is a viewer with one position and no switch.
    pub layouts: &'static [ViewLayout],
    /// The position a tab opens in. One of `layouts`, or `Preview` for a viewer with none.
    pub default_layout: ViewLayout,
    /// The highlighter language the source half is forced into, when the path's own extension
    /// would not name it.
    pub language: Option<FileLanguage>,
    /// Re-read the tab on screen when its file changes on disk and it holds no unsaved edit.
    /// The base leaves the visible tab alone, because rebuilding a buffer the user is reading
    /// reruns its highlighter; a viewer drawing what an agent is writing wants the opposite.
    pub live_reload: bool,
    /// The body, for every position but `Source`.
    pub render: Render,
    /// Extra rows for the tab's `⋯` menu.
    pub menu: Option<Menu>,
    /// Content for the left end of the top row (a status chip).
    pub header_left: Option<HeaderSlot>,
    /// Content for the right end of the top row, before the layout pills (an action button).
    pub header_right: Option<HeaderSlot>,
}

impl Slotted for ViewerSpec {
    fn id(&self) -> SlotId {
        self.id
    }
}

/// Registers a viewer. The one way in, base or second edition.
///
/// # Panics
/// If the spec's default layout is not one it declares — a tab would open in a position its own
/// switch does not offer.
pub fn register(reg: &mut Registry<ViewerSpec>, spec: ViewerSpec) {
    assert!(
        spec.layouts.is_empty() || spec.layouts.contains(&spec.default_layout),
        "ext::viewer: viewer `{}` opens in {:?}, which its layouts do not offer",
        spec.id,
        spec.default_layout
    );
    reg.insert(ids::VIEWER, spec);
}

/// The base's own contributed viewers: none. Every viewer the base ships is a built-in variant of
/// [`ViewerKind`]. A second edition receives this on `Contributions` all the same, so registering
/// into it is the same call everywhere.
pub fn base_registry() -> Registry<ViewerSpec> {
    Registry::new()
}

struct Resolved {
    order: Vec<&'static ViewerSpec>,
    by_id: HashMap<SlotId, &'static ViewerSpec>,
}

static RESOLVED: OnceLock<Resolved> = OnceLock::new();

/// Installs the composed registry, before the first window. The base binary never calls this:
/// nothing installed resolves [`base_registry`] instead.
///
/// # Panics
/// If the order has already been resolved — a second `install`, or one after something asked.
pub fn install(reg: Registry<ViewerSpec>) {
    if RESOLVED.set(resolve(reg)).is_err() {
        panic!("ext::viewer: the viewer container was already resolved before install");
    }
}

fn resolve(reg: Registry<ViewerSpec>) -> Resolved {
    // Leaked deliberately and exactly once, the way every other container's is.
    let reg: &'static mut Registry<ViewerSpec> = Box::leak(Box::new(reg));
    let order = Registry::resolve(reg, GROUPS);
    let by_id = order.iter().map(|spec| (spec.id, *spec)).collect();
    Resolved { order, by_id }
}

fn resolved() -> &'static Resolved {
    RESOLVED.get_or_init(|| resolve(base_registry()))
}

/// Every contributed viewer, in registration order.
pub fn viewers() -> &'static [&'static ViewerSpec] {
    &resolved().order
}

/// One viewer by id, or `None` for an id nothing is registered under.
pub fn spec(id: SlotId) -> Option<&'static ViewerSpec> {
    resolved().by_id.get(&id).copied()
}

/// The first viewer claiming `path` as its default.
pub fn default_for(path: &str, head: Option<&str>) -> Option<SlotId> {
    viewers()
        .iter()
        .find(|spec| (spec.claims)(path, head) == Claim::Default)
        .map(|spec| spec.id)
}

/// Every viewer that would draw `path` — its default claimants and its offers — in registration
/// order.
pub fn offering(path: &str, head: Option<&str>) -> Vec<SlotId> {
    viewers()
        .iter()
        .filter(|spec| (spec.claims)(path, head) != Claim::No)
        .map(|spec| spec.id)
        .collect()
}

/// The `⋯` rows the viewer drawing `file` contributes, or none.
pub fn actions(app: &AppState, file: &OpenFile, cx: &App) -> Vec<ViewerAction> {
    match file.viewer {
        ViewerKind::Contributed(id) => spec(id)
            .and_then(|spec| spec.menu)
            .map(|menu| menu(app, file, cx))
            .unwrap_or_default(),
        // The two web editors carry their own theme, independent of the window's.
        ViewerKind::Excalidraw | ViewerKind::Drawio => {
            use crate::state::web_panel::WebTheme;
            let Some(app_id) = crate::web_export::bridge::web_app(file.viewer) else {
                return Vec::new();
            };
            let current = app.web_theme(app_id);
            let row = |mode: WebTheme,
                       run: fn(&mut AppState, &str, &mut Window, &mut Context<AppState>)| {
                let tick = if current == mode { "\u{2713} " } else { "" };
                ViewerAction {
                    label: format!("{tick}Theme: {}", mode.label()).into(),
                    enabled: true,
                    run,
                }
            };
            vec![
                row(WebTheme::Auto, |app, key, _w, cx| {
                    app.set_web_theme_for_tab(key, WebTheme::Auto, cx)
                }),
                row(WebTheme::Dark, |app, key, _w, cx| {
                    app.set_web_theme_for_tab(key, WebTheme::Dark, cx)
                }),
                row(WebTheme::Light, |app, key, _w, cx| {
                    app.set_web_theme_for_tab(key, WebTheme::Light, cx)
                }),
            ]
        }
        _ => Vec::new(),
    }
}

/// How much of a file's text a claim is shown: enough for a format's signature, never the file.
pub const HEAD_BYTES: usize = 4096;

/// The head of `text` a claim is handed, cut on a character boundary.
pub fn head(text: &str) -> &str {
    if text.len() <= HEAD_BYTES {
        return text;
    }
    let mut end = HEAD_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
