//! The near-fullscreen zoom modal (T-185): one picture, on the same pan-and-zoom camera a diagram
//! panel already draws with, plus a Copy button.
//!
//! Raised from the corner button [`super::zoom_button`] puts on a scaled-down image or diagram in
//! a markdown preview. One instance, on [`crate::state::overlay::Layer::ImageZoom`]'s own rule —
//! `crate::state::workbench::WorkbenchState::image_zoom` carries which picture.

use gpui::{
    AnyElement, ClickEvent, Context, ImageSource, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, img, px,
};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::overlay::Layer;
use crate::state::viewport::Content;
use crate::state::zoom::ImageZoom;
use crate::theme;
use crate::ui::kit::{ghost_button, icon_button, modal_sized, primary_button};

/// How much of the window the modal takes — near-fullscreen, with a margin the scrim reads through.
const MODAL_FRACTION: f32 = 0.92;

/// The camera key the modal's own viewport is kept under, distinct from the small inline picture's
/// so opening the modal does not fight the document's own camera over one entry.
fn camera_key(zoom: &ImageZoom) -> String {
    format!("zoom-modal::{}", zoom.key)
}

/// The modal, or nothing when it is not up. The caller — `ui::shell` — only calls this when
/// `app.workbench.image_zoom` is `Some`, the same shape every other one-at-a-time modal follows,
/// but reading the option here too keeps this function safe to call unconditionally.
pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(zoom) = app.workbench.image_zoom.clone() else {
        return div().into_any_element();
    };

    let viewport = window.viewport_size();
    let width = f32::from(viewport.width) * MODAL_FRACTION;
    let height = f32::from(viewport.height) * MODAL_FRACTION;

    let key = camera_key(&zoom);
    let content = Content::from_size(zoom.width, zoom.height);
    let picture = {
        let camera = {
            let vp = app.viewport(&key);
            vp.camera(content, vp.panel_w, vp.panel_h)
        };
        img(ImageSource::Image(zoom.image.clone()))
            .absolute()
            .left(px(camera.offset_x))
            .top(px(camera.offset_y))
            .w(px(zoom.width * camera.scale))
            .h(px(zoom.height * camera.scale))
            .into_any_element()
    };

    let body = super::viewport::surface(
        app,
        &key,
        theme::app_bg(),
        content,
        picture,
        div().into_any_element(),
        cx,
    );

    modal_sized(
        "image-zoom-modal",
        theme::accent(),
        width,
        Some(height),
        &zoom.title,
        body,
        footer(cx),
        crate::ui::dismiss(&cx.entity(), Layer::ImageZoom, |this, _, cx| {
            this.close_image_zoom(cx)
        }),
        window,
    )
}

/// Copy, plus the reminder of how to get back to fitted — the corner button already told the
/// reader wheel-zoom and drag work, and repeating that here would be the tooltip's job twice over.
fn footer(cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let close_view = view.clone();
    let copy_view = view;
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "image-zoom-close",
            None,
            "Close",
            move |_: &ClickEvent, _window: &mut Window, cx: &mut gpui::App| {
                close_view.update(cx, |this, cx| this.close_image_zoom(cx));
            },
        ))
        .child(primary_button(
            "image-zoom-copy",
            Some(IconName::Copy),
            "Copy image",
            move |_: &ClickEvent, _window: &mut Window, cx: &mut gpui::App| {
                copy_view.update(cx, |this, cx| this.copy_zoomed_image(cx));
            },
        ))
        .into_any_element()
}

/// The small corner button every scaled-down picture carries: opens [`render`] on this picture.
///
/// Built with the platform's own `window.root` lookup rather than `cx.listener`, because the two
/// call sites that need it — a fence's block renderer, handed only a `Window` and an `App` — have
/// no `Context<AppState>` to close over in the first place. See `ui/viewer/markdown.rs`'s own note
/// on what a block renderer is handed.
pub(crate) fn zoom_button(id: gpui::ElementId, target: ImageZoom) -> AnyElement {
    icon_button(
        id,
        IconName::Maximize,
        false,
        move |_: &ClickEvent, window: &mut Window, cx: &mut gpui::App| {
            if let Some(Some(app)) = window.root::<AppState>() {
                let target = target.clone();
                app.update(cx, |this, cx| this.open_image_zoom(target, cx));
            }
        },
    )
    .tooltip(|window, cx| {
        gpui_component::tooltip::Tooltip::new(
            "Zoom \u{2014} opens near-fullscreen, with pan, zoom and Copy",
        )
        .build(window, cx)
    })
    .into_any_element()
}
