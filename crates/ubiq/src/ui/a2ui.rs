//! Drawing an A2UI surface with Ubiq's own kit.
//!
//! **This renders a protocol Ubiq does not own.** The component set comes from
//! `_docs/references/a2ui-catalog.md` and the parse from [`crate::state::a2ui`]; what happens here
//! is one translation, component by component, into the primitives every other screen is built out
//! of. Nothing is invented for it — a card is `kit::card`, a tick is `kit::check_box`, a colour is a
//! token — so an A2UI surface looks like the rest of the window rather than like a second interface
//! smuggled inside it.
//!
//! **Every input is inert.** A2UI's inputs write into a data model, and the data model is not
//! parsed: a `TextField` has no buffer behind it, a `CheckBox` has nothing to flip, a `Button`'s
//! action names a handler on the agent's side that this half of the protocol never calls. So the
//! inputs are drawn at the value the payload carries and do nothing when clicked, and a property
//! bound to a JSON Pointer is drawn as the pointer, faintly, so a reader can tell a written value
//! from one that would arrive later. The one thing that does move is what the *preview* owns rather
//! than the surface: which tab of a `Tabs` is showing and whether a `Modal` is disclosed, both of
//! which the caller holds and passes back in.
//!
//! **Two platform limits.** GPUI has no video and no audio element, so `Video` and `AudioPlayer`
//! are framed placeholders naming their URL rather than players. And the crate carries no base64
//! decoder, so an `Image` whose URL is a `data:` URI is framed the same way — as is any `http(s)`
//! one, which would need a fetch this screen has no business making.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::state::a2ui::{Children, Component, Kind, MAX_DEPTH, Prop, Surface};
use crate::theme;
use crate::ui::kit::{check_box, field, ghost_button, meter, mono, pill, primary_button, slab};
use crate::ui::{eid, eid2};

/// A tab title was clicked: which `Tabs` component, and which of its entries.
///
/// `kit::IndexedAction`'s shape, plus the component id — a surface may hold several tab strips, and
/// an index alone would not say which one moved.
pub type TabAction = Rc<dyn Fn(SharedString, usize, &mut Window, &mut App)>;

/// A modal's trigger was clicked, naming the `Modal` to open or shut. `kit::Action` plus the id,
/// for the same reason.
pub type ModalAction = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;

/// What the preview holds on the surface's behalf, and how it is told to change.
///
/// A surface is data and is redrawn from the payload every frame; the two things a reader can move
/// are not in it. They are here instead, keyed by component id, with the callbacks the kit's own
/// shape uses — a plain closure, so nothing under `ui/` learns which view is drawing.
pub struct Ctx<'a> {
    /// The selected index of each `Tabs` component, by its id. A missing entry is the first tab.
    pub tabs: &'a HashMap<String, usize>,
    /// The `Modal` whose content is disclosed, if any.
    pub open_modal: Option<&'a str>,
    /// A tab title was clicked: the `Tabs` id, and the index within it.
    pub on_tab: TabAction,
    /// A modal's trigger was clicked: the `Modal` id, to be opened or shut.
    pub on_modal: ModalAction,
}

/// Draw a surface, from its root down.
pub fn surface(surface: &Surface, ctx: &Ctx) -> AnyElement {
    node(surface, &surface.root, 0, ctx)
}

