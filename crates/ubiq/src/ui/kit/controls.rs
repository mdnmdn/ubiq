//! The primitives the component library does not give us.
//!
//! Everything here is view-agnostic: interactive helpers take a plain click handler, so a call site
//! passes `cx.listener(...)` and the kit never learns which view it is drawing for.

use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, ClickEvent, Div, ElementId, FontWeight,
    InteractiveElement, IntoElement, ParentElement, PathBuilder, Pixels, Rgba, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Window, canvas, div, point, pulsating_between,
    px, relative,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::theme;
use crate::theme::{Family, Role};

/// A surface, in the shape everything in Ubiq is drawn in: square, filled, and identified by a
/// coloured left edge. Nothing here is rounded — the edge is what says what a thing is.
pub fn slab(edge: Rgba) -> Div {
    div()
        .flex()
        .flex_col()
        .bg(theme::surface())
        .border_l(px(theme::accent_edge()))
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
        .border_l(px(theme::accent_edge()))
        .border_color(colour);
    if focused {
        root = root.border_b(px(theme::accent_edge()));
    }
    root
}

/// A monospace run, at the size the chrome uses for paths, counts and code.
pub fn mono(text: impl Into<SharedString>, color: Rgba) -> Div {
    div()
        .font_family(theme::MONO_FONT)
        .text_size(theme::font(Family::Chrome, Role::Label))
        .text_color(color)
        .child(text.into())
}

/// The uppercase group heading used by the rail and the panel headers.
pub fn section_label(text: &str) -> impl IntoElement {
    div()
        .text_size(theme::font(Family::Chrome, Role::Micro))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::text_faint())
        .child(SharedString::from(text.to_uppercase()))
}

/// A 7px state dot with the soft ring that makes it readable against any surface.
///
/// A `Div` rather than `impl IntoElement`, because a caller that animates it — the lifecycle dot
/// pulsing while a turn wants the reader — needs an element to hang `with_animation` on.
pub fn status_dot(color: Rgba, ring: Rgba) -> Div {
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
        .border_l(px(theme::accent_edge()))
        .border_color(edge)
}

/// A [`pill`] that does something when it is clicked — what one item in a list the user put
/// there is drawn as, where taking it back out is not on offer.
///
/// **The colour is the caller's.** `fill` is the tag's ground, `edge` its left edge and `colour`
/// its text and glyph, so a tag can report something about itself — a warning, a state — without
/// this function knowing what it is reporting. Every one of them is a theme token; nothing here
/// computes a shade.
///
/// The label is elided rather than wrapped — a tag is one line high — and `tooltip` is what says
/// it in full, the same bargain [`elided_with`] makes.
///
/// `struck` draws the label struck through, on the sub-task list's own reasoning for a done row:
/// a colour and a line together are one signal read at a glance, not two competing ones.
///
/// [`removable_tag`] is this with a dismiss beside it, built from this one rather than written
/// twice: the two are the same object in two states of a list's life — before it is sent, and
/// after — and a second copy of the pill is how they drift apart.
pub fn tag(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tooltip: impl Into<SharedString>,
    fill: Rgba,
    edge: Rgba,
    colour: Rgba,
    struck: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let tip: SharedString = tooltip.into();

    pill(edge)
        .bg(fill)
        .h(px(22.))
        .px_2()
        .gap_1p5()
        .max_w(px(220.))
        .child(
            div()
                .id(id)
                .flex_shrink(1.0)
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(colour)
                .truncate()
                .when(struck, |this| this.line_through())
                .cursor_pointer()
                .child(label.into())
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                })
                .on_click(on_click),
        )
}

