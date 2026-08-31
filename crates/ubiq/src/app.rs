use gpui::{Context, IntoElement, ParentElement, Render, Window, div, prelude::*, px};
use std::collections::HashMap;
use uuid::Uuid;

use crate::theme;

/// Main application state for Ubiq multiplexer.
///
/// Manages:
/// - Pane lifecycle (spawn, resize, focus)
/// - State for all active agent harnesses
/// - UI layout and rendering
pub struct AppState {
    /// Active panes keyed by ID
    panes: HashMap<Uuid, PaneState>,
    /// Currently focused pane ID
    focused_pane: Option<Uuid>,
    /// Layout configuration (to be extended)
    layout_mode: LayoutMode,
}

/// Single agent harness pane state
#[derive(Clone)]
pub struct PaneState {
    pub id: Uuid,
    pub harness: String,
    pub args: Vec<String>,
    pub rows: u16,
    pub cols: u16,
    pub title: String,
}

/// Layout mode for pane arrangement
#[derive(Clone, Copy, Debug)]
pub enum LayoutMode {
    /// Single pane fills the window
    Single,
    /// Side-by-side vertical split
    Vsplit,
    /// Top-bottom horizontal split
    Hsplit,
    /// Grid layout (future)
    Grid,
}

impl AppState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            panes: HashMap::new(),
            focused_pane: None,
            layout_mode: LayoutMode::Single,
        }
    }

    pub fn spawn_pane(
        &mut self,
        harness: String,
        args: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Uuid {
        let pane_id = Uuid::new_v4();
        let pane = PaneState {
            id: pane_id,
            harness: harness.clone(),
            args,
            rows: 24,
            cols: 80,
            title: harness,
        };
        self.panes.insert(pane_id, pane);
        self.focused_pane = Some(pane_id);
        cx.notify();
        pane_id
    }

    pub fn close_pane(&mut self, pane_id: Uuid, cx: &mut Context<Self>) {
        self.panes.remove(&pane_id);
        if self.focused_pane == Some(pane_id) {
            self.focused_pane = self.panes.keys().next().copied();
        }
        cx.notify();
    }

    pub fn resize_pane(&mut self, pane_id: Uuid, cols: u16, rows: u16, cx: &mut Context<Self>) {
        if let Some(pane) = self.panes.get_mut(&pane_id) {
            pane.cols = cols;
            pane.rows = rows;
            cx.notify();
        }
    }

    pub fn focus_pane(&mut self, pane_id: Uuid, cx: &mut Context<Self>) {
        if self.panes.contains_key(&pane_id) {
            self.focused_pane = Some(pane_id);
            cx.notify();
        }
    }
}

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::app_bg())
            .text_color(theme::text())
            .child(render_titlebar(self, cx))
            .child(render_panes(self, cx))
    }
}

fn render_titlebar(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let pane_count = app.panes.len();
    let focused_title = app
        .focused_pane
        .and_then(|id| app.panes.get(&id))
        .map(|p| p.title.as_str())
        .unwrap_or("—");

    div()
        .h(px(40.))
        .px_4()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(theme::border())
        .bg(theme::pane_bg())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_color(theme::text())
                        .text_sm()
                        .child("Ubiq"),
                )
                .child(
                    div()
                        .text_color(theme::text_muted())
                        .text_xs()
                        .child(format!("{} pane{}", pane_count, if pane_count == 1 { "" } else { "s" })),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme::text_muted())
                .child(format!("Focused: {}", focused_title)),
        )
}

fn render_panes(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .gap_1()
        .p_1()
        .bg(theme::app_bg())
        .child(
            if app.panes.is_empty() {
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .size_full()
                    .text_color(theme::text_muted())
                    .child(
                        div()
                            .text_base()
                            .child("No panes active"),
                    )
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .bg(theme::pane_bg())
                    .border_1()
                    .border_color(theme::border())
                    .rounded_md()
                    .p_2()
                    .children(
                        app.panes
                            .values()
                            .map(|pane| {
                                let is_focused = app.focused_pane == Some(pane.id);
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .p_2()
                                    .border_1()
                                    .border_color(if is_focused {
                                        theme::border_focus()
                                    } else {
                                        theme::border()
                                    })
                                    .rounded_sm()
                                    .bg(theme::surface())
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::text_muted())
                                            .child(&pane.title),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .mt_2()
                                            .text_color(theme::text())
                                            .child(format!(
                                                "Pane: {} ({}×{})",
                                                pane.id, pane.cols, pane.rows
                                            )),
                                    )
                            })
                            .collect::<Vec<_>>(),
                    )
                    .into_any_element()
            },
        )
}
