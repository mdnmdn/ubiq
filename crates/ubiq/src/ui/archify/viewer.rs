//! The Archify viewer: a contributed viewer of the IDE editor (D4 = A, the base's viewer seam
//! `D204`). There is no Diagrams mode and no document area of its own: a `.archify` file is
//! a tab like any other, and this spec is what draws it.
//!
//! - **Claims.** `.archify` is the viewer's by default; any `.json` is only *offered*, under "Open
//!   with" in the explorer's right-click and the editor's `⋯`, so a JSON file is never hijacked.
//! - **Layouts.** Preview first and by default, then Source. The base draws the switch in the
//!   editor's top row and draws Source itself (the tab's own buffer, highlighted as JSON); this
//!   file draws Preview.
//! - **The file is the IDE's.** It reads, watches and saves the tab. `live_reload` has it re-read
//!   the visible tab when an agent writes the file and the tab holds no unsaved edit.
//! - **Preview** is the painted diagram of the buffer's text (`compiled.rs` compiles it off the
//!   UI thread, keyed by tab key and text hash; `paint.rs` draws the scene), the diagnostics
//!   panel under it (`panels.rs`). The status chip and the agent's last tool call sit in the
//!   editor's top row, left (`header_left`); Ask agent sits there before the Preview/Source pills
//!   (`header_right`) — the base's `ViewerSpec` header slots, no row of its own.
//! - **The `⋯` menu** adds Ask agent, New diagram, Fit, Show/Hide diagnostics, Find node, Next
//!   lens, Replay trace, Copy layout JSON and the chart theme (Auto / Dark / Light, the current
//!   one ticked, kept in the Diagrams settings): how the canvas is painted, independent of Ubiq's
//!   own theme (`theme.rs`). It also exports the tab's scene in that theme: Save as PNG / SVG
//!   (native save dialog, written off the UI thread), Copy as PNG (an image clipboard entry, 2x) /
//!   SVG (text) and Copy as JSON (the document's text).
//!
//! Everything the viewer keeps about a tab is in `state::Ui`, keyed by the tab's key.

use ubiq_archify::compile::{self, Opts, peek};
use ubiq_archify::layout_json;
use ubiq_archify::tokens::Token;
use gpui::{
    AnyElement, App, AppContext, ClickEvent, ClipboardItem, Context, IntoElement, ParentElement, Styled,
    Window, div,
};
use crate::app::AppState;
use crate::ext::viewer::{self, Claim, ViewerAction, ViewerCtx, ViewerSpec};
use crate::ext::Registry;
use crate::state::OpenFile;
use crate::state::editor::{FileLanguage, ViewLayout};
use crate::theme;
use crate::ui::viewer::{note, surface};

use crate::ui::archify::agent::{self, Notice};
use crate::state::archify::compiled::{self, Outcome};
use crate::state::archify::{self as state, picture_key, ui};
use crate::ui::archify::theme::ChartTheme;
use crate::state::archify::{animate, motion};
use crate::ui::archify::{finder, paint, panels};
use crate::ext::ids;


/// The positions of the editor's top row: the picture first, then the source. No Split.
const LAYOUTS: &[ViewLayout] = &[ViewLayout::Preview, ViewLayout::Source];

pub fn register(reg: &mut Registry<ViewerSpec>) {
    viewer::register(
        reg,
        ViewerSpec {
            id: ids::ARCHIFY_VIEWER,
            label: "Archify",
            claims,
            layouts: LAYOUTS,
            default_layout: ViewLayout::Preview,
            language: Some(FileLanguage::Json),
            live_reload: true,
            render,
            menu: Some(menu),
            header_left: Some(header_left),
            header_right: Some(header_right),
        },
    );
}

/// `.archify` opens here; every other `.json` is offered. A pure function of the path: the head of
/// the text is not consulted, so a JSON file that merely looks like a diagram is still the user's
/// to open with Archify.
pub fn claims(path: &str, _head: Option<&str>) -> Claim {
    if path.ends_with(compile::EXTENSION) {
        Claim::Default
    } else if path.ends_with(".json") {
        Claim::Offer
    } else {
        Claim::No
    }
}

// --------------------------------------------------------------------------------------------- //
// Preview
// --------------------------------------------------------------------------------------------- //

