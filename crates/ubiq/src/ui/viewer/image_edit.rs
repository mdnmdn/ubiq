//! The capture editor: a toolbar over the scene the session authored.
//!
//! The picture is the scene painter's — the base image and every annotation, live on the
//! viewport camera — with one transparent layer above it for the tools. A gesture the layer
//! declines reaches the viewport's own underneath, so pan, zoom and fit keep working around
//! the tools rather than behind a mode.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Styled, div, px,
};

use crate::app::AppState;
use crate::state::editor::OpenFile;
use crate::state::image_edit::ImageTool;
use crate::state::{FileBody, viewport::Content};
use crate::theme;
use crate::ui::kit::{choice_pill, ghost_button};
use crate::ui::{eid, eid2};

/// The header strip an editable capture grows: tools, style, undo. The 32 pixels Markdown and
/// Mermaid already take, and the focus the keyboard needs to reach any of it.
pub fn toolbar(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> impl IntoElement {
    let key = file.key();
    let (tool, stroke, filled, width) = match &file.body {
        FileBody::ImageEdit(edit) => (
            edit.tool,
            edit.stroke,
            edit.fill.is_some(),
            edit.stroke_width,
        ),
        _ => (ImageTool::Select, None, false, 2.0),
    };
    let _ = app;

    let mut row = div()
        .h(px(super::HEADER))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border());

    for entry in ImageTool::all() {
        let key = key.clone();
        row = row.child(
            choice_pill(
                eid2("img-tool", &key, entry.label()),
                entry.label(),
                tool == entry,
                cx.listener(move |this, _, _, cx| this.set_image_tool(&key, entry, cx)),
            )
            .h_full(),
        );
    }

    let key_stroke = key.clone();
    row = row.child(ghost_button(
        eid2("img-stroke", &key, "stroke"),
        None,
        match stroke {
            Some(colour) => format!("#{:02x}{:02x}{:02x}", colour.r, colour.g, colour.b),
            None => String::from("stroke"),
        },
        cx.listener(move |this, _, _, cx| this.cycle_image_stroke(&key_stroke, cx)),
    ));
    let key_width = key.clone();
    row = row.child(ghost_button(
        eid2("img-width", &key, "width"),
        None,
        format!("{width:.0}px"),
        cx.listener(move |this, _, _, cx| this.cycle_image_width(&key_width, cx)),
    ));
    let key_fill = key.clone();
    row = row.child(choice_pill(
        eid2("img-fill", &key, "fill"),
        "Fill",
        filled,
        cx.listener(move |this, _, _, cx| this.toggle_image_fill(&key_fill, cx)),
    ));

    let key_undo = key.clone();
    row = row.child(ghost_button(
        eid2("img-undo", &key, "undo"),
        None,
        "Undo",
        cx.listener(move |this, _, _, cx| this.undo_image(&key_undo, cx)),
    ));
    let key_redo = key.clone();
    row = row.child(ghost_button(
        eid2("img-redo", &key, "redo"),
        None,
        "Redo",
        cx.listener(move |this, _, _, cx| this.redo_image(&key_redo, cx)),
    ));
    let key_delete = key.clone();
    row = row.child(ghost_button(
        eid2("img-delete", &key, "delete"),
        None,
        "Delete",
        cx.listener(move |this, _, _, cx| this.delete_image_selection(&key_delete, cx)),
    ));
    row
}

/// The tab's body: the scene plus the tool layer, or what the bytes draw when they never
/// became a scene.
pub fn render(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> AnyElement {
    let FileBody::ImageEdit(edit) = &file.body else {
        // Bytes that never became a scene have no decodable picture; draw what the viewer can.
        return match &file.body {
            FileBody::Bytes(bytes) => super::image::render(bytes, &file.path),
            _ => super::note("Nothing to draw", theme::text_faint()),
        };
    };
    let key = file.key();
    let scene = edit.display_scene();
    let tool = edit.tool;
    let content = Content {
        min_x: scene.bounds.min_x,
        min_y: scene.bounds.min_y,
        width: scene.bounds.width(),
        height: scene.bounds.height(),
    };
    let key_down = key.clone();
    let key_move = key.clone();
    let key_up = key.clone();
    let overlay = div()
        .id(eid("image-tool", &key))
        .absolute()
        .inset_0()
        .size_full()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                if let Some(at) = to_scene(this, &key_down, content, event.position) {
                    this.image_down(key_down.clone(), tool, at, window, cx);
                }
            }),
        )
        .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
            if event.dragging()
                && let Some(at) = to_scene(this, &key_move, content, event.position)
            {
                this.image_move(&key_move, at, cx);
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                if let Some(at) = to_scene(this, &key_up, content, event.position) {
                    this.image_up(&key_up, at, cx);
                }
            }),
        );
    super::scene::live_with_overlay(app, &key, &scene, overlay.into_any_element(), cx)
}

/// A window point as a scene point, through the stored panel — the diagram viewer's device.
/// `None` is an unmeasured panel, and the gesture is declined until there is one.
fn to_scene(
    app: &AppState,
    key: &str,
    content: Content,
    position: Point<Pixels>,
) -> Option<(f32, f32)> {
    let viewport = app.viewport(key);
    if viewport.panel_w < 8.0 || viewport.panel_h < 8.0 {
        return None;
    }
    let camera = viewport.camera(content, viewport.panel_w, viewport.panel_h);
    if camera.scale <= 0.0 {
        return None;
    }
    Some((
        (f32::from(position.x) - viewport.origin_x - camera.offset_x) / camera.scale,
        (f32::from(position.y) - viewport.origin_y - camera.offset_y) / camera.scale,
    ))
}
