//! Drawing an A2UI surface with Ubiq's own kit.
//!
//! **This renders a protocol Ubiq does not own.** The component set comes from
//! `_docs/references/a2ui-catalog.md` and the parse from [`crate::state::a2ui`]; what happens here
//! is one translation, component by component, into the primitives every other screen is built out
//! of. Nothing is invented for it — a card is `kit::card`, a tick is `kit::check_box`, a colour is
//! a token — so an A2UI surface looks like the rest of the window rather than like a second
//! interface smuggled inside it.
//!
//! **The catalog carries no colour, no padding and no position**, only `justify`, `align`, a
//! per-child `weight` and a discrete `variant` per component. That is not a gap: it is what makes
//! a payload from an untrusted producer safe to draw at all, because there is nothing in one that
//! could put a literal colour on screen. Every visual decision below is Ubiq's, made once.
//!
//! **The inputs are live.** A bound property is resolved against the data model the surface
//! carries, a keystroke writes back at the pointer it named, and a button resolves its action's
//! context and hands it up. None of that reaches an agent from here — the callbacks in [`Ctx`] are
//! plain closures, so this module never learns which view is drawing or what becomes of what it
//! produces.
//!
//! **Three platform limits.** GPUI has no video and no audio element, so `Video` and `AudioPlayer`
//! are framed placeholders naming their URL. `Image` is framed too, and that one is a rule rather
//! than a limit: nothing drawn from an agent's payload may fetch (`D115`). And the catalog's 59
//! icon names are not Ubiq's set, so a name with no honest equivalent is drawn as the name.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};
use serde_json::Value;

use crate::state::a2ui::live::Field;
use crate::state::a2ui::{Children, Component, Dynamic, Kind, MAX_DEPTH, Scope, Surface, scoped};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::eid2;
use crate::ui::kit::{
    card, check_box, choice_pill, field, ghost_button, icon_button, meter, mono, primary_button,
    slab, toggle_pill,
};

pub mod path;
pub mod registry;
pub mod svg;

/// A tab title was clicked: which `Tabs` component, and which of its entries.
///
/// `kit::IndexedAction`'s shape, plus the component id — a surface may hold several tab strips, and
/// an index alone would not say which one moved.
pub type TabAction = Rc<dyn Fn(SharedString, usize, &mut Window, &mut App)>;

/// A modal's trigger was clicked, naming the `Modal` to open or shut.
pub type ModalAction = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;

/// A button was clicked, named by its scoped key — the id, and the template row it was drawn for.
pub type FireAction = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;

/// An input changed: the **already resolved** absolute pointer, and what is now at it.
///
/// The pointer is resolved here rather than at the other end because only the renderer knows which
/// template row a control belongs to, and a scope cannot survive into a closure.
pub type WriteAction = Rc<dyn Fn(SharedString, Value, &mut Window, &mut App)>;

/// Everything drawing a surface needs that is not in the surface.
///
/// The data model and the field buffers are read; the four callbacks are how anything that happened
/// leaves. They are plain closures, in the kit's own shape, so nothing under `ui/` names `AppState`.
pub struct Ctx<'a> {
    pub surface: &'a Surface,
    pub model: &'a Value,
    /// One buffer per text input, by [`crate::state::a2ui::scoped`] key.
    pub fields: &'a HashMap<String, Field>,
    /// What blocked the last action, by the scoped key of the input that refused it. Empty until
    /// something has actually been submitted — a form does not accuse a reader of a mistake they
    /// have not had a chance to make.
    pub invalid: &'a HashMap<String, String>,
    /// Whether any check in the surface fails right now. Computed every frame, so a button that
    /// cannot fire says so before it is pressed.
    pub blocked: bool,
    pub tabs: &'a HashMap<String, usize>,
    pub open_modal: Option<&'a str>,
    pub on_tab: TabAction,
    pub on_modal: ModalAction,
    pub on_action: FireAction,
    pub on_value: WriteAction,
}

/// Draw a surface, from its root down.
pub fn render(ctx: &Ctx) -> AnyElement {
    node(ctx, &ctx.surface.root, 0, Scope::default())
}

