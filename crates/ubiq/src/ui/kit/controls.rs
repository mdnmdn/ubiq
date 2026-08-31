//! The primitives the component library does not give us.
//!
//! Everything here is view-agnostic: interactive helpers take a plain click handler, so a call site
//! passes `cx.listener(...)` and the kit never learns which view it is drawing for.

use gpui::{
    App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement,
    PathBuilder, Rgba, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, canvas,
    div, point, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::theme;

/// A surface, in the shape everything in Ubiq is drawn in: square, filled, and identified by a
/// coloured left edge. Nothing here is rounded — the edge is what says what a thing is.
pub fn slab(edge: Rgba) -> Div {
    div()
        .flex()
        .flex_col()
        .bg(theme::surface())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(edge)
}

/// A monospace run, at the size the chrome uses for paths, counts and code.
pub fn mono(text: impl Into<SharedString>, color: Rgba) -> Div {
    div()
        .font_family(theme::MONO_FONT)
        .text_size(px(12.))
        .text_color(color)
        .child(text.into())
}

/// The uppercase group heading used by the rail and the panel headers.
pub fn section_label(text: &str) -> impl IntoElement {
    div()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::text_faint())
        .child(SharedString::from(text.to_uppercase()))
}

/// A 7px state dot with the soft ring that makes it readable against any surface.
pub fn status_dot(color: Rgba, ring: Rgba) -> impl IntoElement {
    div()
        .size(px(13.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(ring)
        .child(div().size(px(7.)).rounded_full().bg(color))
}

/// The chip shared by the status strip, the git readout and the token readout. Its left edge
/// carries the colour of whatever it is reporting.
pub fn pill(edge: Rgba) -> Div {
    div()
        .h(px(26.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::surface())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(edge)
}

/// A single-letter git badge, or the muted `ignored` marker.
pub fn badge(text: &str, color: Rgba) -> impl IntoElement {
    mono(SharedString::from(text.to_string()), color)
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
}

/// A square icon button. `active` is what the titlebar's panel toggles use to show a panel is open.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let fg = if active {
        theme::accent()
    } else {
        theme::text_muted()
    };

    let mut root = div()
        .id(id)
        .size(px(30.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer();

    if active {
        root = root.bg(theme::accent_soft());
    }

    root.hover(|this| this.bg(theme::hover()))
        .child(Icon::new(icon).with_size(Size::Small).text_color(fg))
        .on_click(on_click)
}

/// A text button with no fill, used for `+ New chat` and the panel header actions.
pub fn ghost_button(
    id: impl Into<ElementId>,
    icon: Option<IconName>,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let mut root = div()
        .id(id)
        .h(px(26.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .text_size(px(12.5))
        .text_color(theme::text_muted())
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()).text_color(theme::text()));

    if let Some(icon) = icon {
        root = root.child(Icon::new(icon).with_size(Size::XSmall));
    }

    root.child(label.into()).on_click(on_click)
}

/// The context-window donut in the chat's status strip.
///
/// Drawn rather than approximated with a border, because the arc is the whole point of it.
pub fn progress_ring(pct: u8, diameter: f32) -> impl IntoElement {
    let fraction = (pct as f32 / 100.0).clamp(0.0, 1.0);
    let track = theme::text_faint();
    let fill = theme::accent();

    div().size(px(diameter)).flex_none().child(canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let stroke = (diameter * 0.22).max(2.0);
            let radius = (diameter - stroke) / 2.0;
            let centre = bounds.origin + point(px(diameter / 2.0), px(diameter / 2.0));

            let mut arc = |from: f32, to: f32, colour: Rgba| {
                if (to - from).abs() < f32::EPSILON {
                    return;
                }
                let mut path = PathBuilder::stroke(px(stroke));
                let steps = 48;
                for step in 0..=steps {
                    let t = from + (to - from) * (step as f32 / steps as f32);
                    // Start at twelve o'clock and sweep clockwise, as a progress ring reads.
                    let angle = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    let p = centre + point(px(angle.cos() * radius), px(angle.sin() * radius));
                    if step == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, colour);
                }
            };

            arc(0.0, 1.0, track);
            arc(0.0, fraction, fill);
        },
    ))
}
