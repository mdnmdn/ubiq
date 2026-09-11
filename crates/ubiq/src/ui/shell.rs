//! The window's skeleton: titlebar, rail, the dock, and the status bar.
//!
//! Everything between the chrome is the **dock** — a tree of tabbed groups the user rearranges by
//! dragging. The window no longer fixes an arrangement: which panels exist is `AppState`'s answer,
//! where each sits is the user's, and what any of it looks like is `ui::dock::skin`'s.
//!
//! The chrome does not move. The titlebar, the rail and the status bar are the frame the dock is
//! drawn inside, and `D18`'s window edge is theirs rather than the dock's.

use gpui::{Context, InteractiveElement, IntoElement, ParentElement, Styled, Window, div, px};

use crate::app::{AppState, FocusFileFilter, ImageRedo, ImageUndo, SubmitSearch, ZoomIn, ZoomOut};
use crate::theme;
use crate::ui::sink::project as project_settings;
use crate::ui::{
    new_agent, rail, remote_connect, remote_hosts, ribbon, settings, status_bar, titlebar,
};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // The content family's base is the *project's*, and the theme is one thread-local shared by
    // every window on the thread — so it is pushed in here, at the top of the window that is about
    // to draw, rather than written once when a zoom changes. Two windows on two projects then each
    // draw at their own size instead of at the last one set.
    theme::set_text_scale(theme::TextScale {
        content: app.content_font_size_or_default(cx),
        ..theme::text_scale()
    });

    div()
        .id("workbench-root")
        .flex()
        .flex_col()
        .size_full()
        .relative()
        .key_context("Workbench")
        // A fallback keyboard rest: see [`AppState::take_editor_focus`] for why a tab with
        // nothing focusable of its own hands the keyboard here rather than leaving it on
        // whatever the previous tab left behind.
        .track_focus(&app.workbench_focus)
        .on_action(cx.listener(AppState::save_active_file))
        .on_action(cx.listener(AppState::new_untitled_file))
        .on_action(cx.listener(AppState::capture_window))
        .on_action(cx.listener(AppState::paste_clipboard_image))
        .on_action(cx.listener(AppState::close_active_editor))
        .on_action(cx.listener(AppState::open_search))
        .on_action(cx.listener(AppState::open_outline))
        .on_action(cx.listener(AppState::back))
        .on_action(cx.listener(AppState::forward))
        .on_action(cx.listener(AppState::toggle_bookmark))
        .on_action(cx.listener(AppState::open_navigator))
        // ⌘⌥Y and ⌘⌥N answer the permission prompt the conversation being read is blocked on.
        // Both no-op when nothing is asking, so neither key is taken from anything else.
        .on_action(cx.listener(AppState::allow_permission))
        .on_action(cx.listener(AppState::reject_permission))
        // ⌘⏎ is the search itself, whether the navigator is up or not.
        .on_action(cx.listener(|this, _: &SubmitSearch, window, cx| {
            this.close_navigator(cx);
            this.submit_header_search(window, cx);
        }))
        .on_action(cx.listener(|this, _: &FocusFileFilter, window, cx| {
            this.reveal_explorer_filter(window, cx)
        }))
        // Undo and redo on the capture behind the active tab. Both no-op on anything else,
        // so neither key is taken from a buffer with its own stack.
        .on_action(cx.listener(|this, _: &ImageUndo, window, cx| {
            this.undo_image_action(&ImageUndo, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ImageRedo, window, cx| {
            this.redo_image_action(&ImageRedo, window, cx)
        }))
        // Enter and Escape answer the file question that is up. Both propagate when none is, so
        // the explorer's Escape and every field's Enter are untouched.
        .on_action(cx.listener(AppState::confirm_dialog))
        .on_action(cx.listener(AppState::cancel_dialog))
        .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.nudge_content_font_size(1, cx)))
        .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.nudge_content_font_size(-1, cx)))
        .bg(theme::app_bg())
        .text_color(theme::text())
        // The window wears its project's colour down its whole left edge.
        .border_l(px(theme::accent_edge() * 2.0))
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
                .child(rail::render(app, window, cx))
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
        // The New agent modal, over the settings page and the forms it raises: it is opened from
        // the workbench rather than from settings, and a start question left under an open page
        // would be a modal the user cannot see.
        .children(
            app.workbench
                .new_agent
                .as_ref()
                .map(|_| new_agent::render(app, window, cx)),
        )
        // The profile form, painted beside the login modal: both are raised from the harnesses
        // section, and only one is ever up.
        .children(
            app.workbench
                .settings
                .profile_form
                .as_ref()
                .map(|_| settings::profile_form(app, window, cx)),
        )
        // The accounts section's rename, delete or sign-out question — painted after the login
        // modal for the same reason that one is painted after the settings page: each can be up
        // over what raised it and has to be on top.
        .children(
            app.workbench
                .settings
                .dialog
                .as_ref()
                .map(|_| settings::account_dialog(app, window, cx)),
        )
        // The connect flow, painted here rather than from the connectors section for the reason
        // the login modal is: a flow outlives the page that started it, and closing settings
        // mid-flow must not abandon a browser sign-in already under way.
        .children(
            app.workbench
                .settings
                .connect
                .as_ref()
                .map(|_| settings::connect(app, window, cx)),
        )
        // The application-registration form, painted beside the connect modal rather than over it:
        // the two share their fields, so only one is ever up.
        .children(
            app.workbench
                .settings
                .app_form
                .as_ref()
                .map(|_| settings::app_form(app, window, cx)),
        )
        // The connectors section's rename, disconnect or forget question — over the connect
        // modal, since either can be up over what raised it.
        .children(
            app.workbench
                .settings
                .connector
                .as_ref()
                .map(|_| settings::connector_dialog(app, window, cx)),
        )
        // The certificate question, last of the three: it interrupts a running flow, so it has
        // to sit on top of the modal that flow is drawn in.
        .children(
            app.workbench
                .settings
                .cert
                .as_ref()
                .map(|_| settings::certificate(app, window, cx)),
        )
        // The API-provider form, raised from the assistance section — painted here for the reason
        // every settings modal above it is: where a dialog is asked for is not where it is drawn.
        .children(
            app.workbench
                .settings
                .ai_form
                .as_ref()
                .map(|_| settings::ai_form(app, window, cx)),
        )
        // The provider test, beside the form rather than over it: both are raised from a provider
        // row, and only one is ever up.
        .children(
            app.workbench
                .settings
                .ai_test
                .as_ref()
                .map(|_| settings::ai_test(app, window, cx)),
        )
        // The removal question, last of the three: it is raised from the same row and has to sit
        // on top of whatever raised it.
        .children(
            app.workbench
                .settings
                .ai_remove
                .as_ref()
                .map(|_| settings::ai_remove(app, window, cx)),
        )
        // The clone modal, over the picker that raised it and over the settings page, since the
        // omni search can raise it from anywhere.
        .children(
            app.workbench
                .clone_project
                .as_ref()
                .map(|_| crate::ui::clone::render(app, window, cx)),
        )
        // The "All projects" modal, raised from the picker's History group — painted here on the
        // same terms as the clone modal just above.
        .children(
            app.workbench
                .all_projects
                .as_ref()
                .map(|_| crate::ui::all_projects::render(app, window, cx)),
        )
        // The file question a gesture in the explorer or a save on an untitled buffer asked —
        // painted here rather than from either, because both raise the same one.
        .children(
            app.workbench
                .file_dialog
                .as_ref()
                .map(|_| crate::ui::file_dialog::render(app, window, cx)),
        )
        // The file picker, raised by a composer's `+`, by an explorer gesture or by a remote
        // project's Open — painted here for the reason every dialog above it is: one may be up at
        // a time, and where it is asked for is not where it is drawn. A picker raised from inside
        // a dock panel and painted from that panel would be a picker with no element in the tree,
        // because the panel that raised it is not what the window puts on top.
        .children(
            app.file_picker
                .as_ref()
                .map(|picker| crate::ui::file_picker::render(app, picker, window, cx)),
        )
        // The tab context menu, named a panel and a point by a right-click in the dock. It lives
        // at the window root rather than in a panel, so it stays on screen whether a tab closes or
        // a panel moves.
        .children(
            (app.workbench.open_menu == Some(crate::state::MenuId::Tab))
                .then(|| crate::ui::tab_menu::overlay(app, window, cx)),
        )
        // The new-pane control's chevron menu, named a point by a click on the bottom region's tab
        // bar. It is painted here for the same reason the file tab's menu is: the skin that drew
        // the chevron does not know what there is to offer.
        .children(
            (app.workbench.open_menu == Some(crate::state::MenuId::NewPane))
                .then(|| crate::ui::new_pane_menu::overlay(app, window, cx)),
        )
        // The titlebar's overflow chevron menu, named a point by a click on the chevron itself.
        // Painted here for the same reason the new-pane menu just above is.
        .children(
            (app.workbench.open_menu == Some(crate::state::MenuId::Overflow))
                .then(|| crate::ui::overflow_menu::overlay(app, window, cx)),
        )
        // The `+` menu, named a point by whichever surface asked — the agents screen's control,
        // the IDE chat strip's `+`, or the sink's bench. It is painted here rather than from any
        // of them, so all three get it: the state it reads is the window's.
        .children(
            app.workbench
                .new_agent_menu
                .is_some()
                .then(|| crate::ui::agents::new_agent_menu(app, window, cx)),
        )
        // The remote-hosts manager, raised from the titlebar — painted here on the same terms
        // as the clone modal just above. The connect modal paints after it, so "New
        // connection" from the panel lands on top, and Escape peels them the other way up.
        .children(
            app.workbench
                .remote_manager
                .open
                .then(|| remote_hosts::render(app, window, cx)),
        )
        // The remote-connect modal, raised from the titlebar rather than from settings — painted
        // here on the same terms as the clone modal just above.
        .children(
            app.workbench
                .remote_connect
                .as_ref()
                .map(|_| remote_connect::render(app, window, cx)),
        )
        // The bell's list, painted last of the overlays: it is reached from the titlebar, which
        // is above every screen, so it has to be above every dialog a screen raised.
        .child(crate::ui::notifications::render(app, window, cx))
        // The build-channel ribbon, over everything: the window always says which build it is.
        .child(ribbon::render())
        // Last child of the root, and it draws nothing: it prepaints after every panel and every
        // overlay above, which is the only moment at which the window knows which embedded
        // browsers were actually on screen this frame. See `crate::ui::web_view`.
        .child(crate::ui::web_view::sweeper(overlaid(app)))
}

/// Whether anything is painted over the dock this frame.
///
/// A child webview is a native view the platform stacks over the whole window, so it cannot be
/// covered by a modal the way an element can — the window has to take it off screen instead. The
/// list is the overlays above, and a new one belongs here as well as there.
fn overlaid(app: &AppState) -> bool {
    let workbench = &app.workbench;
    workbench.project_settings.is_some()
        || workbench.settings.open
        || workbench.new_agent.is_some()
        || workbench.clone_project.is_some()
        || workbench.all_projects.is_some()
        || workbench.file_dialog.is_some()
        || workbench.remote_manager.open
        || workbench.remote_connect.is_some()
        || workbench.new_agent_menu.is_some()
        || workbench.open_menu.is_some()
        || app.file_picker.is_some()
}