/// One component and everything under it.
///
/// The depth counter is the cycle guard: the adjacency list may point two components at each other,
/// and a renderer that followed that would never return.
pub(crate) fn node(ctx: &Ctx, id: &str, depth: u32, scope: Scope<'_>) -> AnyElement {
    if depth > MAX_DEPTH {
        return note(format!("‹too deep: {id}›"));
    }
    let Some(component) = ctx.surface.get(id) else {
        return missing(id);
    };
    // Hidden means hidden. A placeholder saying something is hidden here would defeat the one
    // thing the property is for.
    if component
        .accessibility
        .as_ref()
        .is_some_and(|access| access.hidden)
    {
        return div().into_any_element();
    }

    chrome(draw(ctx, component, depth, scope), component, scope)
}

/// The two properties every component may carry whatever its type: how much of a row it takes, and
/// what it says to something that is not looking at it.
///
/// A wrapper rather than a builder call, because [`AnyElement`] cannot be restyled once it is
/// erased — and only a wrapper when there is something to say, so an ordinary component pays
/// nothing.
fn chrome(inner: AnyElement, component: &Component, scope: Scope<'_>) -> AnyElement {
    let label = component
        .accessibility
        .as_ref()
        .and_then(|access| access.label.clone())
        .filter(|label| !label.is_empty());
    if component.weight.is_none() && label.is_none() {
        return inner;
    }

    let mut wrapper = div()
        .flex()
        .flex_col()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(inner);
    if let Some(weight) = component.weight {
        wrapper = wrapper.flex_grow(weight);
    }
    match label {
        Some(label) => wrapper
            .id(eid2("a2ui-a11y", &component.id, scope.key()))
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(label.clone()).build(window, cx)
            })
            .into_any_element(),
        None => wrapper.into_any_element(),
    }
}

