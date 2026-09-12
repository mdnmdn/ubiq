//! The A2UI page: a surface an agent could send, beside the JSON it was drawn from and the two
//! things using it produces.
//!
//! **The payload is still the only source of the surface**, but it is no longer re-read every
//! frame. A surface now holds a data model its own inputs write into, and re-parsing on each frame
//! would throw those values away; so the payload is re-read when it *changes*, and what is drawn
//! is the surface that re-read built. A payload broken mid-keystroke puts a line above the last
//! one that parsed rather than replacing it — the reader is editing a program, and losing the
//! running one on every intermediate keystroke is not a reasonable price.
//!
//! **The lower pane is the demonstration.** The data model is where a keystroke in the preview
//! lands; the action log is the message an agent would have received. Between them they are the
//! whole of what A2UI's two directions are, on one page, with nothing behind them.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use gpui_component::input::{Editor, EditorState};
use std::rc::Rc;

use crate::app::AppState;
use crate::state::a2ui;
use crate::state::sink::A2uiPane;
use crate::state::workbench::MenuId;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::menu::{Picker, PickerStyle};
use crate::ui::kit::{ghost_button, mono, slab};
use crate::ui::{handler, indexed};

pub fn render(app: &AppState, _window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(
            half(preview(app, cx))
                .border_r_1()
                .border_color(theme::border()),
        )
        .child(half(source(app, cx)))
        .into_any_element()
}

/// The drawn surface, under whatever the current payload has to say about itself.
fn preview(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let live = &app.sink.a2ui.live;
    let Some(surface) = live.surface.as_ref() else {
        return note(
            live.error.as_deref().unwrap_or("\u{2026}"),
            match live.error.is_some() {
                true => theme::danger(),
                false => theme::text_faint(),
            },
        );
    };

    let view = cx.entity();
    let on_tab = {
        let view = view.clone();
        Rc::new(
            move |id: SharedString, index: usize, _window: &mut Window, cx: &mut gpui::App| {
                view.update(cx, |this, cx| {
                    this.select_a2ui_tab(id.to_string(), index, cx)
                });
            },
        )
    };
    let on_modal = {
        let view = view.clone();
        Rc::new(
            move |id: SharedString, _window: &mut Window, cx: &mut gpui::App| {
                view.update(cx, |this, cx| this.toggle_a2ui_modal(id.to_string(), cx));
            },
        )
    };
    let on_action = {
        let view = view.clone();
        Rc::new(
            move |key: SharedString, _window: &mut Window, cx: &mut gpui::App| {
                view.update(cx, |this, cx| this.fire_a2ui_action(key.to_string(), cx));
            },
        )
    };
    let on_value = {
        let view = view.clone();
        Rc::new(
            move |path: SharedString,
                  value: serde_json::Value,
                  _window: &mut Window,
                  cx: &mut gpui::App| {
                view.update(cx, |this, cx| {
                    this.set_a2ui_value(path.to_string(), value, cx)
                });
            },
        )
    };

    // Which checks fail is computed every frame rather than stored, so a button that cannot fire
    // says so before it is pressed. What a failing check *says* is stored instead, and only after
    // a submit — a form does not accuse a reader of a mistake they have not had a chance to make.
    let blocked = live
        .parsed()
        .map(|parsed| !a2ui::action::blocking_checks(&parsed).is_empty())
        .unwrap_or(false);

    let ctx = crate::ui::a2ui::Ctx {
        surface,
        model: &live.model,
        fields: &live.fields,
        invalid: &live.invalid,
        blocked,
        tabs: &live.tabs,
        open_modal: live.modal.as_deref(),
        on_tab,
        on_modal,
        on_action,
        on_value,
    };

    div()
        .id("a2ui-preview")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .p_4()
        .gap_2()
        .overflow_y_scroll()
        // The payload in the editor does not parse, and what is below is the last one that did.
        .children(live.error.as_ref().map(|reason| {
            slab(theme::danger())
                .p_2()
                .mb_2()
                .child(
                    mono(
                        format!("{reason} — showing the last payload that parsed"),
                        theme::danger(),
                    )
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
                )
                .into_any_element()
        }))
        .child(crate::ui::a2ui::render(&ctx))
        .into_any_element()
}

