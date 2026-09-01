//! The surface a picture sits on: fills its panel, starts fitted, wheel zooms, drag pans.
//!
//! Mermaid and Excalidraw both draw a picture into a rectangle they do not own. The camera that
//! maps their coordinates onto that rectangle is [`crate::state::viewport`]; this module is the
//! rectangle — the hits, the wheel, the drag — and the one canvas that records the panel so a
//! later event has somewhere to be about.
//!
//! Double-click restores the fit. A pinch zooms about its centre. Nothing here paints the picture;
//! the caller adds that as a child.

use gpui::{
    App, Bounds, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Pixels, PinchEvent, Point, Rgba, ScrollDelta, ScrollWheelEvent,
    StatefulInteractiveElement, Styled, canvas, px,
};

use crate::app::AppState;
use crate::state::viewport::Content;
use crate::ui::eid;

/// The interactive frame a picture is drawn in.
///
/// The caller adds the picture as a child. A recording canvas sits under it so the panel's size
/// is known to the next wheel or drag; it paints nothing and takes no click.
pub fn surface(
    app: &AppState,
    key: &str,
    ground: Rgba,
    content: Content,
    cx: &mut Context<AppState>,
) -> gpui::Stateful<gpui::Div> {
    app.touch_viewport(key, content);

    let view = cx.entity();
    let key_measure = key.to_string();
    let key_wheel = key.to_string();
    let key_down = key.to_string();
    let key_move = key.to_string();
    let key_click = key.to_string();
    let key_pinch = key.to_string();

    super::surface()
        .id(eid("viewport", key))
        .bg(ground)
        .relative()
        .overflow_hidden()
        .cursor_grab()
        .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
            cx.stop_propagation();
            this.zoom_viewport(&key_wheel, wheel_factor(event.delta), event.position, cx);
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.start_viewport_drag(&key_down, event.position, cx);
            }),
        )
        .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
            if event.dragging() {
                this.drag_viewport(&key_move, event.position, cx);
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| this.end_viewport_drag(cx)),
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if event.click_count() >= 2 {
                this.reset_viewport(&key_click, cx);
            }
        }))
        .on_pinch(cx.listener(move |this, event: &PinchEvent, _, cx| {
            this.zoom_viewport(&key_pinch, 1.0 + event.delta, event.position, cx);
        }))
        .child(
            canvas(
                move |bounds, _, cx| {
                    view.update(cx, |this, cx| {
                        if this.note_viewport_panel(&key_measure, bounds) {
                            cx.notify();
                        }
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0()
            .size_full(),
        )
}

/// A wheel step as a zoom factor. Trackpad pixels and mouse notches both land in a range that
/// feels like a step rather than a leap; the camera clamps the resulting scale.
fn wheel_factor(delta: ScrollDelta) -> f32 {
    let y = match delta {
        ScrollDelta::Pixels(p) => f32::from(p.y),
        ScrollDelta::Lines(p) => p.y * 32.0,
    };
    1.0 + (y / 400.0).clamp(-0.5, 0.5)
}

/// Window coordinates as the numbers the camera speaks.
pub fn point(at: Point<Pixels>) -> (f32, f32) {
    (f32::from(at.x), f32::from(at.y))
}

/// A panel's size and origin as the numbers the camera speaks.
pub fn panel(bounds: Bounds<Pixels>) -> (f32, f32, f32, f32) {
    (
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
    )
}

/// A length the camera mapped, as GPUI wants it.
pub fn px_of(value: f32) -> gpui::Pixels {
    px(value)
}
