//! Application settings, over the window: a nav, a column of rows, a fixed-size panel.
//!
//! **Not the kit's one-question modal.** A settings page is worked in rather than answered, so it
//! follows the project-settings overlay: scrim, coloured left edge, left nav, scrolling body.
//! The size is fixed — switching sections must not resize the panel.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, ElementId, Focusable, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, anchored, deferred,
    div, point, px,
};
use gpui_component::IconName;
use gpui_component::input::Input;
use ubiq_proto::ids::PaneId;

use crate::app::AppState;
use crate::state::settings::{LoginStep, MarkdownOpen, SettingsSection};
use crate::theme;
use crate::ui::kit::{
    check_box, choice_pill, column, field, ghost_button, heading, icon_button, label_block, modal,
    modal_note, nav_item, primary_button, setting_row,
};

pub fn overlay(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let viewport = window.viewport_size();
    let panel = dialog(app, window, cx).on_mouse_down_out(cx.listener(|this, _, _, cx| {
        this.close_settings(cx);
    }));

    deferred(
        anchored().position(point(px(0.), px(0.))).child(
            div()
                .id("app-settings")
                .w(viewport.width)
                .h(viewport.height)
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::scrim())
                .occlude()
                .child(panel),
        ),
    )
    .priority(2)
    .into_any_element()
}

fn dialog(
    app: &AppState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> gpui::Stateful<gpui::Div> {
    let viewport = window.viewport_size();

    div()
        .id("app-settings-dialog")
        .w(px(theme::SETTINGS_WIDTH))
        .h(px(theme::SETTINGS_HEIGHT))
        .max_w(viewport.width)
        .max_h(viewport.height)
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::accent())
        .shadow_lg()
        .child(header(cx))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .child(nav(app, cx))
                .child(body(app, cx)),
        )
}

fn header(cx: &mut Context<AppState>) -> AnyElement {
    div()
        .h(px(52.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child("Settings"),
        )
        .child(icon_button(
            "app-settings-close",
            IconName::Close,
            false,
            cx.listener(|this, _, _, cx| this.close_settings(cx)),
        ))
        .into_any_element()
}

fn nav(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let current = app.workbench.settings.nav;
    let items: Vec<AnyElement> = SettingsSection::all()
        .iter()
        .copied()
        .map(|item| {
            nav_item(
                ElementId::Name(format!("app-settings-nav-{}", item.label()).into()),
                nav_icon(item),
                item.label(),
                None,
                item == current,
                true,
                cx.listener(move |this, _, _, cx| this.set_settings_nav(item, cx)),
            )
        })
        .collect();

    div()
        .id("app-settings-nav")
        .w(px(220.))
        .flex()
        .flex_none()
        .flex_col()
        .gap_1()
        .px_2()
        .py_3()
        .bg(theme::pane_bg())
        .border_r_1()
        .border_color(theme::border())
        .children(items)
        .into_any_element()
}

fn nav_icon(item: SettingsSection) -> IconName {
    match item {
        SettingsSection::FileExplorer => IconName::Folder,
        SettingsSection::Editor => IconName::File,
        SettingsSection::Harnesses => IconName::Asterisk,
    }
}

fn body(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let content = match app.workbench.settings.nav {
        SettingsSection::FileExplorer => file_explorer(app, cx),
        SettingsSection::Editor => editor(app, cx),
        SettingsSection::Harnesses => harnesses(app, cx),
    };

    div()
        .id("app-settings-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scroll()
        .px_6()
        .py_5()
        .child(content)
        .into_any_element()
}

fn file_explorer(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let on = app.workbench.settings.ui.explorer_preview;
    column(vec![
        heading(
            "File explorer",
            "How a click on a file in the tree opens it.",
        ),
        setting_row(
            "Open files in previews",
            "A single click opens a temporary tab, replaced by the next preview. Double-click, \
             Shift-click and Shift-Enter open permanently either way.",
            check_box(
                "app-settings-explorer-preview",
                on,
                cx.listener(|this, _, _, cx| this.toggle_explorer_preview(cx)),
            )
            .into_any_element(),
        ),
    ])
}

fn editor(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let current = app.workbench.settings.ui.markdown_open;
    let pills: Vec<AnyElement> = MarkdownOpen::all()
        .iter()
        .copied()
        .map(|choice| {
            choice_pill(
                ElementId::Name(format!("app-settings-markdown-{}", choice.label()).into()),
                choice.label(),
                choice == current,
                cx.listener(move |this, _, _, cx| this.set_markdown_open(choice, cx)),
            )
            .into_any_element()
        })
        .collect();

    column(vec![
        heading(
            "Editor",
            "How a file opens. Already-open tabs keep the layout they were left in.",
        ),
        setting_row(
            "Markdown opens in",
            "New markdown files start in this layout. Source, preview and split remain available \
             on the tab.",
            div().flex().gap_1().children(pills).into_any_element(),
        ),
    ])
}

fn harnesses(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let isolated = app.workbench.settings.host.isolate_agents;
    column(vec![
        heading(
            "Harnesses",
            "Every agent runs on a harness. Register as many as you like — the same tool twice \
             with different credentials is normal, and each entry carries its own defaults.",
        ),
        setting_row(
            "Confine agents to their project",
            "An agent reads and writes only its project's folder, plus a throwaway configuration \
             of its own. Off, an agent can reach anywhere on the machine.",
            check_box(
                "app-settings-isolate-agents",
                isolated,
                cx.listener(|this, _, _, cx| this.toggle_isolate_agents(cx)),
            )
            .into_any_element(),
        ),
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(primary_button(
                "app-settings-add-harness",
                Some(IconName::Plus),
                "Add harness",
                cx.listener(|this, _, window, cx| this.open_harness_login(window, cx)),
            ))
            .into_any_element(),
        accounts(app),
    ])
}

/// The identities registered here, each with the harnesses it can actually start.
///
/// What a row shows is a *reference*: a name, and which harnesses have a captured login
/// under it. No credential and no path — neither ever crosses the bus, so neither is here to
/// draw.
fn accounts(app: &AppState) -> AnyElement {
    let accounts = &app.workbench.settings.accounts;
    if accounts.is_empty() {
        return div()
            .px_3()
            .py_8()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(SharedString::from("No harnesses registered.")),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::text_faint())
                    .child(SharedString::from(
                        "Add one to sign in — the harness runs its own login.",
                    )),
            )
            .into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .gap_px()
        .children(accounts.iter().map(|account| {
            // An account with no captured login is a reference to an environment variable
            // rather than a session, and saying so is the difference between "nothing here"
            // and "nothing to seed a run with".
            let signed_in = if account.logged_in.is_empty() {
                "not signed in".to_string()
            } else {
                account.logged_in.join(", ")
            };
            div()
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text())
                        .child(SharedString::from(account.id.clone())),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(if account.logged_in.is_empty() {
                            theme::text_faint()
                        } else {
                            theme::text_muted()
                        })
                        .child(SharedString::from(signed_in)),
                )
        }))
        .into_any_element()
}

