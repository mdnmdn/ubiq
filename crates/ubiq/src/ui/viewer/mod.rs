//! What draws a file when it is not plain text.
//!
//! **A viewer is a pure function of bytes and a kind.** Nothing here opens a file, resolves a path
//! or spawns anything: a viewer is handed content and draws it. Usually that content is the file's
//! own bytes; sometimes — a diff, a diagram — it is something the host made from them, which
//! changes where the work happens and not the rule.
//!
//! Which viewer draws a path is [`crate::state::editor::ViewerKind`]'s answer, and anything with no
//! viewer of its own is the editor — the general case rather than a fallback. The one piece of
//! state a viewer keeps is which of its layouts is on screen, which lives on the open file beside
//! its buffer rather than in here. A diagram or a scene also has a camera — pan and zoom — which
//! lives on the window, keyed by the tab, because it is not a property of the file.

pub mod diagram;
pub mod diff;
pub mod image;
pub mod markdown;
pub mod scene;
pub mod viewport;

use gpui::{
    AnyElement, Context, Entity, IntoElement, ParentElement, Rgba, SharedString, Styled, div, px,
    relative,
};
use gpui_component::input::{Editor, EditorState};

use crate::app::AppState;
use crate::state::editor::{ViewLayout, ViewerKind};
use crate::state::{FileBody, OpenFile};
use crate::theme;
use crate::ui::eid2;
use crate::ui::kit::{choice_pill, mono};

/// The strip the layout toggle sits in, above whatever the viewer drew.
const HEADER: f32 = 32.0;

/// One open file, drawn by the viewer its kind names.
///
/// This is the whole of what a file panel shows. The header comes first and is the viewer's only
/// chrome — a viewer with more than one layout says which one it is in, and one with a single
/// layout says nothing at all, because the editor and the image have nothing to toggle between.
pub fn render(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> AnyElement {
    let mut root = surface();
    if file.viewer.has_preview() {
        root = root.child(header(file, cx));
    }
    root.child(body(app, file, cx)).into_any_element()
}

/// The three-way toggle: source, what the viewer drew, or both.
///
/// Which one is on screen belongs to the file rather than to this row, so the click goes to
/// `AppState` and comes back as the file's own `layout` — which is also what the panel writes into
/// the dock's saved arrangement, so a document reopens as it was left.
fn header(file: &OpenFile, cx: &mut Context<AppState>) -> impl IntoElement {
    let key = file.key();
    let current = file.layout;

    div()
        .h(px(HEADER))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .justify_end()
        .gap_1()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .children(ViewLayout::all().map(|layout| {
            let key = key.clone();
            choice_pill(
                eid2("view-layout", &key, layout.label()),
                layout.label(),
                current == layout,
                cx.listener(move |this, _, _, cx| this.set_view_layout(&key, layout, cx)),
            )
            .h_full()
        }))
}

/// What the file is showing, which is not always what its viewer draws: a tab exists before its
/// bytes do, and a read that failed has to say so somewhere.
fn body(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> AnyElement {
    match &file.body {
        FileBody::Loading => note("Reading\u{2026}", theme::text_faint()),
        FileBody::Failed(reason) => note(reason.clone(), theme::danger()),
        FileBody::Binary => note("Not text \u{b7} nothing to show", theme::text_faint()),
        FileBody::Diff(diff) => diff::render(diff, file.layout),
        FileBody::Bytes(bytes) => image::render(bytes, &file.path),
        FileBody::Text { state, .. } => drawn(app, file, state, cx),
    }
}

/// The bytes, once they are here: the buffer, what the viewer made of it, or the two side by side.
fn drawn(
    app: &AppState,
    file: &OpenFile,
    state: &Entity<EditorState>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    // The editor is the general case rather than a fallback: an extension with no viewer of its
    // own lands here and gets the highlighted buffer.
    let key = file.key();
    let source = state.read(cx).value().to_string();

    // The viewer needs the project's editor chrome — the point size its files draw at — which is
    // a preference of the project it sits in. `None` means the default.
    let font_size = app.ui_font_size(cx);
    let buf = || buffer(state, font_size);

    // A viewer with no source/preview toggle draws one thing only. The editor is the general
    // case; an Excalidraw scene is preview-only — its source is a serialised document nobody
    // edits by hand — so it draws the scene and never a JSON buffer; an image's bytes are its
    // body.
    if !file.viewer.has_preview() {
        return match file.viewer {
            ViewerKind::Excalidraw => scene::live(app, &key, &source, cx),
            ViewerKind::Editor => buf(),
            ViewerKind::Image => note("Nothing to draw", theme::text_faint()),
            // Markdown and Mermaid do have the toggle, so these are unreachable here.
            ViewerKind::Markdown | ViewerKind::Mermaid => {
                markdown::render(app, &key, &source, font_size, file.frontmatter_open, cx)
            }
        };
    }

    let mut preview = || match file.viewer {
        ViewerKind::Markdown => {
            markdown::render(app, &key, &source, font_size, file.frontmatter_open, cx)
        }
        ViewerKind::Mermaid => diagram::render(app, &key, &source, cx),
        ViewerKind::Excalidraw => scene::live(app, &key, &source, cx),
        // `has_preview` names Markdown and Mermaid and nothing else.
        ViewerKind::Editor | ViewerKind::Image => note("Nothing to draw", theme::text_faint()),
    };

    match file.layout {
        ViewLayout::Source => buf(),
        ViewLayout::Preview => preview(),
        ViewLayout::Split => div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .child(half(buf()).border_r_1().border_color(theme::border()))
            .child(half(preview()))
            .into_any_element(),
    }
}

/// The file's own buffer. Never a copy of it: the source half of a split is the same entity the
/// source layout draws, so a toggle costs nothing and loses no undo history. It draws at the
/// project's point size, or the default when the project has no preference.
fn buffer(state: &Entity<EditorState>, font_size: Option<f32>) -> AnyElement {
    let mut editor = Editor::new(state).h(relative(1.)).p_0().border_0();
    if let Some(size) = font_size {
        editor = editor.text_size(px(size));
    }
    editor.into_any_element()
}

/// One side of a split, each taking half and neither pushing the other out.
fn half(child: AnyElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(child)
}

/// What a viewer says when it has nothing to draw yet, or cannot draw what it was given.
///
/// Centred and quiet: a viewer that failed says why in the same place the drawing would have been,
/// rather than leaving an empty box the reader has to interpret.
pub fn note(text: impl Into<SharedString>, colour: Rgba) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .p_4()
        .child(mono(text, colour))
        .into_any_element()
}

/// The frame every viewer's body is drawn in: it fills its panel, scrolls nothing by itself, and
/// carries the application's ground rather than the surface a panel header sits on.
pub fn surface() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
}