/// One component and everything under it.
///
/// The depth counter is the cycle guard: the adjacency list may point two components at each other,
/// and a renderer that followed that would never return.
fn node(surface: &Surface, id: &str, depth: u32, ctx: &Ctx) -> AnyElement {
    if depth > MAX_DEPTH {
        return note(format!("‹too deep: {id}›"));
    }
    let Some(component) = surface.get(id) else {
        return missing(id);
    };

    match &component.kind {
        Kind::Text { text, variant } => {
            let (body, bound) = dynamic(text.as_ref());
            let caption = variant.as_deref() == Some("caption");
            let colour = if bound {
                theme::text_faint()
            } else if caption {
                theme::text_muted()
            } else {
                theme::text()
            };
            div()
                .text_size(px(if caption { 11.5 } else { 13. }))
                .text_color(colour)
                .child(body)
                .into_any_element()
        }

        // No base64 decoder is in the crate's graph, so a `data:` URI cannot be turned into the
        // bytes `Image::from_bytes` wants, and an `http(s)` one would need a fetch. Both are framed.
        Kind::Image {
            url, description, ..
        } => framed(
            IconName::Frame,
            description.as_deref().unwrap_or("Image"),
            &dynamic(url.as_ref()).0,
        ),

        Kind::Icon { name } => {
            let (name, bound) = dynamic(name.as_ref());
            let colour = if bound {
                theme::text_faint()
            } else {
                theme::text_muted()
            };
            match icon_for(&name) {
                Some(icon) => Icon::new(icon)
                    .with_size(Size::Small)
                    .text_color(colour)
                    .into_any_element(),
                // The catalog names 59 icons and Ubiq's set is not that set. An unmapped name is
                // shown as the name, rather than as a glyph that means something else.
                None => mono(format!("⬚ {name}"), theme::text_faint()).into_any_element(),
            }
        }

        // GPUI draws no video and plays no audio: there is no element to reach for, so both say
        // what they would have played.
        Kind::Video { url, .. } => framed(IconName::Play, "Video", &dynamic(url.as_ref()).0),
        Kind::AudioPlayer { url, .. } => {
            framed(IconName::Play, "AudioPlayer", &dynamic(url.as_ref()).0)
        }

        Kind::Row {
            children,
            justify,
            align,
            weight,
        } => container(
            surface, depth, ctx, children, false, justify, align, *weight,
        ),
        Kind::Column {
            children,
            justify,
            align,
            weight,
        } => container(surface, depth, ctx, children, true, justify, align, *weight),
        Kind::List {
            children,
            direction,
            align,
        } => container(
            surface,
            depth,
            ctx,
            children,
            direction.as_deref() != Some("horizontal"),
            &None,
            align,
            None,
        ),

        Kind::Card { child } => crate::ui::kit::card(eid("a2ui-card", id), theme::border(), false)
            .p_2()
            .gap_2()
            .child(match child {
                Some(child) => node(surface, child, depth + 1, ctx),
                None => note("empty card"),
            })
            .into_any_element(),

        Kind::Divider { axis } => match axis.as_deref() == Some("vertical") {
            true => div().w(px(1.)).flex_none().bg(theme::border()),
            false => div().h(px(1.)).w_full().flex_none().bg(theme::border()),
        }
        .into_any_element(),

        // The click does nothing: `action` names a handler on the agent's side of the protocol.
        Kind::Button { child, variant } => {
            let label = button_label(surface, child.as_deref());
            let id = eid("a2ui-button", id);
            match variant.as_deref() == Some("primary") {
                true => primary_button(id, None, label, |_, _, _| {}).into_any_element(),
                false => ghost_button(id, None, label, |_, _, _| {}).into_any_element(),
            }
        }

        Kind::TextField {
            label,
            value,
            placeholder,
            ..
        } => {
            let (value, bound) = dynamic(value.as_ref());
            let (shown, colour) = match value.is_empty() {
                true => (
                    SharedString::from(placeholder.clone().unwrap_or_default()),
                    theme::text_faint(),
                ),
                false => (
                    value,
                    if bound {
                        theme::text_faint()
                    } else {
                        theme::text()
                    },
                ),
            };
            labelled(
                label.as_deref(),
                field(theme::border(), false)
                    .h(px(26.))
                    .px_2()
                    .child(div().text_size(px(12.5)).text_color(colour).child(shown))
                    .into_any_element(),
            )
        }

        Kind::CheckBox { label, value } => {
            let checked = value
                .as_ref()
                .and_then(|value| value.literal().copied())
                .unwrap_or(false);
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(check_box(eid("a2ui-check", id), checked, |_, _, _| {}))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme::text())
                        .child(SharedString::from(label.clone().unwrap_or_default())),
                )
                .children(
                    value
                        .as_ref()
                        .and_then(|value| value.binding())
                        .map(binding),
                )
                .into_any_element()
        }

        Kind::Slider {
            label,
            value,
            min,
            max,
        } => {
            let min = min.unwrap_or(0.);
            let max = max.unwrap_or(1.);
            let value = value
                .as_ref()
                .and_then(|value| value.literal().copied())
                .unwrap_or(min);
            let span = max - min;
            let fraction = match span.abs() < f32::EPSILON {
                true => 0.,
                false => (value - min) / span,
            };
            labelled(
                label.as_deref(),
                div()
                    .w(px(220.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(meter(fraction, theme::accent()))
                    .child(mono(
                        format!("{value} ∈ [{min}, {max}]"),
                        theme::text_faint(),
                    ))
                    .into_any_element(),
            )
        }

        Kind::ChoicePicker {
            label,
            options,
            value,
            ..
        } => {
            let selected = value.as_ref().and_then(|value| value.literal());
            let pills = options.iter().map(|option| {
                let on = selected.is_some_and(|values| values.contains(&option.value));
                let edge = match on {
                    true => theme::accent(),
                    false => theme::border(),
                };
                let mut chip = pill(edge).h(px(24.)).px_2p5();
                if on {
                    chip = chip.bg(theme::accent_soft());
                }
                chip.child(
                    mono(
                        option.label.clone(),
                        match on {
                            true => theme::text(),
                            false => theme::text_muted(),
                        },
                    )
                    .text_size(px(11.5)),
                )
                .into_any_element()
            });
            labelled(
                label.as_deref(),
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1p5()
                    .children(pills)
                    .children(
                        value
                            .as_ref()
                            .and_then(|value| value.binding())
                            .map(binding),
                    )
                    .into_any_element(),
            )
        }

        // Both flags default to false, and an upstream fixture ships exactly that, so the neither
        // state is the ordinary one: no glyph, and a line saying what it means.
        Kind::DateTimeInput {
            label,
            value,
            enable_date,
            enable_time,
        } => {
            let (value, bound) = dynamic(value.as_ref());
            let colour = if bound {
                theme::text_faint()
            } else {
                theme::text()
            };
            let mut box_ = field(theme::border(), false).h(px(26.)).px_2().gap_1p5();
            if *enable_date {
                box_ = box_.child(
                    Icon::new(IconName::Calendar)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                );
            }
            if *enable_time {
                // Ubiq's set has no clock; the spinner's dial is the nearest honest stand-in.
                box_ = box_.child(
                    Icon::new(IconName::LoaderCircle)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                );
            }
            labelled(
                label.as_deref(),
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(box_.child(div().text_size(px(12.5)).text_color(colour).child(value)))
                    .when(!enable_date && !enable_time, |this| {
                        this.child(note("enableDate and enableTime are both false"))
                    })
                    .into_any_element(),
            )
        }

        Kind::Tabs { tabs } => {
            if tabs.is_empty() {
                return note("tabs with no entries");
            }
            let selected = ctx.tabs.get(id).copied().unwrap_or(0).min(tabs.len() - 1);
            let strip = tabs.iter().enumerate().map(|(index, tab)| {
                let active = index == selected;
                let on_tab = ctx.on_tab.clone();
                let owner = SharedString::from(id.to_string());
                div()
                    .id(eid2("a2ui-tab", id, index))
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .flex_none()
                    .items_center()
                    .border_b_2()
                    .border_color(match active {
                        true => theme::accent(),
                        false => theme::border(),
                    })
                    .text_size(px(12.5))
                    .text_color(match active {
                        true => theme::text(),
                        false => theme::text_muted(),
                    })
                    .cursor_pointer()
                    .hover(|this| this.bg(theme::hover()))
                    .child(SharedString::from(tab.title.clone()))
                    .on_click(move |_, window, cx| on_tab(owner.clone(), index, window, cx))
                    .into_any_element()
            });

            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().flex().children(strip))
                .child(node(surface, &tabs[selected].child, depth + 1, ctx))
                .into_any_element()
        }

        // Not `kit::modal`: a real overlay belongs to the page hosting this surface, and one is
        // painted at the window origin. Inline disclosure is what a preview can honestly show.
        Kind::Modal { trigger, content } => {
            let on_modal = ctx.on_modal.clone();
            let owner = SharedString::from(id.to_string());
            let open = ctx.open_modal == Some(id);
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .id(eid("a2ui-modal", id))
                        .cursor_pointer()
                        .child(match trigger {
                            Some(trigger) => node(surface, trigger, depth + 1, ctx),
                            None => note("modal with no trigger"),
                        })
                        .on_click(move |_, window, cx| on_modal(owner.clone(), window, cx)),
                )
                .when(open, |this| {
                    this.child(
                        slab(theme::accent())
                            .bg(theme::surface_raised())
                            .p_2()
                            .gap_2()
                            .child(match content {
                                Some(content) => node(surface, content, depth + 1, ctx),
                                None => note("modal with no content"),
                            }),
                    )
                })
                .into_any_element()
        }

        Kind::Unknown => note(format!("unsupported component: {}", component.kind.tag())),
    }
}

