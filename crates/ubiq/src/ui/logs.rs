//! The log console: a tab in the terminal dock, beside the panes.
//!
//! Everything the application, the coordinator and the harness library say through `tracing`
//! lands in the process-wide sink in [`ubiq_proto::log`]; this draws a filtered view of it. It reads and
//! never writes — clearing the ring is the one thing it asks of the sink.
//!
//! The dock owns the chrome, so this module hands it two pieces: [`actions`] for its tab strip,
//! and [`body`] for the space a pane's emulator would fill. Rows are uniform and drawn lazily,
//! because a full ring is thousands of them and only the visible ones are worth laying out.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Rgba, ScrollStrategy,
    SharedString, Styled, div, px, uniform_list,
};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::{LogState, MenuId};
use crate::theme;
use crate::ui::kit::{Picker, ghost_button, icon_button, mono};
use crate::ui::{handler, indexed};
use ubiq_proto::log::{LogLevel, LogRecord};

/// The height of one row. Uniform is what makes the list lazy, so a message is one line and is cut
/// off rather than wrapped.
const ROW_HEIGHT: f32 = 22.0;

/// The colour a level is reported in. Status is never shown by wording alone, so the level word,
/// the message and the row's edge all take it.
pub fn level_colour(level: LogLevel) -> Rgba {
    match level {
        LogLevel::Error => theme::danger(),
        LogLevel::Warn => theme::warning(),
        LogLevel::Info => theme::info(),
        LogLevel::Debug => theme::text_muted(),
        LogLevel::Trace => theme::text_faint(),
    }
}

/// What the console puts in the dock's tab strip: what it is showing, and the two selectors that
/// decide.
pub fn actions(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let logs = &app.logs;
    let (kept, dropped) = ubiq_proto::log::logs().counts();

    let counted = if dropped == 0 {
        format!("{kept} records")
    } else {
        format!("{kept} records \u{b7} {dropped} dropped")
    };

    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            Picker::new("log-subsystem", logs.subsystem_label())
                .items(LogState::subsystem_items())
                .selected(logs.subsystem_index())
                .open(app.workbench.open_menu == Some(MenuId::LogSubsystem))
                .on_toggle(handler(&view, |this, _, cx| {
                    this.open_menu(MenuId::LogSubsystem, cx)
                }))
                .on_pick(indexed(&view, |this, index, _, cx| {
                    this.pick_log_subsystem(index, cx)
                }))
                .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx))),
        )
        .child(
            Picker::new("log-level", logs.min_level.label())
                .items(LogState::level_items())
                .selected(logs.level_index())
                .open(app.workbench.open_menu == Some(MenuId::LogLevel))
                .on_toggle(handler(&view, |this, _, cx| {
                    this.open_menu(MenuId::LogLevel, cx)
                }))
                .on_pick(indexed(&view, |this, index, _, cx| {
                    this.pick_log_level(index, cx)
                }))
                .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx))),
        )
        .child(mono(counted, theme::text_faint()).text_size(px(11.)))
        .child(icon_button(
            "log-follow",
            IconName::ArrowDown,
            logs.follow,
            cx.listener(|this, _, _, cx| this.toggle_log_follow(cx)),
        ))
        .child(ghost_button(
            "log-clear",
            Some(IconName::Delete),
            "Clear",
            cx.listener(|this, _, _, cx| this.clear_logs(cx)),
        ))
        .into_any_element()
}

/// The records themselves, in the space a pane's emulator would fill.
pub fn body(app: &AppState, _cx: &mut Context<AppState>) -> AnyElement {
    let records = ubiq_proto::log::logs().snapshot(app.logs.filter());

    // Following means the tail stays in view as records arrive. The list defers the request, so it
    // costs nothing when the console is not the tab on screen.
    if app.logs.follow && !records.is_empty() {
        app.log_scroll
            .scroll_to_item(records.len() - 1, ScrollStrategy::Top);
    }

    let surface = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .bg(theme::pane_bg())
        // The console is a surface like a pane, and its edge carries the loudest thing it holds.
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(
            ubiq_proto::log::logs()
                .loudest()
                .filter(|level| *level >= LogLevel::Warn)
                .map_or(theme::text_faint(), level_colour),
        )
        // The keyboard stops here while the console is shown, rather than reaching the pane behind.
        .track_focus(app.log_focus());

    if records.is_empty() {
        return surface.child(empty(app)).into_any_element();
    }

    surface
        .child(
            uniform_list(
                "log-rows",
                records.len(),
                move |range, _window, _cx| -> Vec<AnyElement> {
                    range
                        .filter_map(|index| records.get(index).map(row))
                        .collect()
                },
            )
            .track_scroll(&app.log_scroll)
            .flex_1()
            .min_h(px(0.)),
        )
        .into_any_element()
}

/// One record: when, how loud, from where, and what it said.
fn row(record: &Arc<LogRecord>) -> AnyElement {
    let colour = level_colour(record.level);
    let warns = record.level >= LogLevel::Warn;

    div()
        .h(px(ROW_HEIGHT))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .overflow_hidden()
        .child(
            mono(record.time(), theme::text_faint())
                .flex_none()
                .text_size(px(11.)),
        )
        .child(
            mono(record.level.label(), colour)
                .w(px(46.))
                .flex_none()
                .text_size(px(11.)),
        )
        .child(
            mono(record.subsystem.label(), theme::text_muted())
                .w(px(88.))
                .flex_none()
                .text_size(px(11.)),
        )
        .child(
            mono(
                SharedString::from(record.message.clone()),
                if warns { colour } else { theme::text() },
            )
            .flex_1()
            .min_w(px(0.))
            .overflow_hidden(),
        )
        .into_any_element()
}

/// What the console shows when the filter matches nothing. It names the filter, because an empty
/// console is usually a filter and not a quiet application.
fn empty(app: &AppState) -> impl IntoElement {
    let level = app.logs.min_level.label();
    let note = match app.logs.subsystem {
        Some(subsystem) => format!("Nothing from {} at {level} or above.", subsystem.label()),
        None => format!("Nothing at {level} or above."),
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .child(mono(note, theme::text_faint()))
}
