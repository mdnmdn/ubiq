//! The window's skeleton: titlebar, rail, the dock, and the status bar.
//!
//! Everything between the chrome is the **dock** — a tree of tabbed groups the user rearranges by
//! dragging. The window no longer fixes an arrangement: which panels exist is `AppState`'s answer,
//! where each sits is the user's, and what any of it looks like is `ui::dock::skin`'s.
//!
//! The chrome does not move. The titlebar, the rail and the status bar are the frame the dock is
//! drawn inside, and `D18`'s window edge is theirs rather than the dock's.

use gpui::{Context, InteractiveElement, IntoElement, ParentElement, Styled, Window, div, px};

use crate::app::{
    AppState, FocusFileFilter, ImageRedo, ImageUndo, OpenHelp, PointAtSomething, ProjectSlot1,
    ProjectSlot2, ProjectSlot3, ProjectSlot4, ProjectSlot5, ProjectSlot6, ProjectSlot7,
    ProjectSlot8, ProjectSlot9, RailSlot1, RailSlot2, RailSlot3, RailSlot4, RailSlot5, RailSlot6,
    RailSlot7, RailSlot8, RailSlot9, SubmitSearch, ZoomIn, ZoomOut,
};
use crate::state::RailMode;
use crate::theme;
use crate::ui::sink::project as project_settings;
use crate::ui::{
    git, handler, kit, new_agent, new_mission, rail, remote_connect, remote_hosts, ribbon,
    settings, status_bar, titlebar,
};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // Nothing about size is pushed in here any more. Appearance is one setting for all of Ubiq
    // (`D151`), so the thread-local the theme already is carries the whole of it, written when a
    // size changes rather than at the top of every paint.
    //
    // The frame's `.ui_id(...)` collection starts here, before any child is built: element
    // construction finishes before layout begins, so every rectangle this frame records arrives
    // after this line. See `ui::ident`.
    crate::ui::ident::begin_frame(window);
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
        .on_action(cx.listener(|this, _: &OpenHelp, window, cx| this.reveal_help(window, cx)))
        .on_action(cx.listener(|this, _: &PointAtSomething, _, cx| this.open_help_target(cx)))
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
        .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.nudge_content_trim(1, cx)))
        .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.nudge_content_trim(-1, cx)))
        // ⌘1..⌘9 jump to the Nth project the rail's badges show; ⌃1..⌃9 jump to the Nth rail
        // mode enabled for the current project. Both no-op past the last one.
        .on_action(cx.listener(|this, _: &ProjectSlot1, window, cx| {
            this.activate_project_slot(1, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ProjectSlot2, window, cx| {
            this.activate_project_slot(2, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ProjectSlot3, window, cx| {
            this.activate_project_slot(3, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ProjectSlot4, window, cx| {
            this.activate_project_slot(4, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ProjectSlot5, window, cx| {
            this.activate_project_slot(5, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ProjectSlot6, window, cx| {
            this.activate_project_slot(6, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ProjectSlot7, window, cx| {
            this.activate_project_slot(7, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ProjectSlot8, window, cx| {
            this.activate_project_slot(8, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ProjectSlot9, window, cx| {
            this.activate_project_slot(9, window, cx)
        }))
        .on_action(cx.listener(|this, _: &RailSlot1, _, cx| this.activate_rail_mode_slot(1, cx)))
        .on_action(cx.listener(|this, _: &RailSlot2, _, cx| this.activate_rail_mode_slot(2, cx)))
        .on_action(cx.listener(|this, _: &RailSlot3, _, cx| this.activate_rail_mode_slot(3, cx)))
        .on_action(cx.listener(|this, _: &RailSlot4, _, cx| this.activate_rail_mode_slot(4, cx)))
        .on_action(cx.listener(|this, _: &RailSlot5, _, cx| this.activate_rail_mode_slot(5, cx)))
        .on_action(cx.listener(|this, _: &RailSlot6, _, cx| this.activate_rail_mode_slot(6, cx)))
        .on_action(cx.listener(|this, _: &RailSlot7, _, cx| this.activate_rail_mode_slot(7, cx)))
        .on_action(cx.listener(|this, _: &RailSlot8, _, cx| this.activate_rail_mode_slot(8, cx)))
        .on_action(cx.listener(|this, _: &RailSlot9, _, cx| this.activate_rail_mode_slot(9, cx)))
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
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .min_h(px(0.))
                        .children(
                            (app.workbench.rail_mode == RailMode::Git && app.project(cx).is_some())
                                .then(|| git::toolbar(app, window, cx)),
                        )
                        .child(app.dock().clone())
                        .children(
                            (app.workbench.rail_mode == RailMode::Git).then(ribbon::experimental),
                        ),
                ),
        )
        .child(status_bar::render(app, window, cx))
        // Project settings is a form with a nav, not the kit's one-question modal, so it is
        // painted here — over the window — rather than from the picker that asked for it.
        .children(
            app.workbench
                .project_settings
                .as_ref()
                .map(|_| project_settings::overlay(app, window, cx)),
        )
        // The knowledge base's "Add source" question, over the project settings page that raised
        // it — painted here rather than from that page for the reason the login modal is painted
        // over settings: a modal drawn inside the page it overlays is a modal the page can clip.
        .children(
            app.workbench
                .kb_source
                .as_ref()
                .map(|_| crate::ui::kb::source_form::render(app, window, cx)),
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
        // The new-mission dialog, raised from the tasks board's toolbar rather than from
        // anything drawn here — painted at the window root the same way the New agent modal is.
        .children(
            app.workbench
                .new_mission
                .as_ref()
                .map(|_| new_mission::render(app, window, cx)),
        )
        // The definition form, painted beside the login modal: both are raised from the harnesses
        // section, and only one is ever up.
        .children(
            app.workbench
                .settings
                .definition_form
                .as_ref()
                .map(|_| settings::definition_form(app, window, cx)),
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
        // The SSH-profile form, raised from the SSH profiles section — painted here on the same
        // terms as the provider form above it.
        .children(
            app.workbench
                .settings
                .ssh_form
                .as_ref()
                .map(|_| settings::ssh_form(app, window, cx)),
        )
        // Its removal question, over whatever raised it.
        .children(
            app.workbench
                .settings
                .ssh_remove
                .as_ref()
                .map(|_| settings::ssh_remove(app, window, cx)),
        )
        // The Stop-drone question, raised from the Drones section — over whatever raised it, on
        // the same terms as the SSH-profile removal above it.
        .children(
            app.workbench
                .settings
                .drone_stop
                .as_ref()
                .map(|_| settings::drone_stop(app, window, cx)),
        )
        // The clone modal, over the picker that raised it and over the settings page, since the
        // omni search can raise it from anywhere.
        .children(
            app.workbench
                .clone_project
                .as_ref()
                .map(|_| crate::ui::clone::render(app, window, cx)),
        )
        // The feedback modal, over the clone modal on the same terms: raised from the titlebar,
        // from anywhere, and over whatever is already on screen — which is the window it just
        // photographed.
        .children(
            app.workbench
                .feedback
                .as_ref()
                .map(|_| crate::ui::feedback::render(app, window, cx)),
        )
        // An agent's question, over the feedback modal on the same terms — except that it is
        // never actually raised over anything: an ask arriving while any layer is up leaves a
        // notification and a transcript entry instead, and the entry's button is what raises it
        // once the screen is clear. See `app::ask`.
        .children(
            app.workbench
                .ask
                .as_ref()
                .map(|_| crate::ui::ask::render(app, window, cx)),
        )
        // The "All projects" modal, raised from the picker's History group — painted here on the
        // same terms as the clone modal just above.
        .children(
            app.workbench
                .all_projects
                .as_ref()
                .map(|_| crate::ui::all_projects::render(app, window, cx)),
        )
        // One mission's full view, framed as a modal — raised from the side panel's `⤢` outside
        // IDE mode. Painted under the plan, which its *Plan & docs* tab raises over it.
        .children(
            app.workbench
                .mission
                .map(|task_id| crate::ui::mission::full::modal(app, task_id, window, cx)),
        )
        // A task's plan, read as rendered markdown — raised from the task panel, over whatever
        // else is up, on the same terms the modals above it are.
        .children(
            app.workbench
                .plan
                .as_ref()
                // A document open in a markdown tab's annotation layout draws in the panel, not
                // here: same surface, no modal around it.
                .filter(|doc| doc.is_modal())
                .map(|_| crate::ui::plan::render(app, window, cx)),
        )
        // The image/diagram zoom modal (T-185), over the plan on the same terms it can be raised
        // from either the standard viewer's tab or the plan surface's own rendered markdown.
        .children(
            app.workbench
                .image_zoom
                .as_ref()
                .map(|_| crate::ui::viewer::zoom_modal::render(app, window, cx)),
        )
        // The file question a gesture in the explorer or a save on an untitled buffer asked —
        // painted here rather than from either, because both raise the same one.
        .children(
            app.workbench
                .file_dialog
                .as_ref()
                .map(|_| crate::ui::file_dialog::render(app, window, cx)),
        )
        // The size preset's name prompt. At the window root because two places raise the same
        // one — the status bar's popover and the Size settings section — and neither is where it
        // is drawn.
        .children(
            app.workbench
                .size_prompt
                .as_ref()
                .map(|_| crate::ui::size::name_prompt(app, window, cx)),
        )
        // The theme editor, then the name prompt over it. At the window root for the reason the
        // size prompt is: the Appearance section raises the editor, the editor and the Themes row
        // both raise the prompt, and neither is where either is drawn.
        .children(
            app.workbench
                .theme_editor
                .as_ref()
                .map(|_| crate::ui::themes::editor(app, window, cx)),
        )
        .children(
            app.workbench
                .theme_prompt
                .as_ref()
                .map(|_| crate::ui::themes::name_prompt(app, window, cx)),
        )
        // The terminal tab's Close, asked before it is done. Painted at the window root rather
        // than from the pane it names, because the answer is what takes that pane off the screen
        // — a question drawn inside the thing it is about to destroy has nowhere to be.
        .children(
            app.workbench
                .confirm_close_pane
                .map(|_| close_pane_confirm(app, window, cx)),
        )
        // The chat tab's Close, asked before it is done — and painted here for exactly the reason
        // the pane's is. It used to hang off the conversation panel, which works for the
        // three-dots menu (the panel is still up) but not for the tab's own ×: that path takes
        // the panel down *first*, so the question was drawn by nothing and the conversation was
        // quietly not closed. The confirm needs only the agent's id, so the window root is where
        // it can always be answered.
        .children(
            app.workbench
                .confirm_end_conversation
                .map(|_| end_conversation_confirm(app, window, cx)),
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
        // The titlebar's new-project chevron menu, named a point by a click on the chevron beside
        // the `+`. Painted here for the same reason the overflow menu just above is.
        .children(
            (app.workbench.open_menu == Some(crate::state::MenuId::NewProject))
                .then(|| crate::ui::new_project_menu::overlay(app, window, cx)),
        )
        // The titlebar's run chevron menu, named a point by a click on the chevron beside the
        // play triangle. Painted here for the same reason the new-project menu just above is.
        .children(
            (app.workbench.open_menu == Some(crate::state::MenuId::RunTool))
                .then(|| crate::ui::run_tool_menu::overlay(app, window, cx)),
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
        // In-place help's targeting mode, painted after every other overlay — including the
        // bell's list, which is otherwise the last of them. It is the one layer that is not a
        // dialog: the reader is pointing at the window rather than answering it, and a dialog's
        // own controls are exactly the things they cannot otherwise ask about. `Layer::HelpTarget`
        // is the top rung for the same reason, so Escape peels this before anything under it.
        .child(crate::ui::help_target::overlay(app, window, cx))
        // The build-channel ribbon, over everything: the window always says which build it is.
        .child(ribbon::render())
        // Last child of the root, and it draws nothing: it prepaints after every panel and every
        // overlay above, which is the only moment at which the window knows which embedded
        // browsers were actually on screen this frame. See `crate::ui::web_view`.
        .child(crate::ui::web_view::sweeper(overlaid(app)))
}

/// The question a terminal tab's Close asks before it ends the harness.
///
/// Named after what is lost rather than after the gesture: killing the harness is the part that
/// cannot be undone, and the screen it has been writing to goes with it. The message also says
/// what the user probably wanted instead — Hide, one row up in the same menu — because the two
/// rows read alike and only one of them is irreversible.
fn close_pane_confirm(
    app: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    let entity = cx.entity();
    kit::confirm_modal(
        "close-pane-confirm",
        "Close terminal",
        &format!(
            "Close {}? Its harness is killed and the screen it has been writing to goes with it. \
             This cannot be undone \u{2014} use Hide to put the tab away and leave the harness \
             running.",
            app.workbench
                .confirm_close_pane
                .and_then(|pane_id| app.pane(pane_id))
                .map(|pane| pane.title.clone())
                .unwrap_or_else(|| "this terminal".to_string()),
        ),
        "Close",
        true,
        handler(&entity, |this, _, cx| this.confirm_close_pane(cx)),
        handler(&entity, |this, _, cx| this.dismiss_close_pane_confirm(cx)),
        window,
    )
}

/// The conversation's Close, asked before it is done.
///
/// Destructive and irreversible — the transcript and the run directory, seeded credentials
/// included, go with it — so it is confirmed rather than fired on the click. The wording names
/// Hide, the row above it in both menus that offer this, because the two read alike and only one
/// of them cannot be taken back.
fn end_conversation_confirm(
    app: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    let entity = cx.entity();
    kit::confirm_modal(
        "conversation-delete-confirm",
        "Close conversation",
        &format!(
            "Close {}? Its transcript and run directory \u{2014} seeded credentials included \
             \u{2014} go with it. This cannot be undone \u{2014} use Hide to put the view away and \
             leave the conversation running.",
            app.workbench
                .confirm_end_conversation
                .and_then(|agent_id| app.work(cx)?.agent(agent_id).map(|a| a.name.clone()))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "this conversation".to_string()),
        ),
        "Close",
        true,
        handler(&entity, |this, _, cx| this.confirm_end_conversation(cx)),
        handler(&entity, |this, _, cx| {
            this.dismiss_end_conversation_confirm(cx)
        }),
        window,
    )
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
        || workbench.size_prompt.is_some()
        || workbench.theme_editor.is_some()
        || workbench.theme_prompt.is_some()
        || workbench.confirm_close_pane.is_some()
        || workbench.confirm_end_conversation.is_some()
        || workbench.remote_manager.open
        || workbench.remote_connect.is_some()
        || workbench.new_agent_menu.is_some()
        || workbench.teams_create_menu.is_some()
        || workbench.open_menu.is_some()
        // A child webview is stacked over the window by the platform, so it would sit on top of
        // the targeting overlay and take the mouse moves it lives on.
        || workbench.help_target.is_some()
        || app.file_picker.is_some()
}