/// One component, as itself.
fn draw(ctx: &Ctx, component: &Component, depth: u32, scope: Scope<'_>) -> AnyElement {
    let id = component.id.as_str();
    let key = scoped(id, scope);

    match &component.kind {
        Kind::Text { text, variant } => {
            let body = display(ctx, text.as_ref(), scope);
            let caption = variant.as_deref() == Some("caption");
            let (role, colour) = match caption {
                true => (Role::Meta, theme::text_muted()),
                false => (Role::Body, theme::text()),
            };
            // The catalog says `Text` is a Markdown subset, and a heading drawn as its own hash is
            // the reason to honour that. A run with no markup in it skips the block layout.
            match markup(&body) {
                true => div()
                    .text_size(theme::font(Family::Chrome, role))
                    .text_color(colour)
                    .child(gpui_component::text::TextView::markdown(
                        eid2("a2ui-md", id, scope.key()),
                        body,
                    ))
                    .into_any_element(),
                false => div()
                    .text_size(theme::font(Family::Chrome, role))
                    .text_color(colour)
                    .child(body)
                    .into_any_element(),
            }
        }

        // Framed, and not because a decoder is missing: a surface drawn from an agent's payload
        // makes no outbound request, so there is nothing to draw but where the picture would have
        // come from. `D115`.
        Kind::Image {
            url,
            description,
            variant,
            ..
        } => {
            // `variant` still decides the box, because where a picture would have sat is layout
            // and the surface around it has to hold its shape. `fit` is read and not used: it says
            // how a picture fills its box, and there is no picture.
            let (width, height) = image_box(variant.as_deref());
            let title = description
                .as_ref()
                .map(|d| d.as_display_string(ctx.model, scope))
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| "Image".to_string());
            framed(IconName::Frame, &title, &display(ctx, url.as_ref(), scope))
                .pipe_size(width, height)
        }

        Kind::Icon { name } => {
            let value = name
                .as_ref()
                .map(|name| name.eval(ctx.model, scope))
                .unwrap_or(Value::Null);
            // A name, or an object carrying `svgPath`, or a binding that resolved to either.
            if let Some(d) = value.get("svgPath").and_then(Value::as_str) {
                return path::draw(eid2("a2ui-path", id, scope.key()), d, theme::text_muted());
            }
            let name = value.as_str().unwrap_or_default();
            match icon_for(name) {
                Some(icon) => Icon::new(icon)
                    .with_size(Size::Small)
                    .text_color(theme::text_muted())
                    .into_any_element(),
                // The catalog names 59 icons and Ubiq's set is not that set. An unmapped name is
                // shown as the name, rather than as a glyph that means something else.
                None => mono(format!("⬚ {name}"), theme::text_faint()).into_any_element(),
            }
        }

        // GPUI draws no video and plays no audio: there is no element to reach for, so both say
        // what they would have played.
        Kind::Video { url, .. } => {
            framed(IconName::Play, "Video", &display(ctx, url.as_ref(), scope)).pipe_none()
        }
        Kind::AudioPlayer { url, .. } => framed(
            IconName::Play,
            "AudioPlayer",
            &display(ctx, url.as_ref(), scope),
        )
        .pipe_none(),

        Kind::Row {
            children,
            justify,
            align,
        } => container(ctx, depth, scope, children, false, justify, align, 2.0),
        Kind::Column {
            children,
            justify,
            align,
        } => container(ctx, depth, scope, children, true, justify, align, 3.0),
        Kind::List {
            children,
            direction,
            align,
        } => container(
            ctx,
            depth,
            scope,
            children,
            direction.as_deref() != Some("horizontal"),
            &None,
            align,
            1.5,
        ),

        Kind::Card { child } => card(eid2("a2ui-card", id, scope.key()), theme::border(), false)
            .p_2()
            .gap_2()
            .child(match child {
                Some(child) => node(ctx, child, depth + 1, scope),
                None => note("empty card"),
            })
            .into_any_element(),

        Kind::Divider { axis } => match axis.as_deref() == Some("vertical") {
            // `self_stretch` rather than a full height: a divider has to draw whatever its row's
            // `align` says, and with no height of its own it drew nothing at all.
            true => div()
                .w(px(1.))
                .flex_none()
                .self_stretch()
                .min_h(px(8.))
                .bg(theme::border()),
            false => div().h(px(1.)).w_full().flex_none().bg(theme::border()),
        }
        .into_any_element(),

        Kind::Button {
            child,
            variant,
            action,
            ..
        } => button(
            ctx,
            id,
            &key,
            child.as_deref(),
            variant.as_deref(),
            action.is_some(),
            depth,
            scope,
        ),

        Kind::TextField {
            label,
            value,
            placeholder,
            variant,
            ..
        } => {
            let control = match ctx.fields.get(&key) {
                // A bound field: the buffer *is* the value at the pointer, so there is one copy
                // and a keystroke is already a write.
                Some(field_state) => field(theme::border(), false)
                    .h(px(26.))
                    .px_2()
                    .child(
                        Input::new(&field_state.input)
                            .appearance(false)
                            .text_size(theme::font(Family::Chrome, Role::Body)),
                    )
                    .into_any_element(),
                // A literal value has nowhere to write, and is drawn as the unwritable thing it is
                // rather than as a field that swallows keystrokes.
                None => {
                    let shown = display(ctx, value.as_ref(), scope);
                    let (shown, colour) = match shown.is_empty() {
                        true => (
                            display(ctx, placeholder.as_ref(), scope),
                            theme::text_faint(),
                        ),
                        false => (shown, theme::text()),
                    };
                    field(theme::border(), false)
                        .h(px(26.))
                        .px_2()
                        .child(
                            div()
                                .text_size(theme::font(Family::Chrome, Role::Body))
                                .text_color(colour)
                                .child(shown),
                        )
                        .into_any_element()
                }
            };
            let _ = variant;
            labelled(
                ctx,
                label.as_ref(),
                scope,
                control,
                value.as_ref(),
                ctx.invalid.get(&key),
            )
        }

        Kind::CheckBox { label, value, .. } => {
            let checked = value
                .as_ref()
                .map(|value| value.as_bool(ctx.model, scope))
                .unwrap_or(false);
            let write = value.as_ref().and_then(|value| value.write_target(scope));
            let on_value = ctx.on_value.clone();
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(check_box(
                            eid2("a2ui-check", id, scope.key()),
                            checked,
                            move |_, window, cx| {
                                if let Some(path) = &write {
                                    on_value(
                                        SharedString::from(path.clone()),
                                        Value::Bool(!checked),
                                        window,
                                        cx,
                                    );
                                }
                            },
                        ))
                        .child(
                            div()
                                .text_size(theme::font(Family::Chrome, Role::Body))
                                .text_color(theme::text())
                                .child(display(ctx, label.as_ref(), scope)),
                        ),
                )
                .children(binding_note(value.as_ref()))
                .children(invalid_note(ctx.invalid.get(&key)))
                .into_any_element()
        }

        Kind::Slider {
            label,
            value,
            min,
            max,
            steps,
            ..
        } => {
            let min = min.unwrap_or(0.);
            let max = max.unwrap_or(1.);
            let now = value
                .as_ref()
                .and_then(|value| value.as_f64(ctx.model, scope))
                .map(|v| v as f32)
                .unwrap_or(min);
            let span = max - min;
            let fraction = match span.abs() < f32::EPSILON {
                true => 0.,
                false => ((now - min) / span).clamp(0., 1.),
            };
            // The catalog's `steps` is the number of discrete divisions, so one nudge is one
            // division — and with none named, a hundredth of the span is a reasonable grain.
            let grain = match steps.unwrap_or(0) {
                0 => span / 100.0,
                steps => span / steps as f32,
            };
            let write = value.as_ref().and_then(|value| value.write_target(scope));
            let nudge = |delta: f32| {
                let write = write.clone();
                let on_value = ctx.on_value.clone();
                move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                    if let Some(path) = &write {
                        let next = (now + delta).clamp(min.min(max), max.max(min));
                        on_value(
                            SharedString::from(path.clone()),
                            serde_json::json!(next),
                            window,
                            cx,
                        );
                    }
                }
            };
            let control = div()
                .w(px(220.))
                .flex()
                .flex_col()
                .gap_1()
                .child(meter(fraction, theme::accent()))
                // A meter does not drag, so the value is moved the way every other number in the
                // window is moved. Stated as the limit it is: `kit::stepper` itself is not reached
                // for, because its two element ids are built from a `&'static str` and a component
                // inside a template needs an id carrying which repeat it is.
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .child(icon_button(
                            eid2("a2ui-slider-down", id, scope.key()),
                            IconName::Minus,
                            false,
                            nudge(-grain),
                        ))
                        .child(
                            div()
                                .w(px(46.))
                                .flex()
                                .justify_center()
                                .child(mono(format!("{now}"), theme::text_muted())),
                        )
                        .child(icon_button(
                            eid2("a2ui-slider-up", id, scope.key()),
                            IconName::Plus,
                            false,
                            nudge(grain),
                        )),
                )
                .child(mono(format!("{now} ∈ [{min}, {max}]"), theme::text_faint()))
                .into_any_element();
            labelled(
                ctx,
                label.as_ref(),
                scope,
                control,
                value.as_ref(),
                ctx.invalid.get(&key),
            )
        }

        Kind::ChoicePicker {
            label,
            options,
            value,
            variant,
            display_style,
            filterable,
            ..
        } => {
            let single = variant.as_deref() != Some("multipleSelection");
            let chosen = value
                .as_ref()
                .map(|value| value.as_string_list(ctx.model, scope))
                .unwrap_or_default();
            let write = value.as_ref().and_then(|value| value.write_target(scope));

            let rows = options.iter().enumerate().map(|(index, option)| {
                let text = option
                    .label
                    .as_ref()
                    .map(|label| label.as_display_string(ctx.model, scope))
                    .unwrap_or_default();
                let value = option
                    .value
                    .as_ref()
                    .map(|value| value.as_display_string(ctx.model, scope))
                    .unwrap_or_default();
                let on = chosen.iter().any(|held| held == &value);
                let next = next_choice(&chosen, &value, single, on);
                let write = write.clone();
                let on_value = ctx.on_value.clone();
                let click = move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                    if let Some(path) = &write {
                        on_value(
                            SharedString::from(path.clone()),
                            Value::Array(next.iter().cloned().map(Value::String).collect()),
                            window,
                            cx,
                        );
                    }
                };
                let element_id = eid2("a2ui-choice", format!("{id}{}", scope.key()), index);
                match display_style.as_deref() == Some("checkbox") {
                    // Ubiq draws no radio glyph. A column of tick boxes is honest, and it is the
                    // value binding rather than the widget that keeps a single choice single.
                    true => div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(check_box(element_id, on, click))
                        .child(mono(
                            text,
                            match on {
                                true => theme::text(),
                                false => theme::text_muted(),
                            },
                        ))
                        .into_any_element(),
                    false => match single {
                        true => choice_pill(element_id, text, on, click).into_any_element(),
                        false => toggle_pill(element_id, text, theme::accent(), on, click)
                            .into_any_element(),
                    },
                }
            });

            let control = div()
                .flex()
                .when(display_style.as_deref() == Some("checkbox"), |this| {
                    this.flex_col().gap_1()
                })
                .when(display_style.as_deref() != Some("checkbox"), |this| {
                    this.flex_wrap().gap_1p5()
                })
                .children(rows)
                .into_any_element();

            let control = match filterable {
                // The bar is drawn because the payload asked for it; what it does not yet do is
                // filter, which is state this surface does not carry. Said, rather than faked.
                true => div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        field(theme::border(), false).h(px(24.)).px_2().child(
                            mono("filter\u{2026}", theme::text_faint())
                                .text_size(theme::font(Family::Chrome, Role::Meta)),
                        ),
                    )
                    .child(control)
                    .into_any_element(),
                false => control,
            };

            labelled(
                ctx,
                label.as_ref(),
                scope,
                control,
                value.as_ref(),
                ctx.invalid.get(&key),
            )
        }

        // Both flags default to false, and an upstream fixture ships exactly that, so the neither
        // state is the ordinary one: no glyph, and a line saying what it means.
        Kind::DateTimeInput {
            label,
            value,
            enable_date,
            enable_time,
            ..
        } => {
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
            let box_ = match ctx.fields.get(&key) {
                Some(field_state) => box_.child(
                    Input::new(&field_state.input)
                        .appearance(false)
                        .text_size(theme::font(Family::Chrome, Role::Body)),
                ),
                None => box_.child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text())
                        .child(display(ctx, value.as_ref(), scope)),
                ),
            };
            let control = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(box_)
                .when(!enable_date && !enable_time, |this| {
                    this.child(note("enableDate and enableTime are both false"))
                })
                .into_any_element();
            labelled(
                ctx,
                label.as_ref(),
                scope,
                control,
                value.as_ref(),
                ctx.invalid.get(&key),
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
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .text_color(match active {
                        true => theme::text(),
                        false => theme::text_muted(),
                    })
                    .cursor_pointer()
                    .hover(|this| this.bg(theme::hover()))
                    .child(
                        tab.title
                            .as_ref()
                            .map(|title| title.as_display_string(ctx.model, scope))
                            .unwrap_or_default(),
                    )
                    .on_click(move |_, window, cx| on_tab(owner.clone(), index, window, cx))
                    .into_any_element()
            });

            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().flex().children(strip))
                .child(node(ctx, &tabs[selected].child, depth + 1, scope))
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
                        .id(eid2("a2ui-modal", id, scope.key()))
                        .cursor_pointer()
                        .child(match trigger {
                            Some(trigger) => node(ctx, trigger, depth + 1, scope),
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
                                Some(content) => node(ctx, content, depth + 1, scope),
                                None => note("modal with no content"),
                            }),
                    )
                })
                .into_any_element()
        }

        // A component from a catalog other than the basic one. Ubiq's own is what the registry
        // answers for; anything else draws the placeholder the protocol asks for.
        Kind::Extension { component, props } => match registry::lookup(component) {
            Some(draw) => draw(&registry::ExtCtx {
                id,
                props,
                ctx,
                depth,
                scope,
            }),
            None => note(format!("unsupported component: {component}")),
        },
    }
}