/// A row or a column of children, with A2UI's alignment words mapped onto the flex box.
#[allow(clippy::too_many_arguments)]
fn container(
    surface: &Surface,
    depth: u32,
    ctx: &Ctx,
    children: &Children,
    column: bool,
    justify: &Option<String>,
    align: &Option<String>,
    weight: Option<f32>,
) -> AnyElement {
    let mut root = div().flex().gap_2().when(column, |this| this.flex_col());

    root = match justify.as_deref() {
        Some("center") => root.justify_center(),
        Some("end") => root.justify_end(),
        Some("spaceBetween") => root.justify_between(),
        Some("spaceAround") => root.justify_around(),
        Some("spaceEvenly") => root.justify_evenly(),
        _ => root.justify_start(),
    };
    root = match align.as_deref() {
        Some("center") => root.items_center(),
        Some("end") => root.items_end(),
        Some("stretch") => root.items_stretch(),
        _ => root.items_start(),
    };
    if let Some(weight) = weight {
        root = root.flex_grow(weight);
    }

    // A template repeats one component per element the pointer finds, and the data model is not
    // parsed — so it is drawn once and the repeat is reported rather than guessed at.
    if let Some((path, component)) = children.template() {
        return root
            .child(node(surface, component, depth + 1, ctx))
            .child(note(format!(
                "template over {path} — drawn once, not repeated: the data model is not parsed"
            )))
            .into_any_element();
    }

    root.children(
        children
            .ids()
            .iter()
            .map(|id| node(surface, id, depth + 1, ctx)),
    )
    .into_any_element()
}