/// The login modal: pick a harness, name the identity, watch the harness do its own login.
///
/// Three steps, one at a time, and the user can leave at any of them. Leaving a running
/// login abandons it, which is safe by construction — a flow that wrote no credential
/// captured nothing, and the host says so rather than recording a half-made account.
///
/// This is a modal rather than a tab on purpose: an OAuth flow wants the whole of the user's
/// attention for the half-minute it takes, and a login that scrolled away behind a pane is a
/// login nobody finishes.
pub fn login(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(login) = &app.workbench.settings.login else {
        return div().into_any_element();
    };
    let view = cx.entity();

    let (title, body, footer) = match &login.step {
        LoginStep::Choosing { agent_type } => (
            "Add harness",
            choosing(app, agent_type.as_deref(), window, cx),
            choosing_footer(agent_type.is_some(), app, cx),
        ),
        LoginStep::Running { pane } => (
            "Signing in",
            running(app, *pane, cx),
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(ghost_button(
                    "app-settings-login-abort",
                    None,
                    "Abort",
                    cx.listener(|this, _, _, cx| this.close_harness_login(cx)),
                ))
                .into_any_element(),
        ),
        LoginStep::Done { captured, message } => (
            if *captured {
                "Signed in"
            } else {
                "Not signed in"
            },
            div().pt_3().child(modal_note(message)).into_any_element(),
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(primary_button(
                    "app-settings-login-done",
                    None,
                    "Close",
                    cx.listener(|this, _, _, cx| this.close_harness_login(cx)),
                ))
                .into_any_element(),
        ),
    };

    modal(
        "app-settings-login",
        theme::accent(),
        title,
        body,
        footer,
        crate::ui::handler(&view, |this, _, cx| this.close_harness_login(cx)),
        window,
    )
}

/// Step one: which harness, and what to call the identity.
fn choosing(
    app: &AppState,
    chosen: Option<&str>,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let focused = app
        .login_account_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(
            "The harness runs its own sign-in. Ubiq opens it here and keeps the credential \
             under this name.",
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Harness",
                    "Which tool this identity signs in to.",
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        // Only harnesses whose binary is actually here: a sign-in for a tool
                        // that is not installed cannot start, and offering it would fail as a
                        // spawn the user has to interpret.
                        .children(
                            app.workbench
                                .agent_types
                                .iter()
                                .filter(|t| t.available)
                                .map(|agent_type| {
                                    let id = agent_type.id.clone();
                                    choice_pill(
                                        ElementId::Name(
                                            format!("app-settings-login-harness-{}", agent_type.id)
                                                .into(),
                                        ),
                                        &agent_type.label,
                                        chosen == Some(agent_type.id.as_str()),
                                        cx.listener(move |this, _, _, cx| {
                                            this.pick_login_harness(id.clone(), cx)
                                        }),
                                    )
                                }),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Name",
                    "What to call this identity. One name can sign in to several harnesses.",
                ))
                .child(
                    field(theme::border(), focused)
                        .h(px(30.))
                        .px_2()
                        .child(Input::new(&app.login_account_input).appearance(false)),
                ),
        )
        .into_any_element()
}

/// Step one's footer. Confirm is dead until a harness is picked, because the other half of
/// what it needs — the name — is in a field this function cannot read without a window.
fn choosing_footer(picked: bool, _app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "app-settings-login-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.close_harness_login(cx)),
        ))
        .child(
            primary_button(
                "app-settings-login-start",
                None,
                "Sign in",
                cx.listener(|this, _, _, cx| this.start_harness_login(cx)),
            )
            .when(!picked, |button| button.opacity(0.5)),
        )
        .into_any_element()
}

/// Step two: the harness's own login, in a real terminal.
///
/// The height is explicit because the modal body is a scrolling column, and the emulator
/// measures its own bounds to decide the geometry it reports to the harness — a box that
/// measured to nothing would tell the harness it had no screen.
fn running(app: &AppState, pane: PaneId, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(
            "Finish the sign-in below. It may open a browser; come back here when it is done.",
        ))
        .child(
            div()
                .h(px(260.))
                .border_1()
                .border_color(theme::border())
                .child(crate::ui::terminal::pane(app, pane, cx)),
        )
        .into_any_element()
}