/// A row or a column of children, with A2UI's alignment words mapped onto the flex box.
///
/// **`align` defaults to `stretch`**, which is the catalog's default and not the one this renderer
/// used to take — a `Column` of cards that does not fill its width is not what any payload written
/// against the specification expects.
#[allow(clippy::too_many_arguments)]
fn container(
    ctx: &Ctx,
    depth: u32,
    scope: Scope<'_>,
    children: &Children,
    column: bool,
    justify: &Option<String>,
    align: &Option<String>,
    gap: f32,
) -> AnyElement {
    let mut root = div()
        .flex()
        .gap(px(gap * 4.0))
        .when(column, |this| this.flex_col());

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
        Some("start") => root.items_start(),
        _ => root.items_stretch(),
    };

    // A template is one component drawn once per element the pointer finds, with that element as
    // the binding scope — which is what makes a relative path inside it resolve.
    if let Some((path, template)) = children.template() {
        let base = scope.resolve(path);
        let count = crate::state::a2ui::value::get(ctx.model, &base)
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        return root
            .children((0..count).map(|index| {
                let item = format!("{base}/{index}");
                node(
                    ctx,
                    template,
                    depth + 1,
                    Scope {
                        item: Some(&item),
                        index: Some(index),
                    },
                )
            }))
            .into_any_element();
    }

    root.children(
        children
            .ids()
            .iter()
            .map(|id| node(ctx, id, depth + 1, scope)),
    )
    .into_any_element()
}

