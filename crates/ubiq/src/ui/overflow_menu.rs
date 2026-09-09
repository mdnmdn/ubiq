//! The titlebar's overflow chevron menu: the rarely-used commands moved off the strip.
//!
//! Remote connect, web export, window capture and settings are all things the user reaches for
//! occasionally rather than every session, so they read as one family behind a chevron rather than
//! four permanent squares — painted over the window for the reason [`super::new_pane_menu`] is:
//! the titlebar does not name `AppState` beyond what it already reads, and this is a second menu
//! the window draws the same way.
//!
//! The rows are `WorkbenchState::overflow_rows`, read again here exactly as
//! `AppState::pick_overflow_menu` reads them — the rule every position-matched menu in this window
//! follows.

use gpui::{Context, IntoElement, Window, div, point, px};

use crate::app::AppState;
use crate::state::OverflowRow;
use crate::ui::{self, kit};

/// Draw the open overflow menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(at) = app.workbench.overflow_menu else {
        return div().into_any_element();
    };

    let has_project = app.project(cx).is_some();
    let capture_offered = app.capture_offered(cx);
    let items: Vec<_> = app
        .workbench
        .overflow_rows(has_project, capture_offered)
        .into_iter()
        .map(|row| kit::ContextItem::new(label(row)).icon(icon(row)))
        .collect();

    kit::context_menu(
        "overflow-menu",
        point(px(at.0), px(at.1)),
        items,
        ui::indexed(&cx.entity(), |this, index, window, cx| {
            this.pick_overflow_menu(index, window, cx);
        }),
        ui::handler(&cx.entity(), |this, _, cx| this.dismiss_overflow_menu(cx)),
    )
    .into_any_element()
}

/// What one row reads as. The words no longer sit on the strip — the icon does — but they are
/// still what the row's tooltip says, and what the titlebar's own controls said before these
/// moved in here.
fn label(row: OverflowRow) -> &'static str {
    match row {
        OverflowRow::RemoteConnect => "Connect to a remote host",
        OverflowRow::WebExport => "Explore the project in browser",
        OverflowRow::CaptureWindow => "Capture this window",
        OverflowRow::Settings => "Settings",
    }
}

/// The glyph one row draws before its label, from Ubiq's own set.
fn icon(row: OverflowRow) -> kit::UbiqIcon {
    match row {
        OverflowRow::RemoteConnect => kit::UbiqIcon::HostRemote,
        OverflowRow::WebExport => kit::UbiqIcon::TitlebarBrowser,
        OverflowRow::CaptureWindow => kit::UbiqIcon::CaptureWindow,
        OverflowRow::Settings => kit::UbiqIcon::TitlebarSettings,
    }
}
