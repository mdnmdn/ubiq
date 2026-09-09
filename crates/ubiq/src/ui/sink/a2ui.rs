//! The A2UI page: a surface an agent could send, beside the JSON it was drawn from.
//!
//! **The payload is the state.** The buffer on the right is the only place the preview comes from
//! — picking an example writes into it, and the parse happens on the way to drawing, every frame.
//! Nothing caches the result, so the two halves cannot disagree: whatever is on the left is what
//! the text on the right says, including when that text has just been broken by a keystroke.
//!
//! **The controls it draws are inert**, and that is the honest thing to draw. A2UI's inputs carry
//! their values in a data model addressed by JSON Pointer, and this page parses no data model — so
//! a field shows what the payload says it holds and refuses to pretend a keystroke went anywhere.
//! Tabs and modals do respond, because where the reader has navigated to is not a value the agent
//! is owed.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use gpui_component::input::{Editor, EditorState};
use std::rc::Rc;

use crate::app::AppState;
use crate::state::a2ui;
use crate::state::workbench::MenuId;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::menu::{Picker, PickerStyle};
use crate::ui::kit::{mono, slab};
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

/// The drawn surface, or the one line saying why there is not one.
fn preview(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let Some(state) = app.a2ui_buffer_state() else {
        // Reached only in the frame between the window opening and its buffers existing.
        return note("\u{2026}", theme::text_faint());
    };
    let source = state.read(cx).value().to_string();

    let surface = match a2ui::parse(&source) {
        Ok(surface) => surface,
        // A payload being typed over is broken most of the time, so this is the page's ordinary
        // state rather than its failure: one line saying what serde objected to, no empty pane.
        Err(reason) => return note(&reason, theme::danger()),
    };

    let view = cx.entity();
    let on_tab = {
        let view = view.clone();
        Rc::new(
            move |id: SharedString, index: usize, window: &mut Window, cx: &mut gpui::App| {
                view.update(cx, |this, cx| {
                    let _ = window;
                    this.select_a2ui_tab(id.to_string(), index, cx);
                });
            },
        )
    };
    let on_modal = {
        let view = view.clone();
        Rc::new(
            move |id: SharedString, window: &mut Window, cx: &mut gpui::App| {
                view.update(cx, |this, cx| {
                    let _ = window;
                    this.toggle_a2ui_modal(id.to_string(), cx);
                });
            },
        )
    };

    let ctx = crate::ui::a2ui::Ctx {
        tabs: &app.sink.a2ui.tabs,
        open_modal: app.sink.a2ui.modal.as_deref(),
        on_tab,
        on_modal,
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
        .child(crate::ui::a2ui::surface(&surface, &ctx))
        .into_any_element()
}

/// The picker over the payload it chooses, which is the half the reader edits.
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
        // Where the payload came from. Four of the five are upstream conformance fixtures and one
        // is written here, and whether the renderer handles real agent output turns on which.
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
        .into_any_element()
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