/// A button, and the one place a surface can refuse to do what it was asked.
#[allow(clippy::too_many_arguments)]
fn button(
    ctx: &Ctx,
    id: &str,
    key: &str,
    child: Option<&str>,
    variant: Option<&str>,
    has_action: bool,
    depth: u32,
    scope: Scope<'_>,
) -> AnyElement {
    let element_id = eid2("a2ui-button", id, scope.key());
    let on_action = ctx.on_action.clone();
    let fired = SharedString::from(key.to_string());
    let click = move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
        on_action(fired.clone(), window, cx)
    };

    // The catalog's own guidance: a button's child is usually a `Text`, and an `Icon` only for an
    // icon-only button. An icon child drawn as a label would read as its component id, which is
    // the payload's private name for it and means nothing to anybody looking at the screen.
    if let Some(icon) = button_icon(ctx, child, scope) {
        let inner = icon_button(element_id, icon, false, click).into_any_element();
        return dimmed(inner, ctx.blocked && has_action);
    }

    let label = button_label(ctx, child, depth, scope);
    let inner = match variant {
        Some("primary") => primary_button(element_id, None, label, click).into_any_element(),
        // The catalog's third variant: a label that is a button, with no chrome around it.
        Some("borderless") => div()
            .id(element_id)
            .h(px(26.))
            .px_1()
            .flex()
            .flex_none()
            .items_center()
            .text_size(theme::font(Family::Chrome, Role::Body))
            .text_color(theme::accent())
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .child(label)
            .on_click(click)
            .into_any_element(),
        _ => ghost_button(element_id, None, label, click).into_any_element(),
    };

    dimmed(inner, ctx.blocked && has_action)
}

