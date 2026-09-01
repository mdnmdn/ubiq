//! One fixture document, drawn by the viewer its name implies.
//!
//! **Every viewer is reached through `ui::viewer`, and none of them is copied here.** The point of
//! the page is that the special viewers can be exercised with no project open and no file read: a
//! fixture is a name and a `&'static str`, the name picks the viewer by exactly the rule a real
//! path goes through, and what the viewer is handed is the buffer's current text — so typing in the
//! source half redraws the preview half, which is the behaviour worth testing.
//!
//! What is *not* reused is [`crate::state::editor::OpenFile`]. A fixture has no path, no version
//! and nothing to save, and dirty against a baseline is meaningless for a document that is a
//! constant. So the layout toggle is the sink's own three pills over the sink's own state, and the
//! chrome stops there.

use gpui::{AnyElement, Context, Entity, IntoElement, ParentElement, Styled, div, px, relative};
use gpui_component::input::{Editor, EditorState};

use crate::app::AppState;
use crate::state::editor::{ViewLayout, ViewerKind};
use crate::state::sink::SinkDoc;
use crate::theme;
use crate::ui::eid2;
use crate::ui::kit::{choice_pill, mono};
use crate::ui::viewer;

pub fn render(app: &AppState, doc: &'static SinkDoc, cx: &mut Context<AppState>) -> AnyElement {
    let mut root = viewer::surface();
    root = root.child(header(app, doc, cx));
    root.child(body(app, doc, cx)).into_any_element()
}

/// What the document is called, and which of its viewer's layouts it is in.
///
/// The name is drawn even for the viewer with no layouts to toggle between, because on this page it
/// is the only thing that says which fixture is on screen.
fn header(app: &AppState, doc: &'static SinkDoc, cx: &mut Context<AppState>) -> impl IntoElement {
    let current = app.sink.layout(doc);
    let toggles = doc.viewer().has_preview();

    div()
        .h(px(32.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .child(mono(doc.name, theme::text_muted()).text_size(px(11.5)))
        .child(div().flex_1().min_w(px(0.)))
        .children(toggles.then(|| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .children(ViewLayout::all().map(|layout| {
                    choice_pill(
                        eid2("sink-layout", doc.key, layout.label()),
                        layout.label(),
                        current == layout,
                        cx.listener(move |this, _, _, cx| this.set_sink_layout(doc, layout, cx)),
                    )
                }))
        }))
}

/// The source, what the viewer drew, or the two side by side.
fn body(app: &AppState, doc: &'static SinkDoc, cx: &mut Context<AppState>) -> AnyElement {
    let Some(state) = app.sink_buffer(doc.key) else {
        // Reached only in the frame between the window opening and its buffers existing.
        return viewer::note("\u{2026}", theme::text_faint());
    };

    // The buffer rather than the constant: an edit in the source half has to reach the preview.
    let source = state.read(cx).value().to_string();
    let drawn = || match doc.viewer() {
        ViewerKind::Markdown => viewer::markdown::render(app, doc.key, &source),
        ViewerKind::Mermaid => viewer::diagram::render(app, doc.key, &source, cx),
        ViewerKind::Excalidraw => viewer::scene::live(app, doc.key, &source, cx),
        // The plain buffer has no preview, so this page never asks it for one.
        ViewerKind::Editor | ViewerKind::Image => {
            viewer::note("Nothing to draw", theme::text_faint())
        }
    };

    match app.sink.layout(doc) {
        ViewLayout::Source => buffer(state),
        ViewLayout::Preview => drawn(),
        ViewLayout::Split => div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .child(
                half(buffer(state))
                    .border_r_1()
                    .border_color(theme::border()),
            )
            .child(half(drawn()))
            .into_any_element(),
    }
}

/// The fixture's own buffer, at the size and inset the file editor draws one at.
fn buffer(state: &Entity<EditorState>) -> AnyElement {
    Editor::new(state)
        .h(relative(1.))
        .p_0()
        .border_0()
        .into_any_element()
}

fn half(child: AnyElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(child)
}
