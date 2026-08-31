//! The bottom dock.
//!
//! A dock tab *is* a pane, and a pane is a terminal. The body is a placeholder until the emulator
//! is wired in — it names the pane and its geometry and nothing else, because the UI knows a pane
//! only as an ID plus a byte stream.

use gpui::{Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::IconName;

use crate::app::AppState;
use crate::theme;
use crate::ui::indexed;
use crate::ui::kit::{Tab, icon_button, mono, panel, tab_strip};

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let view = cx.entity();

    let tabs: Vec<Tab> = app
        .panes()
        .iter()
        .map(|pane| {
            Tab::new(pane.title.clone())
                .dot(if pane.running {
                    theme::success()
                } else {
                    theme::text_faint()
                })
                .closable(true)
        })
        .collect();

    let trailing = div()
        .flex()
        .items_center()
        .gap_1()
        .child(icon_button(
            "dock-new",
            IconName::Plus,
            false,
            cx.listener(|this, _, _, cx| {
                this.spawn_pane("zsh".to_string(), Vec::new(), cx);
            }),
        ))
        .child(icon_button(
            "dock-hide",
            IconName::Minus,
            false,
            cx.listener(|this, _, _, cx| {
                this.workbench.show_bottom = false;
                cx.notify();
            }),
        ))
        .into_any_element();

    let select = indexed(&view, |this, index, _, cx| {
        if let Some(pane) = this.panes().get(index) {
            let id = pane.id;
            this.focus_pane(id, cx);
        }
    });
    let close = std::rc::Rc::new(indexed(&view, |this, index, _, cx| {
        if let Some(pane) = this.panes().get(index) {
            let id = pane.id;
            this.close_pane(id, cx);
        }
    }));

    panel()
        .border_t_1()
        .border_color(theme::border())
        .child(tab_strip(
            "dock-tab",
            tabs,
            app.focused_pane_index(),
            select,
            Some(close),
            Some(trailing),
        ))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .p_2()
                .bg(theme::app_bg())
                .child(placeholder(app)),
        )
}

/// The seam the terminal component drops into.
fn placeholder(app: &AppState) -> impl IntoElement {
    let (title, geometry, running) = match app.focused_pane() {
        Some(pane) => (
            pane.title.clone(),
            format!("{}\u{d7}{}", pane.cols, pane.rows),
            pane.running,
        ),
        None => ("no pane".to_string(), "\u{2014}".to_string(), false),
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap_2()
        .p_3()
        .bg(theme::pane_bg())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(if running {
            theme::success()
        } else {
            theme::text_faint()
        })
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(mono(title, theme::text()))
                .child(mono(geometry, theme::text_faint())),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .items_center()
                .justify_center()
                .child(mono(
                    "terminal component not wired in yet",
                    theme::text_faint(),
                )),
        )
}