/// A failing check disables the action rather than firing it, which is the catalog's own
/// instruction — so a blocked button is drawn as one that will not go, before it is pressed.
fn dimmed(inner: AnyElement, blocked: bool) -> AnyElement {
    match blocked {
        true => div().opacity(0.5).child(inner).into_any_element(),
        false => inner,
    }
}

/// The glyph a button draws instead of a label, when its child is an `Icon` this build can draw.
fn button_icon(ctx: &Ctx, child: Option<&str>, scope: Scope<'_>) -> Option<IconName> {
    let Kind::Icon { name } = &ctx.surface.get(child?)?.kind else {
        return None;
    };
    // A `svgPath` glyph is not an `IconName`, so an icon-only button carrying one falls through to
    // the labelled shapes rather than losing its action to a button with nothing in it.
    icon_for(name.as_ref()?.eval(ctx.model, scope).as_str()?)
}

/// The list a choice writes back, for a click on one option.
fn next_choice(chosen: &[String], value: &str, single: bool, on: bool) -> Vec<String> {
    match (single, on) {
        // One of a set: picking is replacing, and picking the held one again keeps it. A choice
        // with no value is worse than a choice the reader cannot undo.
        (true, _) => vec![value.to_string()],
        (false, true) => chosen
            .iter()
            .filter(|held| *held != value)
            .cloned()
            .collect(),
        (false, false) => {
            let mut next = chosen.to_vec();
            next.push(value.to_string());
            next
        }
    }
}

/// What to draw for a property: whatever it evaluated to, as text.
fn display(ctx: &Ctx, prop: Option<&Dynamic>, scope: Scope<'_>) -> SharedString {
    SharedString::from(
        prop.map(|prop| prop.as_display_string(ctx.model, scope))
            .unwrap_or_default(),
    )
}

/// Whether a run of text carries any of the markup `Text`'s Markdown subset would render.
fn markup(text: &str) -> bool {
    text.contains(['*', '_', '`', '[', '#'])
}