/// What to draw for a property, and whether it is a binding rather than a written value.
fn dynamic(prop: Option<&Prop<String>>) -> (SharedString, bool) {
    match prop {
        Some(prop) => (
            SharedString::from(prop.text().to_string()),
            prop.binding().is_some(),
        ),
        None => (SharedString::default(), false),
    }
}

/// A label above a control, in the shape the settings page uses.
fn labelled(label: Option<&str>, control: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(label.filter(|label| !label.is_empty()).map(|label| {
            div()
                .text_size(px(11.))
                .text_color(theme::text_muted())
                .child(SharedString::from(label.to_string()))
        }))
        .child(control)
        .into_any_element()
}

/// The pointer a bound property names, drawn as the pointer it is.
fn binding(path: &str) -> AnyElement {
    mono(format!("↳ {path}"), theme::text_faint())
        .text_size(px(11.))
        .into_any_element()
}

/// A faint aside: what the renderer is not doing, said where it would have been done.
fn note(text: impl Into<SharedString>) -> AnyElement {
    mono(text.into(), theme::text_faint())
        .text_size(px(11.))
        .into_any_element()
}

/// A child id nothing in the surface answers to. A payload may name one that never arrives.
fn missing(id: &str) -> AnyElement {
    mono(format!("‹missing: {id}›"), theme::danger()).into_any_element()
}

/// Media this build cannot play or decode: what it is, and where it would have come from.
fn framed(icon: IconName, title: &str, url: &str) -> AnyElement {
    slab(theme::border())
        .p_2()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(
                    Icon::new(icon)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme::text_muted())
                        .child(SharedString::from(title.to_string())),
                ),
        )
        .child(mono(url.to_string(), theme::text_faint()).text_size(px(11.)))
        .into_any_element()
}

/// What a button says: the text of its child, or the id of a child that is not one.
fn button_label(surface: &Surface, child: Option<&str>) -> SharedString {
    let Some(child) = child else {
        return SharedString::from("Button");
    };
    match surface.get(child) {
        Some(Component {
            kind: Kind::Text { text, .. },
            ..
        }) => dynamic(text.as_ref()).0,
        _ => SharedString::from(child.to_string()),
    }
}

/// The catalog's icon name, where Ubiq's set has one that means the same thing.
///
/// A2UI names 59 Material-flavoured icons and Ubiq draws a smaller, differently-shaped set. Only
/// the honest matches are here — a name with no equivalent is drawn as the name, because a glyph
/// that nearly means the right thing is worse than none.
fn icon_for(name: &str) -> Option<IconName> {
    Some(match name {
        "accountCircle" => IconName::CircleUser,
        "add" => IconName::Plus,
        "arrowBack" => IconName::ArrowLeft,
        "arrowForward" => IconName::ArrowRight,
        "calendarToday" | "event" => IconName::Calendar,
        "check" => IconName::Check,
        "close" => IconName::Close,
        "delete" => IconName::Delete,
        "download" => IconName::ArrowDown,
        "error" => IconName::CircleX,
        "favorite" => IconName::Heart,
        "favoriteOff" => IconName::HeartOff,
        "folder" => IconName::Folder,
        "info" => IconName::Info,
        "locationOn" => IconName::Map,
        "mail" => IconName::Inbox,
        "menu" => IconName::Menu,
        "moreHoriz" => IconName::Ellipsis,
        "moreVert" => IconName::EllipsisVertical,
        "notifications" => IconName::Bell,
        "pause" => IconName::Pause,
        "person" => IconName::User,
        "play" => IconName::Play,
        "refresh" => IconName::RotateCw,
        "search" => IconName::Search,
        "settings" => IconName::Settings,
        "share" => IconName::ExternalLink,
        "star" => IconName::Star,
        "starOff" => IconName::StarOff,
        "upload" => IconName::ArrowUp,
        "visibility" => IconName::Eye,
        "visibilityOff" => IconName::EyeOff,
        "warning" => IconName::TriangleAlert,
        _ => return None,
    })
}