/// The Preview of one tab: picture, diagnostics.
fn render(ctx: &ViewerCtx<'_>, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let tab_key = ctx.file.key();
    let key = picture_key(&tab_key);
    let text = ctx.buffer.read(cx).value().to_string();
    compiled::drive(&tab_key, &ctx.file.path, &text, cx);

    let (outcome, scene, panel_open) = {
        let ui = ui(cx);
        let tab = ui.tab(&tab_key);
        (tab.compiled.clone(), tab.scene.clone(), tab.panel_open)
    };

    let (body, finder) = match &scene {
        Some(scene) => {
            // The picture resolves its colours in its own preset (P9.1, D38).
            crate::ui::archify::theme::set_preset(scene.preset);
            agent::drive_reveal(ctx.app, &key, cx);
            let trace = outcome.as_ref().is_some_and(|o| o.trace);
            let interaction = ui(cx).peek_view(&key);
            let overlay = animate::advance(ctx.app, &key, scene, trace, &interaction, window, cx);
            // After the frame: an interaction's camera ask starts a glide that needs its frames.
            motion::drive(&key, window, cx);
            (
                paint::preview(ctx.app, &key, scene.clone(), overlay, cx),
                finder::view(ctx.app, &key, scene, window, cx),
            )
        }
        None => (note(placeholder(outcome.as_deref()), theme::text_faint()), None),
    };

    // The finder's list is a sibling of the picture, not inside it (it holds its own keys).
    let mut root = surface()
        .child(body)
        .children(finder);
    if panel_open && let Some(outcome) = &outcome {
        root = root.child(panels::panel(&tab_key, ctx.buffer, outcome, cx));
    }
    root.into_any_element()
}

/// The left of the editor's top row: how the document fares (a chip that opens and closes the
/// diagnostics) and what the agent last did to it. Preview only — Source has no picture.
fn header_left(
    _app: &AppState,
    file: &OpenFile,
    cx: &mut Context<AppState>,
) -> AnyElement {
    if file.layout == ViewLayout::Source {
        return div().into_any_element();
    }
    let tab_key = file.key();
    let (outcome, call) = {
        let ui = ui(cx);
        (ui.tab(&tab_key).compiled.clone(), ui.call_for(&tab_key))
    };
    div()
        .h_full()
        .flex()
        .items_center()
        .child(panels::badge(&tab_key, outcome.as_deref()))
        .children(call.as_ref().map(agent::chip))
        .into_any_element()
}

/// The right of the top row, before the layout pills: Ask agent.
fn header_right(
    _app: &AppState,
    file: &OpenFile,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let ask_key = file.key();
    agent::ask_button(cx.listener(move |this, _: &ClickEvent, window, cx| {
        agent::ask(this, &ask_key, window, cx)
    }))
}

/// Why there is no picture: still compiling, errors to fix, or a type without a layout yet.
fn placeholder(outcome: Option<&Outcome>) -> &'static str {
    match outcome {
        None => "Compiling\u{2026}",
        Some(o) if o.errors() > 0 => "Fix the diagnostics below to see the picture",
        Some(_) => "No layout for this diagram type yet",
    }
}

// --------------------------------------------------------------------------------------------- //
// The `⋯` menu
// --------------------------------------------------------------------------------------------- //

/// The rows the editor's `⋯` adds for one tab, below "Open with". Asked every time the menu opens.
fn menu(_app: &AppState, file: &OpenFile, cx: &App) -> Vec<ViewerAction> {
    let tab = state::peek(cx).and_then(|ui| ui.tabs.get(&file.key()));
    // Fit, the diagnostics and the replay are about the picture, which Source does not show.
    let picture = file.layout != ViewLayout::Source;
    let has_scene = tab.is_some_and(|t| t.scene.is_some());
    let panel_open = tab.is_none_or(|t| t.panel_open);
    let traced = tab
        .and_then(|t| t.compiled.as_ref())
        .is_some_and(|o| o.trace);
    vec![
        ViewerAction {
            label: "Ask agent".into(),
            enabled: true,
            run: agent::ask,
        },
        ViewerAction {
            label: "New diagram\u{2026}".into(),
            enabled: true,
            run: |app, _key, window, cx| agent::ask_new(app, window, cx),
        },
        ViewerAction {
            label: "Fit".into(),
            enabled: picture && has_scene,
            run: |app, key, _window, cx| app.reset_viewport(&picture_key(key), cx),
        },
        ViewerAction {
            label: if panel_open {
                "Hide diagnostics"
            } else {
                "Show diagnostics"
            }
            .into(),
            enabled: picture,
            run: |_app, key, _window, cx| {
                ui(cx).toggle_panel(key);
                cx.notify();
            },
        },
        ViewerAction {
            label: "Find node\u{2026}".into(),
            enabled: picture && has_scene,
            run: |app, key, window, cx| finder::open(app, &picture_key(key), window, cx),
        },
        ViewerAction {
            label: "Next lens".into(),
            enabled: picture && has_scene,
            run: |app, key, _window, cx| animate::cycle_lens(app, &picture_key(key), cx),
        },
        ViewerAction {
            label: "Replay trace".into(),
            enabled: picture && traced,
            run: |_app, key, _window, cx| {
                ui(cx).motions.replay(&picture_key(key));
                cx.notify();
            },
        },
        ViewerAction {
            label: "Copy layout JSON".into(),
            enabled: true,
            run: copy_layout,
        },
        export_action("Save as PNG\u{2026}", has_scene, |a, k, _w, cx| save(a, k, Export::Png, cx)),
        export_action("Save as SVG\u{2026}", has_scene, |a, k, _w, cx| save(a, k, Export::Svg, cx)),
        export_action("Copy as PNG", has_scene, |a, k, _w, cx| copy(a, k, Export::Png, cx)),
        export_action("Copy as SVG", has_scene, |a, k, _w, cx| copy(a, k, Export::Svg, cx)),
        ViewerAction {
            label: "Copy as JSON".into(),
            enabled: true,
            run: |app, key, _window, cx| {
                match source_of(app, key, cx) {
                    Some((_, text)) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        say(app, "Diagram JSON copied.", cx);
                    }
                    None => say(app, "Nothing to copy.", cx),
                }
            },
        },
        chart_theme_action(ChartTheme::Auto, |app, _, _, cx| set_chart(app, ChartTheme::Auto, cx)),
        chart_theme_action(ChartTheme::Dark, |app, _, _, cx| set_chart(app, ChartTheme::Dark, cx)),
        chart_theme_action(ChartTheme::Light, |app, _, _, cx| set_chart(app, ChartTheme::Light, cx)),
    ]
}

