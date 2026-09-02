//! One pane's terminal.
//!
//! A pane is a panel in the window's dock, and this is its body: the emulator, drawn by
//! `gpui-terminal` from the bytes arriving on the pane's stream. Nothing here names a path, a
//! process or a descriptor — the pane is an ID, and the emulator reads one end of the bus.
//!
//! **Every pane draws, not only the focused one.** The dock lays out whichever panels are the
//! displayed tabs of their groups, and each of those measures its own bounds; a pane that is a
//! background tab is not laid out and keeps the geometry its harness was last told, which is
//! correct, while its output goes on arriving and its emulator goes on consuming it.
//!
//! The tab, its dot and its close belong to the dock's skin. What is left here is the surface the
//! emulator sits on, and the configuration every emulator is built with.

use gpui::{AnyElement, App, Edges, IntoElement, ParentElement, Styled, div, px};
use gpui_terminal::{ColorPalette, TerminalConfig};

use crate::app::AppState;
use crate::theme;
use crate::ui::kit::mono;
use ubiq_proto::ids::PaneId;

/// One pane's emulator, or the line its panel shows while there is nothing to draw.
///
/// The pane is named rather than looked up from focus: under a dock every pane has a panel of its
/// own, and which of them the user is typing into is the dock's answer, not this module's.
pub fn pane(app: &AppState, pane_id: PaneId, _cx: &App) -> AnyElement {
    let pane = app.pane(pane_id);
    let terminal = app.terminal(pane_id);
    let running = pane.map(|pane| pane.running).unwrap_or(false);

    let body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .bg(theme::pane_bg())
        // A pane wears its harness's state on the edge that identifies every surface.
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(if running {
            theme::success()
        } else {
            theme::text_faint()
        });

    match terminal {
        Some(terminal) => body
            .child(div().flex().flex_1().min_h(px(0.)).child(terminal.clone()))
            .into_any_element(),
        // The emulator is dropped as the panel leaves the dock; this line is the frame in between.
        None => body
            .p_3()
            .items_center()
            .justify_center()
            .child(mono("no pane", theme::text_faint()))
            .into_any_element(),
    }
}

/// How a pane's emulator is set up: the mono family and sizes the rest of the shell uses, and a
/// palette whose background, text and cursor are Ubiq's own tokens.
///
/// The sixteen ANSI colours are the emulator's defaults, because the terminal body is the
/// harness's output and Ubiq does not style it.
pub fn config(cols: u16, rows: u16, font_size: f32) -> TerminalConfig {
    TerminalConfig {
        cols: cols as usize,
        rows: rows as usize,
        font_family: theme::MONO_FONT.to_string(),
        font_size: px(font_size),
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
    let selection = channels(theme::selection_background());
    let link = channels(theme::link_underline());
    let link_hover = channels(theme::link_underline_hover());
    ColorPalette::builder()
        .background(background.0, background.1, background.2)
        .foreground(foreground.0, foreground.1, foreground.2)
        .cursor(cursor.0, cursor.1, cursor.2)
        .selection_background(selection.0, selection.1, selection.2)
        .link_underline(link.0, link.1, link.2)
        .link_underline_hover(link_hover.0, link_hover.1, link_hover.2)
        .build()
}

/// A token as the emulator's palette wants it: eight bits a channel.
fn channels(colour: gpui::Rgba) -> (u8, u8, u8) {
    let byte = |c: f32| (c.clamp(0., 1.) * 255.).round() as u8;
    (byte(colour.r), byte(colour.g), byte(colour.b))
}
