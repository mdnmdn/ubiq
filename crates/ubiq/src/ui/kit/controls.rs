//! The primitives the component library does not give us.
//!
//! Everything here is view-agnostic: interactive helpers take a plain click handler, so a call site
//! passes `cx.listener(...)` and the kit never learns which view it is drawing for.

use gpui::prelude::FluentBuilder;
use gpui::{
    App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement,
    PathBuilder, Rgba, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, canvas,
    div, point, px, relative,
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

/// The container every text entry sits in: a surface with its edge on the boundary and nothing
/// rounded.
///
/// A field is identified on the left like every other surface; when it holds the keyboard the
/// bottom edge lights as well, so the active box is the one that is underlined. This is the same
/// treatment the sink gives its fields, lifted into the kit where every input shares it.
pub fn field(edge: Rgba, focused: bool) -> Div {
    let colour = if focused { theme::border_focus() } else { edge };
    let mut root = div()
        .flex()
        .items_center()
        .bg(theme::surface())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(colour);
    if focused {
        root = root.border_b(px(theme::ACCENT_EDGE));
    }
    root
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

/// A slab that can be picked: it takes clicks, lights on hover, and says when it is the selected
/// one by borrowing the accent for its edge and a soft fill behind it.
///
/// The edge colour is the thing's own — a status, a project — and selection overrides it, because
/// "which one am I looking at" has to beat "what is this one doing" at a glance.
pub fn card(id: impl Into<ElementId>, edge: Rgba, selected: bool) -> Stateful<Div> {
    let mut root = slab(if selected { theme::accent() } else { edge })
        .id(id)
        .cursor_pointer();

    if selected {
        root = root.bg(theme::accent_soft());
    }

    root.hover(|this| this.bg(theme::hover()))
}

/// A state chip: a dot in the state's colour, then the word for it.
///
/// Colour and wording together, never colour alone — the same rule the explorer's git badges
/// follow, for the same reason.
///
/// `scale` is for the one caller that draws on a surface with a zoom: the graph, where a chip that
/// kept its size while its card shrank would stop fitting on it. Everywhere else passes `1.0`.
pub fn state_chip(label: impl Into<SharedString>, colour: Rgba, scale: f32) -> impl IntoElement {
    pill(colour)
        .h(px(22. * scale))
        .px(px(6. * scale))
        .gap(px(5. * scale))
        .child(
            div()
                .size(px(7. * scale))
                .flex_none()
                .rounded_full()
                .bg(colour),
        )
        .child(mono(label, theme::text()).text_size(px(11. * scale)))
}

/// A flat meter: how far along something is, as a bar rather than a number.
///
/// It sits beside the count rather than replacing it — a bar answers "nearly there?" at a glance
/// and a fraction answers "how many?", and a card is read at both distances.
pub fn meter(fraction: f32, colour: Rgba) -> impl IntoElement {
    let fraction = fraction.clamp(0.0, 1.0);
    div()
        .h(px(3.))
        .w_full()
        .flex()
        .flex_none()
        .bg(theme::fade(theme::text_faint(), 0.35))
        .child(div().h_full().w(relative(fraction)).bg(colour))
}

/// A chip that is also a choice: one of a row of values, exactly one of them lit.
///
/// The switch-shaped `toggle_pill` above it is for independent facets. This one is for a set the
/// user picks from, which is why the off state keeps its outline instead of draining to nothing.
pub fn choice_pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let (text, edge) = if active {
        (theme::text(), theme::accent())
    } else {
        (theme::text_muted(), theme::border())
    };

    let mut root = pill(edge).h(px(24.)).px_2p5().id(id).cursor_pointer();
    if active {
        root = root.bg(theme::accent_soft());
    }

    root.hover(|this| this.bg(theme::hover()))
        .child(mono(label, text).text_size(px(11.5)))
        .on_click(on_click)
}

/// The one filled button in the window: what a screen's single obvious action is drawn as.
pub fn primary_button(
    id: impl Into<ElementId>,
    icon: Option<IconName>,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let mut root = div()
        .id(id)
        .h(px(26.))
        .px_2p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .bg(theme::accent())
        .text_size(px(12.5))
        .text_color(theme::on_accent())
        .cursor_pointer()
        .hover(|this| this.bg(theme::accent_muted()));

    if let Some(icon) = icon {
        root = root.child(Icon::new(icon).with_size(Size::XSmall));
    }

    root.child(label.into()).on_click(on_click)
}

/// A chip that is also a switch: the filter pills over the graph, and anything else that is a set
/// of independent on/off facets rather than a choice between values.
///
/// Off is drawn as the same chip drained of colour rather than as a different shape, so turning
/// one back on does not move the row.
pub fn toggle_pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    colour: Rgba,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let (dot, text, edge) = if active {
        (colour, theme::text(), colour)
    } else {
        (theme::text_faint(), theme::text_faint(), theme::border())
    };

    pill(edge)
        .h(px(24.))
        .px_2p5()
        .gap_1p5()
        .id(id)
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(div().size(px(7.)).flex_none().rounded_full().bg(dot))
        .child(mono(label, text).text_size(px(11.5)))
        .on_click(on_click)
}

