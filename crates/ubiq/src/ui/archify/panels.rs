//! The diagnostics panel (P1.10): the top row's badge that says how the open document fares,
//! and the inline list under the picture (code, message, supported fixes). A click on a row selects
//! its subject: a node of the picture is focused (`focus.rs` lights it and its neighbourhood),
//! otherwise the tab turns to its source and the cursor goes to the subject's JSON path.
//!
//! Only draws and sends clicks to `Ui`; what a click means is `compiled::select`.

use std::sync::Arc;

use ubiq_archify::diag::{Diagnostic, Severity};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::input::{EditorState, Position};
use gpui_component::{Icon, IconName, Sizable as _, Size};
use crate::app::AppState;
use crate::state::editor::ViewLayout;
use crate::theme::{self, Family, Role};
use crate::ui::kit::{icon_button, mono, panel_header, pill};

use crate::state::archify::compiled::{self, Outcome};
use crate::state::archify::{picture_key, ui};

/// The panel is no taller than this; a longer list scrolls.
const MAX_HEIGHT_PX: f32 = 220.0;

/// How a document fares, as a chip that opens and closes the panel: the counts when something is
/// wrong, `ok · <profile>` when it is clean, and a quiet word until the first compile lands.
pub fn badge(key: &str, outcome: Option<&Outcome>) -> AnyElement {
    let (label, colour) = match outcome {
        None => ("checking\u{2026}".to_string(), theme::text_faint()),
        Some(o) => match (o.errors(), o.warnings()) {
            (0, 0) => (format!("ok \u{b7} {}", o.profile), theme::success()),
            (0, w) => (count(w, "warning"), theme::warning()),
            (e, 0) => (count(e, "error"), theme::danger()),
            (e, w) => (
                format!("{} \u{b7} {}", count(e, "error"), count(w, "warning")),
                theme::danger(),
            ),
        },
    };
    let key = key.to_string();
    div()
        .id("archify-diagnostics-toggle")
        .cursor_pointer()
        .h_full()
        .child(flat_chip(label, colour))
        .on_click(move |_, _window, cx| {
            ui(cx).toggle_panel(&key);
            cx.refresh_windows();
        })
        .into_any_element()
}

/// The kit's state chip, as tall as the bar it sits in: a dot and a word, no margin to the bar.
pub fn flat_chip(label: impl Into<SharedString>, colour: gpui::Rgba) -> impl IntoElement {
    pill(colour)
        .h_full()
        .px_2()
        .gap_1p5()
        .child(div().size(px(7.)).flex_none().rounded_full().bg(colour))
        .child(mono(label, theme::text()).text_size(theme::font(Family::Chrome, Role::Meta)))
}

fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// The list under the picture. `key` is the tab's key (a focus lands on its picture,
/// [`picture_key`]); `buffer` is the tab's own, where a path diagnostic puts the cursor.
pub fn panel(
    key: &str,
    buffer: &Entity<EditorState>,
    outcome: &Arc<Outcome>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let close_key = key.to_string();
    let close = icon_button(
        "archify-diagnostics-close",
        IconName::Close,
        false,
        move |_, _window, cx| {
            ui(cx).toggle_panel(&close_key);
            cx.refresh_windows();
        },
    );
    let body = if outcome.diagnostics.is_empty() {
        div()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Icon::new(IconName::CircleCheck)
                    .with_size(Size::XSmall)
                    .text_color(theme::success()),
            )
            .child(mono(
                format!("ok \u{b7} {}", outcome.profile),
                theme::success(),
            ))
            .into_any_element()
    } else {
        div()
            .id("archify-diagnostics-list")
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .children(
                outcome
                    .diagnostics
                    .iter()
                    .enumerate()
                    .map(|(index, d)| row(key, buffer, index, d, cx)),
            )
            .into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .flex_none()
        .max_h(px(MAX_HEIGHT_PX))
        .min_h(px(0.))
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .child(panel_header("Diagnostics", close))
        .child(body)
        .into_any_element()
}

/// One diagnostic: severity icon, code and where it is, the message, the fixes it supports.
fn row(
    key: &str,
    buffer: &Entity<EditorState>,
    index: usize,
    d: &Diagnostic,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let (icon, colour) = match d.severity {
        Severity::Warning => (IconName::TriangleAlert, theme::warning()),
        _ => (IconName::CircleX, theme::danger()),
    };
    let (key, buffer) = (key.to_string(), buffer.clone());
    let mut text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .gap_0p5()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(mono(d.code.clone(), theme::text_muted()))
                .children(where_of(d).map(|at| mono(at, theme::text_faint()))),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text())
                .child(SharedString::from(d.message.clone())),
        );
    for fix in &d.supported_fixes {
        text = text.child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_faint())
                .child(SharedString::from(format!("fix: {fix}"))),
        );
    }
    div()
        .id(("archify-diagnostic", index))
        .px_3()
        .py_1()
        .flex()
        .items_start()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            div()
                .pt_0p5()
                .child(Icon::new(icon).with_size(Size::XSmall).text_color(colour)),
        )
        .child(text)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            pick(this, &key, &buffer, index, window, cx);
        }))
        .into_any_element()
}

/// The diagnostic's subject for a human: the node or edge it names, else its JSON path.
pub(crate) fn where_of(d: &Diagnostic) -> Option<String> {
    let s = &d.subject;
    s.id.clone()
        .or_else(|| s.identity.clone())
        .or_else(|| s.path.clone().filter(|p| p != "/"))
}

/// A click on row `index`: light its node, or turn the tab to its source with the cursor at the
/// subject's path.
fn pick(
    this: &mut AppState,
    key: &str,
    buffer: &Entity<EditorState>,
    index: usize,
    window: &mut Window,
    cx: &mut Context<AppState>,
) {
    let (outcome, scene) = {
        let tab = ui(cx).tab(key);
        (tab.compiled.clone(), tab.scene.clone())
    };
    let Some(d) = outcome.as_ref().and_then(|o| o.diagnostics.get(index)) else {
        return;
    };
    let text = buffer.read(cx).value().to_string();
    let selection = compiled::select(d, scene.as_deref(), &text);
    if let Some(node) = selection.focus {
        // A node is lit in the picture, which is on screen: the source is left alone.
        let view = ui(cx).view(&picture_key(key));
        view.focus = Some(node);
        // The camera follows once the picture is on screen (`agent::drive_reveal`).
        view.reveal = selection.reveal;
    } else if let Some((line, column)) = selection.cursor {
        buffer.update(cx, |state, cx| {
            state.set_cursor_position(Position::new(line, column), window, cx);
        });
        // A path needs the source on screen.
        this.set_view_layout(key, ViewLayout::Source, cx);
    }
    cx.notify();
}
