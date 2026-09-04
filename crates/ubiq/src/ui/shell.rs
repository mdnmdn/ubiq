//! The window's skeleton: titlebar, rail, the dock, and the status bar.
//!
//! Everything between the chrome is the **dock** — a tree of tabbed groups the user rearranges by
//! dragging. The window no longer fixes an arrangement: which panels exist is `AppState`'s answer,
//! where each sits is the user's, and what any of it looks like is `ui::dock::skin`'s.
//!
//! The chrome does not move. The titlebar, the rail and the status bar are the frame the dock is
//! drawn inside, and `D18`'s window edge is theirs rather than the dock's.

use gpui::{Context, InteractiveElement, IntoElement, ParentElement, Styled, Window, div, px};

use crate::app::{AppState, ZoomIn, ZoomOut};
use crate::theme;
use crate::ui::sink::project as project_settings;
use crate::ui::{rail, settings, status_bar, titlebar};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    div()
        .id("workbench-root")
        .flex()
        .flex_col()
        .size_full()
        .key_context("Workbench")
        .on_action(cx.listener(AppState::save_active_file))
        .on_action(cx.listener(AppState::close_active_editor))
        .on_action(cx.listener(AppState::open_search))
        .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.nudge_ui_font_size(1, cx)))
        .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.nudge_ui_font_size(-1, cx)))
        .bg(theme::app_bg())
        .text_color(theme::text())
        // The window wears its project's colour down its whole left edge.
        .border_l(px(theme::ACCENT_EDGE * 2.0))
        .border_color(app.project_tint(cx))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .child(rail::mark(app, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(titlebar::render(app, window, cx)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .child(rail::render(app, cx))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.))
                        .min_h(px(0.))
                        .child(app.dock().clone()),
                ),
        )
        .child(status_bar::render(app, cx))
        // Project settings is a form with a nav, not the kit's one-question modal, so it is
        // painted here — over the window — rather than from the picker that asked for it.
        .children(
            app.workbench
                .project_settings
                .as_ref()
                .map(|_| project_settings::overlay(app, window, cx)),
        )
        // Application settings is a page with a nav, not the kit's one-question modal, so it is
        // painted here — over the window — the same way project settings is.
        .children(
            app.workbench
                .settings
                .open
                .then(|| settings::overlay(app, window, cx)),
        )
        // The login modal, over the settings page that raised it — painted after it so it is on
        // top, and painted here rather than from that page because a login outlives it: closing
        // settings mid-flow must not take the harness's own sign-in with it.
        .children(
            app.workbench
                .settings
                .login
                .as_ref()
                .map(|_| settings::login(app, window, cx)),
        )
        // The file-tab context menu, named a file and a point by a right-click in the dock. It
        // lives at the window root rather than in a panel, so it stays on screen whether a file
        // closes or a panel moves.
        .children(
            (app.workbench.open_menu == Some(crate::state::MenuId::FileTab))
                .then(|| crate::ui::file_tab_menu::overlay(app, window, cx)),
        )
        // The new-pane control's chevron menu, named a point by a click on the bottom region's tab
        // bar. It is painted here for the same reason the file tab's menu is: the skin that drew
        // the chevron does not know what there is to offer.
        .children(
            (app.workbench.open_menu == Some(crate::state::MenuId::NewPane))
                .then(|| crate::ui::new_pane_menu::overlay(app, window, cx)),
        )
        // The new-agent menu, named a point by whichever surface asked for a conversation — the
        // agents screen's control or the IDE chat panel's. It is painted here rather than from
        // either, so both get it: the state it reads is the window's.
        .children(
            app.workbench
                .new_agent_menu
                .is_some()
                .then(|| crate::ui::agents::new_agent_menu(app, cx)),
        )
        // The new-agent naming prompt, raised once a harness and identity are picked and before
        // the conversation actually starts. Painted here for the same reason the menu above is:
        // more than one surface can open it.
        .children(
            app.workbench
                .naming_agent
                .is_some()
                .then(|| crate::ui::agents::new_agent_naming(app, window, cx)),
        )
}
