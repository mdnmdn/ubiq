//! The bottom dock.
//!
//! Almost every dock tab *is* a pane, and a pane is a terminal: the tab strip is the pane list,
//! and the body below it is the emulator, drawn by `gpui-terminal` from the bytes arriving on the
//! pane's stream. Nothing here names a path, a process or a descriptor — the pane is an ID, and
//! the emulator reads one end of the bus.
//!
//! The one tab that is not a pane is the log console, which sits last and takes the same space.
//! It is drawn by `ui::logs`, and it carries no pane ID because nothing about it is a terminal.

use gpui::{Context, Edges, IntoElement, ParentElement, Styled, div, px};
use gpui_component::IconName;
use gpui_terminal::{ColorPalette, TerminalConfig};

use crate::app::{AppState, DockTab};
use crate::theme;
use crate::ui::kit::{Tab, icon_button, mono, panel, tab_strip};
use crate::ui::{indexed, logs};

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let view = cx.entity();

    let showing_logs = app.dock_tab() == DockTab::Logs;

    let mut tabs: Vec<Tab> = app
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

    // The console is always the last tab, and it is never closed: it is the window's own output,
    // not a process someone started. Its dot reports the loudest thing the ring holds, so a
    // warning is visible while a pane is the tab on screen.
    let pane_count = tabs.len();
    let mut console = Tab::new("Logs");
    if let Some(level) = crate::log::logs().loudest()
        && level >= crate::log::LogLevel::Warn
    {
        console = console.dot(logs::level_colour(level));
    }
    tabs.push(console);

    // The strip carries the actions of whatever tab is active, and the dock's own hide either way.
    let tab_actions = if showing_logs {
        logs::actions(app, cx)
    } else {
        icon_button(
            "dock-new",
            IconName::Plus,
            false,
            cx.listener(|this, _, _, cx| {
                this.spawn_pane(None, Vec::new(), cx);
            }),
        )
        .into_any_element()
    };

    let trailing = div()
        .flex()
        .items_center()
        .gap_1()
        .child(tab_actions)
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

    // An index past the last tab marks nothing, which is what a dock with no pane and no console
    // selected needs.
    let active = match app.dock_tab() {
        DockTab::Logs => pane_count,
        DockTab::Pane if pane_count == 0 => tabs.len(),
        DockTab::Pane => app.focused_pane_index(),
    };

    let select = indexed(&view, |this, index, _, cx| this.select_dock_tab(index, cx));
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
            active,
            select,
            Some(close),
            Some(trailing),
        ))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .bg(theme::app_bg())
                .child(if showing_logs {
                    logs::body(app, cx)
                } else {
                    body(app).into_any_element()
                }),
        )
}

/// The focused pane's terminal, or the line the dock shows when there is no pane to draw.
fn body(app: &AppState) -> impl IntoElement {
    let pane = app.focused_pane();
    let terminal = pane.and_then(|pane| app.terminal(pane.id));
    let running = pane.map(|pane| pane.running).unwrap_or(false);

    let body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .bg(theme::pane_bg())
        // The focused pane wears its state on the edge that identifies every surface.
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(if running {
            theme::success()
        } else {
            theme::text_faint()
        });

    match terminal {
        Some(terminal) => body.child(div().flex().flex_1().min_h(px(0.)).child(terminal.clone())),
        None => body
            .p_3()
            .items_center()
            .justify_center()
            .child(mono("no pane", theme::text_faint())),
    }
}

/// How a pane's emulator is set up: the mono family and sizes the rest of the shell uses, and a
/// palette whose background, text and cursor are Ubiq's own tokens.
///
/// The sixteen ANSI colours are the emulator's defaults, because the terminal body is the
/// harness's output and Ubiq does not style it.
pub fn config(cols: u16, rows: u16) -> TerminalConfig {
    TerminalConfig {
        cols: cols as usize,
        rows: rows as usize,
        font_family: theme::MONO_FONT.to_string(),
        font_size: px(theme::TERMINAL_FONT_SIZE),
        scrollback: theme::TERMINAL_SCROLLBACK,
        line_height_multiplier: 1.0,
        padding: Edges::all(px(theme::TERMINAL_PADDING)),
        colors: palette(),
    }
}

fn palette() -> ColorPalette {
    let background = channels(theme::pane_bg());
    let foreground = channels(theme::text());
    let cursor = channels(theme::accent());
    ColorPalette::builder()
        .background(background.0, background.1, background.2)
        .foreground(foreground.0, foreground.1, foreground.2)
        .cursor(cursor.0, cursor.1, cursor.2)
        .build()
}

/// A token as the emulator's palette wants it: eight bits a channel.
fn channels(colour: gpui::Rgba) -> (u8, u8, u8) {
    let byte = |c: f32| (c.clamp(0., 1.) * 255.).round() as u8;
    (byte(colour.r), byte(colour.g), byte(colour.b))
}