fn export_action(
    label: &'static str,
    enabled: bool,
    run: fn(&mut AppState, &str, &mut Window, &mut Context<AppState>),
) -> ViewerAction {
    ViewerAction { label: label.into(), enabled, run }
}

#[derive(Clone, Copy, PartialEq)]
enum Export {
    Png,
    Svg,
}

impl Export {
    fn ext(self) -> &'static str {
        match self {
            Export::Png => "png",
            Export::Svg => "svg",
        }
    }
}

/// PNG export scale: 2x, so the picture stays crisp on a retina screen.
const PNG_SCALE: f32 = 2.0;

pub(crate) fn say(app: &mut AppState, text: &str, cx: &mut Context<AppState>) {
    app.raise_notification(agent::notification(Notice { text: text.into() }));
    cx.notify();
}

/// The tab's path (relative to the project) and its buffer text as it is now.
fn source_of(app: &AppState, key: &str, cx: &App) -> Option<(String, String)> {
    let file = app.file(key, cx)?;
    let buffer = file.buffer()?;
    Some((file.path.clone(), buffer.read(cx).value().to_string()))
}

/// The tab's scene drawn as SVG in the active chart theme, on the chart's own ground.
fn svg_of(key: &str, cx: &App) -> Option<String> {
    let scene = state::peek(cx)?.tabs.get(key)?.scene.clone()?;
    let palette = crate::ui::archify::theme::chart_palette(scene.preset);
    let ground = crate::ui::archify::theme::css(palette.resolve(Token::Bg));
    Some(ubiq_archify::svg::scene_to_svg(
        &scene,
        &|t| crate::ui::archify::theme::css(palette.resolve(t)),
        Some(&ground),
    ))
}

/// Render `svg` to the bytes of `kind`; PNG rasterisation is the slow part, so callers run this
/// off the UI thread.
fn encode(svg: String, kind: Export) -> Result<Vec<u8>, String> {
    match kind {
        Export::Png => ubiq_archify::svg::svg_to_png(&svg, PNG_SCALE).map_err(|e| e.to_string()),
        Export::Svg => Ok(svg.into_bytes()),
    }
}

/// "Save as ...": the native dialog, render off the UI thread, then the host writes the file.
fn save(app: &mut AppState, key: &str, kind: Export, cx: &mut Context<AppState>) {
    let Some(svg) = svg_of(key, cx) else {
        return say(app, "Nothing to export: the diagram has no picture yet.", cx);
    };
    let rel = app.file(key, cx).map(|f| f.path.clone()).unwrap_or_default();
    let rel = std::path::Path::new(&rel);
    let root = app
        .project_snapshot(cx)
        .map(|s| s.record.path.clone())
        .unwrap_or_default();
    let dir = std::path::Path::new(&root).join(rel.parent().unwrap_or(std::path::Path::new("")));
    let stem = rel.file_stem().and_then(|s| s.to_str()).unwrap_or("diagram");
    let name = format!("{stem}.{}", kind.ext());
    let chosen = cx.prompt_for_new_path(&dir, Some(&name));
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(path))) = chosen.await else {
            return;
        };
        let bytes = cx.background_spawn(async move { encode(svg, kind) }).await;
        this.update(cx, |app, cx| match bytes {
            Ok(bytes) => {
                // The host writes the file; `HostFileExported` / `HostFileError` answer (wire.rs).
                let path = path.to_string_lossy().into_owned();
                ui(cx).exports.insert(path.clone());
                app.save_host_file_as(path, bytes);
            }
            Err(e) => say(app, &format!("Export failed: {e}"), cx),
        })
        .ok();
    })
    .detach();
}