/// A tick box: what a row is chosen with when several may be.
///
/// Square like everything else, and the one place a small block fills with the accent — a tick is
/// read at a glance across a column, and an outline that only changed colour is not.
pub fn check_box(
    id: impl Into<ElementId>,
    checked: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let mut root = div()
        .id(id)
        .size(px(18.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .border_1()
        .cursor_pointer();

    root = match checked {
        true => root.bg(theme::accent()).border_color(theme::accent()),
        false => root.bg(theme::surface()).border_color(theme::border()),
    };

    root.hover(|this| this.border_color(theme::accent()))
        .when(checked, |this| {
            this.child(
                Icon::new(IconName::Check)
                    .with_size(Size::XSmall)
                    .text_color(theme::on_accent()),
            )
        })
        .on_click(on_click)
}

/// A run of text that gives up rather than wrapping, and says the whole of itself on hover.
///
/// **A name that does not fit is elided, never folded onto a second line.** A row in this interface
/// is one line high — a picker's rows, a path in a footer, a title in a card — so a long value that
/// wrapped would push everything under it down. What is cut off is not lost: the full string is the
/// element's tooltip, which is why this takes an id.
pub fn elided(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    colour: Rgba,
    size: f32,
) -> Stateful<Div> {
    let text: SharedString = text.into();
    let full = text.clone();
    elided_with(id, text, full, colour, size)
}

/// The same, where what is worth reading in full is not the run itself: a file's name is elided but
/// the whole **path** is what answers "which one is this", and the row already says the name.
pub fn elided_with(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    tooltip: impl Into<SharedString>,
    colour: Rgba,
    size: f32,
) -> Stateful<Div> {
    let text: SharedString = text.into();
    let full: SharedString = tooltip.into();

    div()
        .id(id)
        .flex_1()
        .min_w(px(0.))
        .text_size(px(size))
        .text_color(colour)
        .truncate()
        .child(text)
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(full.clone()).build(window, cx)
        })
}

/// A value between two nudges: `−`, what it currently reads, `+`.
///
/// The label is a string rather than a number because what a stepper steps is not always counted
/// in the same unit it is printed in.
pub fn stepper(
    id: &'static str,
    label: impl Into<SharedString>,
    on_down: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_up: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .child(icon_button((id, 0u32), IconName::Minus, false, on_down))
        .child(
            div()
                .w(px(46.))
                .flex()
                .justify_center()
                .child(mono(label, theme::text_muted()).text_size(px(11.5))),
        )
        .child(icon_button((id, 1u32), IconName::Plus, false, on_up))
}

/// A bar that opens and shuts what is under it: a chevron, a heading, and whatever summary the
/// caller wants readable while it is shut.
pub fn disclosure(
    id: impl Into<ElementId>,
    title: &str,
    summary: impl IntoElement,
    open: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .h(px(32.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(section_label(title))
        .child(summary)
        .child(div().flex_1().min_w(px(0.)))
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronUp
            })
            .with_size(Size::XSmall)
            .text_color(theme::text_faint()),
        )
        .on_click(on_toggle)
}