/// The payload the reader edits, over the two things the surface drawn from it produces.
fn source(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let picked = app.sink.a2ui.example;
    let names: Vec<&'static str> = a2ui::EXAMPLES.iter().map(|example| example.name).collect();
    let current = a2ui::EXAMPLES.get(picked);

    let chrome = div()
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .border_b_1()
        .border_color(theme::border())
        .child(
            Picker::new(
                "a2ui-example",
                current.map(|example| example.name).unwrap_or_default(),
            )
            .items(names)
            .selected(picked)
            .style(PickerStyle::Field)
            .open(app.workbench.open_menu == Some(MenuId::SinkA2ui))
            .on_toggle(handler(&cx.entity(), |this, _, cx| {
                this.open_menu(MenuId::SinkA2ui, cx)
            }))
            .on_pick(indexed(&cx.entity(), |this, index, window, cx| {
                this.pick_a2ui_example(index, window, cx)
            }))
            .on_dismiss(handler(&cx.entity(), |this, _, cx| this.close_menu(cx))),
        )
        // Where the payload came from. Some are upstream conformance fixtures and some are written
        // here, and whether the renderer handles real agent output turns on which.
        .children(current.map(|example| {
            mono(example.origin, theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Micro))
                .into_any_element()
        }));

    let body = match app.a2ui_buffer_state() {
        Some(state) => buffer(state),
        None => note("\u{2026}", theme::text_faint()),
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(chrome)
        .child(div().flex_1().min_h(px(0.)).child(body))
        .child(lower(app, cx))
        .into_any_element()
}

/// The data model and the action log, one of them forward.
fn lower(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let live = &app.sink.a2ui.live;
    let pane = app.sink.a2ui.pane;

    let tab = |label: &'static str, which: A2uiPane, count: Option<usize>| {
        let active = pane == which;
        div()
            .id(label)
            .h(px(28.))
            .px_3()
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .border_b_2()
            .border_color(match active {
                true => theme::accent(),
                false => theme::border(),
            })
            .text_size(theme::font(Family::Chrome, Role::Meta))
            .text_color(match active {
                true => theme::text(),
                false => theme::text_muted(),
            })
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .child(SharedString::from(label))
            .children(count.filter(|count| *count > 0).map(|count| {
                mono(format!("{count}"), theme::text_faint())
                    .text_size(theme::font(Family::Chrome, Role::Micro))
            }))
            .on_click(handler_click(cx, move |this, cx| {
                this.show_a2ui_pane(which, cx)
            }))
            .into_any_element()
    };

    let body = match pane {
        // The model as it stands, which is what a keystroke in the preview just changed.
        A2uiPane::Model => mono(
            serde_json::to_string_pretty(&live.model).unwrap_or_default(),
            theme::text_muted(),
        )
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .into_any_element(),
        A2uiPane::Actions => match live.log.is_empty() {
            true => mono(
                "nothing sent yet — press a button on the surface",
                theme::text_faint(),
            )
            .text_size(theme::font(Family::Chrome, Role::Meta))
            .into_any_element(),
            false => div()
                .flex()
                .flex_col()
                .gap_2()
                .children(live.log.iter().enumerate().map(|(index, entry)| {
                    slab(theme::accent())
                        .p_2()
                        .child(
                            mono(entry.clone(), theme::text_muted())
                                .text_size(theme::font(Family::Chrome, Role::Meta)),
                        )
                        .id(("a2ui-log", index))
                        .into_any_element()
                }))
                .into_any_element(),
        },
    };

    div()
        .flex()
        .flex_col()
        .flex_none()
        .h(px(200.))
        .border_t_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(theme::border())
                .child(tab("Data model", A2uiPane::Model, None))
                .child(tab("Actions", A2uiPane::Actions, Some(live.log.len())))
                .child(div().flex_1())
                .children(
                    (pane == A2uiPane::Actions && !live.log.is_empty()).then(|| {
                        ghost_button(
                            "a2ui-clear-log",
                            None,
                            "Clear",
                            handler_click(cx, |this, cx| this.clear_a2ui_log(cx)),
                        )
                        .into_any_element()
                    }),
                ),
        )
        .child(
            div()
                .id("a2ui-lower")
                .flex_1()
                .min_h(px(0.))
                .p_2()
                .overflow_y_scroll()
                .child(body),
        )
        .into_any_element()
}

/// A click handler over the window, in the shape `kit`'s buttons take.
fn handler_click(
    cx: &Context<AppState>,
    f: impl Fn(&mut AppState, &mut Context<AppState>) + 'static,
) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static {
    let view = cx.entity();
    move |_, _, cx| {
        view.update(cx, |this, cx| f(this, cx));
    }
}

/// The payload's buffer, at the inset the sink's other source halves draw one at.
fn buffer(state: &gpui::Entity<EditorState>) -> AnyElement {
    Editor::new(state)
        .h(relative(1.))
        .p_0()
        .border_0()
        .into_any_element()
}

/// One line in the middle of a half: what is wrong, or that there is nothing yet.
fn note(text: &str, colour: gpui::Rgba) -> AnyElement {
    slab(colour)
        .p_3()
        .m_3()
        .child(mono(text, colour).text_size(theme::font(Family::Chrome, Role::Meta)))
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