/// "Copy as ...": SVG goes as text, PNG as an image entry once rasterised.
fn copy(app: &mut AppState, key: &str, kind: Export, cx: &mut Context<AppState>) {
    let Some(svg) = svg_of(key, cx) else {
        return say(app, "Nothing to export: the diagram has no picture yet.", cx);
    };
    if kind == Export::Svg {
        cx.write_to_clipboard(ClipboardItem::new_string(svg));
        return say(app, "SVG copied.", cx);
    }
    cx.spawn(async move |this, cx| {
        let png = cx.background_spawn(async move { encode(svg, kind) }).await;
        this.update(cx, |app, cx| match png {
            Ok(bytes) => {
                let image = gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes);
                cx.write_to_clipboard(ClipboardItem::new_image(&image));
                say(app, "PNG copied.", cx);
            }
            Err(e) => say(app, &format!("Export failed: {e}"), cx),
        })
        .ok();
    })
    .detach();
}

/// One "Chart theme: X" row, ticked when X is the one in force. Always enabled: it is app-wide.
fn chart_theme_action(
    theme: ChartTheme,
    run: fn(&mut AppState, &str, &mut Window, &mut Context<AppState>),
) -> ViewerAction {
    let tick = if crate::ui::archify::theme::chart_theme() == theme { "\u{2713} " } else { "" };
    ViewerAction {
        label: format!("{tick}Chart theme: {}", theme.label()).into(),
        enabled: true,
        run,
    }
}

fn set_chart(app: &mut AppState, theme: ChartTheme, cx: &mut Context<AppState>) {
    crate::ui::archify::theme::set_chart_theme(theme);
    app.update_archify_settings(
        |s| s.chart_theme = (theme != ChartTheme::Auto).then(|| theme.as_str().into()),
        cx,
    );
    cx.refresh_windows();
}

/// The layout report of the tab (`archify_layout`'s JSON) onto the clipboard, from the buffer's
/// text as it is now. Pure core code run once on the click, so it is allowed here.
fn copy_layout(app: &mut AppState, key: &str, _window: &mut Window, cx: &mut Context<AppState>) {
    let Some((rel, text)) = app.file(key, cx).and_then(|file| {
        let buffer = file.buffer()?;
        Some((file.path.clone(), buffer.read(cx).value().to_string()))
    }) else {
        return;
    };
    let quality = ui(cx).settings.quality.clone();
    let doc_type = peek(&text)
        .diagram_type
        .or_else(|| compile::type_from_name(&rel));
    let opts = Opts {
        input: &rel,
        doc_type: doc_type.as_deref(),
        quality: quality.as_deref(),
    };
    let said = match layout_json::by_type(&text, &opts) {
        Some(reply) => {
            let json = serde_json::to_string_pretty(&reply.json()).unwrap_or_default();
            cx.write_to_clipboard(ClipboardItem::new_string(json));
            "Layout JSON copied."
        }
        None => "No layout JSON: the document names no diagram type.",
    };
    app.raise_notification(agent::notification(Notice { text: said.into() }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archify_files_open_here_and_other_json_is_only_offered() {
        assert_eq!(claims("docs/flow.archify", None), Claim::Default);
        assert_eq!(claims("flow.archify", Some("{}")), Claim::Default);
        // The old name is still a JSON file: offered, never claimed.
        assert_eq!(claims("a/flow.dataflow.json", None), Claim::Offer);
        assert_eq!(claims("package.json", Some("{\"name\":1}")), Claim::Offer);
        assert_eq!(claims("notes.md", None), Claim::No);
        assert_eq!(claims("flow.archify.bak", None), Claim::No);
    }

    #[test]
    fn the_spec_offers_preview_then_source_and_opens_in_preview() {
        // The order is what the top row draws; there is no Split.
        assert_eq!(LAYOUTS, [ViewLayout::Preview, ViewLayout::Source]);
        // `register` panics when the default is not one of the layouts.
        let mut reg = crate::ext::Registry::new();
        register(&mut reg);
    }

    #[test]
    fn the_placeholder_says_why_there_is_no_picture() {
        assert!(placeholder(None).starts_with("Compiling"));
        let clean = Outcome {
            ok: true,
            profile: "standard",
            diagnostics: Vec::new(),
            scene: None,
            trace: false,
        };
        assert!(placeholder(Some(&clean)).starts_with("No layout"));
    }
}