/// A [`tag`] that also goes away when its `×` is clicked — what a list of items the user put
/// there and can take back out is drawn as.
///
/// Two ids, because there are two things to click, and they are **siblings rather than nested**:
/// the label carries the tag's own click and the dismiss carries its own, so taking a tag off
/// never also does what clicking it does.
#[allow(clippy::too_many_arguments)]
pub fn removable_tag(
    id: impl Into<ElementId>,
    remove_id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tooltip: impl Into<SharedString>,
    fill: Rgba,
    edge: Rgba,
    colour: Rgba,
    struck: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_remove: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    tag(id, label, tooltip, fill, edge, colour, struck, on_click).child(
        div()
            .id(remove_id)
            .size(px(16.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .child(
                Icon::new(IconName::Close)
                    .with_size(Size::XSmall)
                    .text_color(colour),
            )
            .on_click(on_remove),
    )
}

/// A single-letter git badge.
pub fn badge(text: &str, color: Rgba) -> impl IntoElement {
    mono(SharedString::from(text.to_string()), color)
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .font_weight(FontWeight::SEMIBOLD)
}

/// A square icon button. `active` is what the titlebar's panel toggles use to show a panel is open.
///
/// The icon is anything that converts into one, so a `gpui-component` variant and one of Ubiq's
/// own [`super::UbiqIcon`] rows both pass without the kit knowing which set it is drawing from.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: impl Into<Icon>,
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
        .text_size(theme::font(Family::Chrome, Role::Body))
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
    progress_ring_in(pct, diameter, theme::accent())
}

/// The same donut in a colour of the caller's choosing, for the rings that sit beside the context
/// one: two accent rings in a row read as one fact drawn twice, which is exactly what they are not.
pub fn progress_ring_in(pct: u8, diameter: f32, fill: Rgba) -> impl IntoElement {
    progress_rings(vec![(pct, fill)], diameter)
}

/// Two concentric donuts in one glyph, outermost band first — for the one place a single mark has
/// to carry two readings of the same kind, the account's rolling windows.
///
/// The bands are thinner than the single ring's, so the pair fits the diameter a row gives it
/// without either band reading as a blob. Which window is which is not inferable from the drawing,
/// so the caller's tooltip says it.
pub fn progress_ring_pair(outer: (u8, Rgba), inner: (u8, Rgba), diameter: f32) -> impl IntoElement {
    progress_rings(vec![outer, inner], diameter)
}

/// The donut painter both forms share: one arc per band, outermost first, each over its own track.
fn progress_rings(bands: Vec<(u8, Rgba)>, diameter: f32) -> impl IntoElement {
    let track = theme::text_faint();

    div().size(px(diameter)).flex_none().child(canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // One band takes the ring's full weight; a pair splits it, with a hair between them so
            // the two arcs stay two arcs at twelve pixels.
            let stroke = match bands.len() {
                0 | 1 => (diameter * 0.22).max(2.0),
                _ => (diameter * 0.15).max(1.5),
            };
            let gap = (stroke * 0.5).max(1.0);
            let centre = bounds.origin + point(px(diameter / 2.0), px(diameter / 2.0));

            let mut arc = |radius: f32, from: f32, to: f32, colour: Rgba| {
                if (to - from).abs() < f32::EPSILON || radius <= 0.0 {
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

            for (band, (pct, fill)) in bands.iter().enumerate() {
                let radius = (diameter - stroke) / 2.0 - band as f32 * (stroke + gap);
                arc(radius, 0.0, 1.0, track);
                arc(radius, 0.0, (*pct as f32 / 100.0).clamp(0.0, 1.0), *fill);
            }
        },
    ))
}

/// The status mark that carries two readings at once: a hexagon whose **border** is one colour
/// and whose **inner fill** is another.
///
/// **The outer hexagon has no fill of its own.** It is a stroke and nothing else, so whatever the
/// mark sits on shows through and the mark stays a mark rather than becoming a second background
/// for the block it is on. That is what lets the two colours be read as two facts: the border is
/// the execution's lifecycle, the fill is what it is doing or what came of it, and one changing
/// does not destroy the other's reading.
///
/// `fill` is `None` where the inner half has nothing to say — an activity nothing reports draws as
/// an empty outline, not as a guessed colour.
///
/// `pulse` asks the core — the fill alone, never the outline — for the same slow, shallow fade
/// [`crate::ui::conversation::lifecycle_dot`] gives a tab's dot: a hint that this one is still
/// going, at the edge of vision rather than an alarm. It is drawn as a second layered element for
/// exactly that reason — animating the whole mark would fade the outline's lifecycle reading along
/// with it, and that one never moves. `id` is the animation's own identity, so two marks on the
/// same screen never share a clock.
///
/// A hexagon rather than a circle or the window's own square: a status is the one mark on a card
/// that is *not* a surface, and giving it the only non-rectilinear silhouette in the window is
/// what makes it findable at a glance without a radius (this window draws no radii,
/// `crates/ubiq/src/theme.rs`).
pub fn hex_mark(
    id: impl Into<ElementId>,
    border: Rgba,
    fill: Option<Rgba>,
    side: f32,
    pulse: bool,
) -> AnyElement {
    // A flat-top hexagon: it sits better beside a line of text than a pointy-top one, whose spare
    // height would push the row it is in taller than the text beside it.
    let stroke = (side * 0.1).max(1.0);
    let corners_of = move |bounds: gpui::Bounds<Pixels>, radius: f32| {
        let centre = bounds.origin + point(px(side / 2.0), px(side / 2.0));
        (0..6)
            .map(|i| {
                let angle = i as f32 * std::f32::consts::TAU / 6.0;
                centre + point(px(angle.cos() * radius), px(angle.sin() * radius))
            })
            .collect::<Vec<_>>()
    };

    // The outline never animates — the lifecycle it carries is a static outer ring.
    let outline = div().size(px(side)).flex_none().child(canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let outer = corners_of(bounds, side / 2.0 - stroke / 2.0);
            let mut path = PathBuilder::stroke(px(stroke));
            path.move_to(outer[0]);
            for p in outer.iter().skip(1) {
                path.line_to(*p);
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, border);
            }
        },
    ));

    let Some(fill) = fill else {
        return outline.into_any_element();
    };

    // The core: the same hexagon at just over half the size, so the ring of ground between the two
    // is wide enough to read as a gap at eighteen pixels — layered over the outline rather than
    // painted in the same pass, so it can carry its own opacity animation.
    let core = div().absolute().inset_0().child(canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let points = corners_of(bounds, side * 0.28);
            let mut inner = PathBuilder::fill();
            inner.move_to(points[0]);
            for p in points.iter().skip(1) {
                inner.line_to(*p);
            }
            inner.close();
            if let Ok(inner) = inner.build() {
                window.paint_path(inner, fill);
            }
        },
    ));

    let core = if pulse {
        core.with_animation(
            id,
            Animation::new(Duration::from_millis(2_400))
                .repeat()
                .with_easing(pulsating_between(0.35, 1.0)),
            |this, delta| this.opacity(delta),
        )
        .into_any_element()
    } else {
        core.into_any_element()
    };

    div()
        .size(px(side))
        .flex_none()
        .child(outline)
        .child(core)
        .into_any_element()
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
        .child(
            mono(label, theme::text()).text_size(theme::font(Family::Chrome, Role::Meta) * scale),
        )
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
        .child(mono(label, text))
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
        .text_size(theme::font(Family::Chrome, Role::Body))
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
        .child(mono(label, text))
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
    size: Pixels,
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
    size: Pixels,
) -> Stateful<Div> {
    let text: SharedString = text.into();
    let full: SharedString = tooltip.into();

    div()
        .id(id)
        .flex_1()
        .min_w(px(0.))
        .text_size(size)
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
                .child(mono(label, theme::text_muted())),
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
