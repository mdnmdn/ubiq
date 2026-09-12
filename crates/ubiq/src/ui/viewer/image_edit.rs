//! The annotation editor: a toolbar over the scene behind a picture.
//!
//! The picture is the scene painter's — the base image and every annotation, live on the
//! viewport camera — with one transparent layer above it for the tools. A gesture the layer
//! declines reaches the viewport's own underneath, so pan, zoom and fit keep working around
//! the tools rather than behind a mode.
//!
//! **Every image whose bytes decode gets this, not only a capture.** The header's View/Edit
//! toggle is what says which: View is the picture and its camera, Edit adds the tool layer over
//! it. A capture opens in Edit, a file from the explorer opens in View.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, Div, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Stateful,
    StatefulInteractiveElement, Styled, div, px,
};

use gpui_component::color_picker::ColorPicker;
use gpui_component::{Icon, IconName, Sizable, Size};

use crate::app::AppState;
use crate::state::editor::OpenFile;
use crate::state::image_edit::{ImageTool, STROKES};
use crate::state::scene::Rgba8;
use crate::state::{FileBody, viewport::Content};
use crate::theme;
use crate::ui::kit::{UbiqIcon, choice_pill, ghost_button, icon_button, mono};
use crate::ui::{eid, eid2};

/// The glyph a tool is drawn as. Seven are ours (`annotate-*`); copying a region is Lucide's own
/// `Copy`, which says it already.
fn tool_icon(tool: ImageTool) -> Icon {
    match tool {
        ImageTool::Select => Icon::new(UbiqIcon::AnnotateSelect),
        ImageTool::Crop => Icon::new(UbiqIcon::AnnotateCrop),
        ImageTool::Rectangle => Icon::new(UbiqIcon::AnnotateRect),
        ImageTool::Ellipse => Icon::new(UbiqIcon::AnnotateEllipse),
        ImageTool::Arrow => Icon::new(UbiqIcon::AnnotateArrow),
        ImageTool::Freehand => Icon::new(UbiqIcon::AnnotateDraw),
        ImageTool::Text => Icon::new(UbiqIcon::AnnotateText),
        ImageTool::Copy => Icon::new(IconName::Copy),
    }
}

/// A stroke colour as GPUI's own, and back. The picker speaks `Hsla`; the scene stores bytes,
/// because that is what an Excalidraw colour is.
pub fn stroke_hsla(colour: Rgba8) -> gpui::Hsla {
    gpui::Rgba {
        r: f32::from(colour.r) / 255.0,
        g: f32::from(colour.g) / 255.0,
        b: f32::from(colour.b) / 255.0,
        a: f32::from(colour.a) / 255.0,
    }
    .into()
}

pub fn stroke_rgba8(colour: gpui::Hsla) -> Rgba8 {
    let rgba = gpui::Rgba::from(colour);
    Rgba8 {
        r: (rgba.r * 255.0).round().clamp(0.0, 255.0) as u8,
        g: (rgba.g * 255.0).round().clamp(0.0, 255.0) as u8,
        b: (rgba.b * 255.0).round().clamp(0.0, 255.0) as u8,
        a: (rgba.a * 255.0).round().clamp(0.0, 255.0) as u8,
    }
}

/// An icon-only action, with the word it replaced as its tooltip.
fn tip(button: Stateful<Div>, label: &'static str) -> impl IntoElement + use<> {
    button.tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(label).build(window, cx))
}

/// The header strip a picture grows: the View/Edit toggle every image carries, and — in Edit —
/// the tools, the style and the history before it. The 32 pixels Markdown and Mermaid already
/// take, and the focus the keyboard needs to reach any of it.
pub fn toolbar(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> impl IntoElement {
    let key = file.key();
    let (tool, filled, width) = match &file.body {
        FileBody::ImageEdit(edit) => (edit.tool, edit.fill.is_some(), edit.stroke_width),
        _ => (ImageTool::Select, false, 2.0),
    };
    let editing = file.image_editing;

    // Left padding only: the View/Edit pair is flush with the strip's top, right and bottom
    // edges, and with each other.
    let mut row = div()
        .h(px(super::HEADER))
        .pl_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border());

    if editing {
        row = row.child(tools(app, &key, tool, filled, width, cx));
    }

    // The toggle every image carries, at the far end of whatever came before it. Its own row, so
    // the strip's gap falls outside the pair rather than between the two halves.
    row = row.child(div().flex_1());
    let mut modes = div().h_full().flex().flex_none().items_stretch();
    for (label, wanted) in [("View", false), ("Edit", true)] {
        let key = key.clone();
        let active = editing == wanted;
        let (text, ground) = match active {
            true => (theme::text(), theme::accent_soft()),
            false => (theme::text_muted(), theme::pane_bg()),
        };
        modes = modes.child(
            div()
                .id(eid2("img-mode", &key, label))
                .h_full()
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .border_l_1()
                .border_color(theme::border())
                .bg(ground)
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(mono(label, text))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.set_image_editing(&key, wanted, cx)),
                ),
        );
    }
    row.child(modes)
}

/// The tools, the style controls and the history — everything Edit adds to the strip.
fn tools(
    app: &AppState,
    key: &str,
    tool: ImageTool,
    filled: bool,
    width: f32,
    cx: &mut Context<AppState>,
) -> gpui::Div {
    let key = key.to_string();
    let mut row = div().flex().items_center().gap_1();
    for entry in ImageTool::all() {
        let key = key.clone();
        row = row.child(tip(
            icon_button(
                eid2("img-tool", &key, entry.label()),
                tool_icon(entry),
                tool == entry,
                cx.listener(move |this, _, _, cx| this.set_image_tool(&key, entry, cx)),
            ),
            entry.label(),
        ));
    }

    // The six the editor suggests sit in the picker's own popover, with its palettes and its hex
    // field behind them, so the control is one button rather than a cycle nobody can aim.
    row = row.child(
        ColorPicker::new(&app.image_stroke)
            .featured_colors(STROKES.iter().copied().map(stroke_hsla).collect())
            .with_size(Size::Small)
            .anchor(gpui::Anchor::TopLeft),
    );
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
    row = row.child(tip(
        icon_button(
            eid2("img-undo", &key, "undo"),
            IconName::Undo,
            false,
            cx.listener(move |this, _, _, cx| this.undo_image(&key_undo, cx)),
        ),
        "Undo",
    ));
    let key_redo = key.clone();
    row = row.child(tip(
        icon_button(
            eid2("img-redo", &key, "redo"),
            IconName::Redo,
            false,
            cx.listener(move |this, _, _, cx| this.redo_image(&key_redo, cx)),
        ),
        "Redo",
    ));
    let key_delete = key.clone();
    row = row.child(tip(
        icon_button(
            eid2("img-delete", &key, "delete"),
            IconName::Delete,
            false,
            cx.listener(move |this, _, _, cx| this.delete_image_selection(&key_delete, cx)),
        ),
        "Delete selection",
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
    // Viewing draws the same scene with no tool layer over it, so nothing takes a gesture the
    // camera wants.
    if !file.image_editing {
        return super::scene::live_with_overlay(app, &key, &scene, div().into_any_element(), cx);
    }
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
        // A tool that draws owns the mouse outright — the camera underneath keeps the wheel and
        // nothing else — so a press cannot land on pan however the two layers are ordered. Select
        // is the one tool that shares: it declines empty space, and that gesture is a pan.
        .when(tool != ImageTool::Select, |this| {
            this.block_mouse_except_scroll()
                .cursor(gpui::CursorStyle::Crosshair)
        })
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