/// A label above a control, the pointer it writes to under it, and what refused it.
fn labelled(
    ctx: &Ctx,
    label: Option<&Dynamic>,
    scope: Scope<'_>,
    control: AnyElement,
    bound: Option<&Dynamic>,
    invalid: Option<&String>,
) -> AnyElement {
    let label = display(ctx, label, scope);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .children((!label.is_empty()).then(|| {
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(label)
        }))
        .child(control)
        .children(binding_note(bound))
        .children(invalid_note(invalid))
        .into_any_element()
}

/// The pointer a bound property names, drawn as the pointer it is.
///
/// A bound property now draws its *value*, which is the whole point of the data model — so this is
/// the only thing left on the page that says where a keystroke went.
fn binding_note(bound: Option<&Dynamic>) -> Option<AnyElement> {
    bound.and_then(Dynamic::path).map(|path| {
        mono(format!("↳ {path}"), theme::text_faint())
            .text_size(theme::font(Family::Chrome, Role::Meta))
            .into_any_element()
    })
}

/// What a failing check said, under the input that failed it.
fn invalid_note(invalid: Option<&String>) -> Option<AnyElement> {
    invalid.map(|message| {
        mono(message.clone(), theme::danger())
            .text_size(theme::font(Family::Chrome, Role::Meta))
            .into_any_element()
    })
}

/// A faint aside: what the renderer is not doing, said where it would have been done.
pub(crate) fn note(text: impl Into<SharedString>) -> AnyElement {
    mono(text.into(), theme::text_faint())
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .into_any_element()
}

/// A refusal: what the surface would not draw, and why. Never a blank — a blank box cannot be told
/// apart from an empty drawing, and a security boundary that is invisible is not one.
pub(crate) fn refused(text: impl Into<SharedString>) -> AnyElement {
    slab(theme::danger())
        .p_2()
        .child(
            mono(text.into(), theme::danger()).text_size(theme::font(Family::Chrome, Role::Meta)),
        )
        .into_any_element()
}

/// A child id nothing in the surface answers to. A payload may name one that never arrives.
fn missing(id: &str) -> AnyElement {
    mono(format!("‹missing: {id}›"), theme::danger()).into_any_element()
}

/// The box an `Image` of each catalog variant occupies.
fn image_box(variant: Option<&str>) -> (f32, Option<f32>) {
    match variant {
        Some("icon") => (theme::A2UI_IMAGE_ICON, Some(theme::A2UI_IMAGE_ICON)),
        Some("avatar") => (theme::A2UI_IMAGE_AVATAR, Some(theme::A2UI_IMAGE_AVATAR)),
        Some("smallFeature") => (theme::A2UI_IMAGE_SMALL, None),
        Some("largeFeature") => (theme::A2UI_IMAGE_LARGE, None),
        Some("header") => (theme::A2UI_IMAGE_LARGE, Some(theme::A2UI_IMAGE_HEADER_H)),
        _ => (theme::A2UI_IMAGE_MEDIUM, None),
    }
}

/// Media this surface will not fetch: what it is, and where it would have come from.
fn framed(icon: IconName, title: &str, url: &str) -> gpui::Div {
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
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text_muted())
                        .child(SharedString::from(title.to_string())),
                ),
        )
        .child(
            mono(url.to_string(), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Meta)),
        )
}

/// Sizing a framed placeholder, which is layout even when there is no picture in it.
trait PipeSize {
    fn pipe_size(self, width: f32, height: Option<f32>) -> AnyElement;
    fn pipe_none(self) -> AnyElement;
}

impl PipeSize for gpui::Div {
    fn pipe_size(self, width: f32, height: Option<f32>) -> AnyElement {
        let mut el = self.flex_none().w(px(width));
        if let Some(height) = height {
            el = el.h(px(height));
        }
        el.into_any_element()
    }

    fn pipe_none(self) -> AnyElement {
        self.into_any_element()
    }
}

/// What a button says: the text of its child, or whatever that child draws as.
fn button_label(ctx: &Ctx, child: Option<&str>, depth: u32, scope: Scope<'_>) -> SharedString {
    let Some(child) = child else {
        return SharedString::from("Button");
    };
    let _ = depth;
    match ctx.surface.get(child) {
        Some(Component {
            kind: Kind::Text { text, .. },
            ..
        }) => display(ctx, text.as_ref(), scope),
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
