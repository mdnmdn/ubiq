//! Application settings, over the window: a nav, a column of rows, a fixed-size panel.
//!
//! **Not the kit's one-question modal.** A settings page is worked in rather than answered, so it
//! follows the project-settings overlay: scrim, coloured left edge, left nav, scrolling body.
//! The size is fixed — switching sections must not resize the panel.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, ClipboardItem, Context, ElementId, Focusable, FontWeight, InteractiveElement,
    IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, anchored,
    deferred, div, point, px,
};
use gpui_component::IconName;
use gpui_component::input::Input;
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::{AccountInfo, CliShortcutAction, LoginStatus};

use crate::app::AppState;
use crate::state::settings::{
    AccountDialog, CliShortcut, LoginStep, MarkdownOpen, SettingsSection, describe_status,
};
use crate::theme;
use crate::ui::kit::{
    check_box, choice_pill, column, confirm_modal, elided, field, ghost_button, heading,
    icon_button, label_block, modal, modal_note, modal_sized, nav_item, primary_button,
    prompt_modal, setting_row,
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
        SettingsSection::Search => IconName::Search,
        SettingsSection::Harnesses => IconName::Asterisk,
        SettingsSection::CommandLine => IconName::SquareTerminal,
    }
}

fn body(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let content = match app.workbench.settings.nav {
        SettingsSection::FileExplorer => file_explorer(app, cx),
        SettingsSection::Editor => editor(app, cx),
        SettingsSection::Search => search(app),
        SettingsSection::Harnesses => harnesses(app, cx),
        SettingsSection::CommandLine => command_line(app, cx),
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

/// The two host-owned lists. Each is a comma-separated line that commits on Enter and on blur —
/// see the subscriptions in `app.rs`, and `sync_search_settings_fields` for what fills them.
fn search(app: &AppState) -> AnyElement {
    let line = |input| {
        field(theme::border(), false)
            .h(px(30.))
            .w(px(300.))
            .px_2()
            .child(Input::new(input).appearance(false))
            .into_any_element()
    };

    column(vec![
        heading(
            "Search",
            "What every project search skips, and what it falls back to. Comma-separated, and \
             written down when the field is left or Enter is pressed.",
        ),
        setting_row(
            "Excluded paths",
            "Globs every project search skips, on top of the ignore rules the project already \
             carries. Emptying the field searches everything those rules allow.",
            line(&app.search_excludes_input),
        ),
        setting_row(
            "Fallback tools",
            "External tools tried in order, and only when the built-in matcher cannot answer a \
             query \u{2014} a pattern its stricter regex engine refuses. Empty means there is no \
             fallback. `find` and `fd` are refused whatever is typed here: they match file names \
             rather than contents, so they cannot answer a content search.",
            line(&app.search_fallbacks_input),
        ),
    ])
}

fn harnesses(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let isolated = app.workbench.settings.host.isolate_agents;
    let mut rows = vec![
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
    ];
    if let Some(error) = app.workbench.settings.error.clone() {
        rows.push(error_banner(&error, cx));
    }
    rows.push(
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
    );
    rows.push(accounts(app, cx));
    column(rows)
}

/// The `ubiq` command on the shell's `PATH`.
///
/// Everything drawn here is the host's answer — where the shortcut is, where one would go, which
/// directories were considered. The interface owns none of these paths and asks for one of three
/// actions; `app/settings.rs` sends them and the answer redraws this section.
fn command_line(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let Some(cli) = app.workbench.settings.cli.clone() else {
        return column(vec![
            heading(COMMAND_LINE_NOTE.0, COMMAND_LINE_NOTE.1),
            note("Looking\u{2026}", theme::text_faint()),
        ]);
    };

    let installed = cli.installed.is_some();
    let (verb, action) = match (installed, cli.stale) {
        (true, true) => ("Update", CliShortcutAction::Install),
        (true, false) => ("Remove", CliShortcutAction::Remove),
        (false, _) => ("Install", CliShortcutAction::Install),
    };
    let target = cli.target.clone().unwrap_or_default();
    let on_path = cli
        .candidates
        .iter()
        .find(|dir| dir.chosen)
        .is_some_and(|dir| dir.on_path);

    let status = match (&cli.installed, cli.stale, cli.target.is_some(), on_path) {
        (_, _, false, _) => (
            "No directory on this machine can hold the command.".to_string(),
            theme::danger(),
        ),
        (Some(path), true, ..) => (
            format!("{path} \u{2014} launches another build of Ubiq"),
            theme::danger(),
        ),
        (Some(path), false, _, true) => (format!("{path} \u{b7} on PATH"), theme::text_muted()),
        (Some(path), false, _, false) => (
            format!("{path} \u{b7} not on PATH \u{2014} add it to your shell profile"),
            theme::text_faint(),
        ),
        (None, .., true) => (format!("would be written to {target}"), theme::text_faint()),
        (None, ..) => (
            format!("would be written to {target} \u{b7} not on PATH"),
            theme::text_faint(),
        ),
    };

    let mut rows = vec![
        heading(COMMAND_LINE_NOTE.0, COMMAND_LINE_NOTE.1),
        setting_row(
            "The `ubiq` command",
            "`ubiq .` opens a folder as a project and `ubiq README.md` opens a file, in the \
             window that already holds it when there is one.",
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    primary_button(
                        "app-settings-cli-action",
                        Some(IconName::SquareTerminal),
                        verb,
                        cx.listener(move |this, _, _, cx| {
                            this.ask_cli_shortcut(action);
                            cx.notify();
                        }),
                    )
                    .when(cli.target.is_none(), |button| button.opacity(0.5)),
                )
                .child(note(&status.0, status.1))
                .into_any_element(),
        ),
    ];
    if let Some(error) = cli.error.clone() {
        rows.push(error_banner(&error, cx));
    }
    rows.push(candidate_list(&cli));
    column(rows)
}

/// The section's own heading, named once because the "looking" state draws it too.
const COMMAND_LINE_NOTE: (&str, &str) = (
    "Command line",
    "A small script Ubiq writes into a directory on your PATH. Removing it removes only that \
     script \u{2014} never a `ubiq` you put there yourself.",
);

/// Every directory considered, in the order the host considered them, so a machine that has none
/// of them says why rather than failing silently.
fn candidate_list(cli: &CliShortcut) -> AnyElement {
    let rows: Vec<AnyElement> = cli
        .candidates
        .iter()
        .map(|dir| {
            let chosen = dir.chosen;
            let state = match (dir.exists, dir.on_path) {
                (false, _) => "missing",
                (true, true) => "exists \u{b7} on PATH",
                (true, false) => "exists \u{b7} not on PATH",
            };
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(11.))
                .child(
                    div()
                        .w(px(220.))
                        .text_color(if chosen {
                            theme::text()
                        } else {
                            theme::text_muted()
                        })
                        .child(SharedString::from(dir.path.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .text_color(theme::text_faint())
                        .child(SharedString::from(state)),
                )
                .when(chosen, |row| {
                    row.child(
                        div()
                            .text_color(theme::accent())
                            .child(SharedString::from("\u{2713} chosen")),
                    )
                })
                .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(rows)
        .into_any_element()
}

/// One line of status beside a control, in the weight the settings rows use for it.
fn note(text: &str, colour: gpui::Rgba) -> AnyElement {
    div()
        .text_size(px(11.))
        .text_color(colour)
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

/// What the host last refused for a rename, delete or sign-out. The same warning-banner shape
/// `ui/project_menu.rs`'s row confirmations use, dismissible because it is history the moment
/// it is read.
fn error_banner(error: &str, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .px_3()
        .py_2()
        .flex()
        .items_center()
        .gap_2()
        .bg(theme::warning_soft())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::warning())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.))
                .text_color(theme::text())
                .child(SharedString::from(error.to_string())),
        )
        .child(ghost_button(
            "app-settings-account-error-dismiss",
            None,
            "Dismiss",
            cx.listener(|this, _, _, cx| this.dismiss_account_error(cx)),
        ))
        .into_any_element()
}

/// The identities registered here, each with the harnesses it can actually start.
///
/// What a block shows is a *reference*: a name, and which harnesses have a captured login
/// under it, each with the check status last asked for. No credential and no path — neither
/// ever crosses the bus, so neither is here to draw.
fn accounts(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    if app.workbench.settings.accounts.is_empty() {
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

    let now_ms = chrono::Utc::now().timestamp_millis();
    let accounts = app.workbench.settings.accounts.clone();

    div()
        .flex()
        .flex_col()
        .gap_4()
        .children(
            accounts
                .iter()
                .map(|account| account_block(app, account, now_ms, cx)),
        )
        .into_any_element()
}

/// The harness's display name, resolved through what the host offers — falling back to the
/// raw id when the host does not (or no longer) list that harness.
fn harness_label<'a>(app: &'a AppState, agent_type: &'a str) -> &'a str {
    app.workbench
        .agent_types
        .iter()
        .find(|info| info.id == agent_type)
        .map(|info| info.label.as_str())
        .unwrap_or(agent_type)
}

/// One account: its header, and one line per harness it has a captured login for.
fn account_block(
    app: &AppState,
    account: &AccountInfo,
    now_ms: i64,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = account.id.clone();

    let header = {
        let rename_id = id.clone();
        let delete_id = id.clone();
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .py_1()
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::text())
                    .child(SharedString::from(id.clone())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(icon_button(
                        ElementId::Name(format!("app-settings-account-{id}-rename").into()),
                        IconName::Replace,
                        false,
                        cx.listener(move |this, _, window, cx| {
                            this.open_rename_account(rename_id.clone(), window, cx)
                        }),
                    ))
                    .child(icon_button(
                        ElementId::Name(format!("app-settings-account-{id}-delete").into()),
                        IconName::Delete,
                        false,
                        cx.listener(move |this, _, _, cx| {
                            this.open_delete_account(delete_id.clone(), cx)
                        }),
                    )),
            )
    };

    let rows: Vec<AnyElement> = if account.logged_in.is_empty() {
        vec![
            div()
                .py_1()
                .text_size(px(11.))
                .text_color(theme::text_faint())
                .child(SharedString::from("not signed in"))
                .into_any_element(),
        ]
    } else {
        account
            .logged_in
            .iter()
            .map(|agent_type| harness_row(app, &id, agent_type, now_ms, cx))
            .collect()
    };

    div()
        .flex()
        .flex_col()
        .child(header)
        .child(div().flex().flex_col().gap_1().pl_1().pt_1().children(rows))
        .into_any_element()
}

/// One harness under one account: its display name, its last-checked status, and the three
/// things that can be done to a login rather than to the account as a whole.
fn harness_row(
    app: &AppState,
    account: &str,
    agent_type: &str,
    now_ms: i64,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let label = harness_label(app, agent_type).to_string();
    let status = app
        .workbench
        .settings
        .statuses
        .get(&(agent_type.to_string(), account.to_string()));

    let status_line = status.map(|status| {
        let colour = if matches!(status, LoginStatus::Expired { .. }) {
            theme::danger()
        } else if matches!(status, LoginStatus::Missing) {
            theme::text_faint()
        } else {
            theme::text_muted()
        };
        div()
            .text_size(px(11.))
            .text_color(colour)
            .child(SharedString::from(describe_status(status, now_ms)))
            .into_any_element()
    });

    let (check_id, reauth_id, signout_id) = (
        ElementId::Name(format!("app-settings-account-{account}-{agent_type}-check").into()),
        ElementId::Name(format!("app-settings-account-{account}-{agent_type}-reauth").into()),
        ElementId::Name(format!("app-settings-account-{account}-{agent_type}-signout").into()),
    );

    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .py_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme::text())
                        .child(SharedString::from(label)),
                )
                .children(status_line),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(ghost_button(
                    check_id,
                    None,
                    "Check",
                    cx.listener({
                        let account = account.to_string();
                        let agent_type = agent_type.to_string();
                        move |this, _, _, cx| {
                            this.check_harness_login(agent_type.clone(), account.clone(), cx)
                        }
                    }),
                ))
                .child(ghost_button(
                    reauth_id,
                    None,
                    "Re-authenticate",
                    cx.listener({
                        let account = account.to_string();
                        let agent_type = agent_type.to_string();
                        move |this, _, _, cx| {
                            this.reauthenticate_harness(agent_type.clone(), account.clone(), cx)
                        }
                    }),
                ))
                .child(ghost_button(
                    signout_id,
                    None,
                    "Sign out",
                    cx.listener({
                        let account = account.to_string();
                        let agent_type = agent_type.to_string();
                        move |this, _, _, cx| {
                            this.open_sign_out(agent_type.clone(), account.clone(), cx)
                        }
                    }),
                )),
        )
        .into_any_element()
}

/// The rename, delete or sign-out question over one account, drawn from the same place the
/// login modal is: over the settings page, so it layers correctly above it.
pub fn account_dialog(
    app: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    match app.workbench.settings.dialog.clone() {
        None => div().into_any_element(),
        Some(AccountDialog::Rename { account }) => {
            let value = app.account_rename_input.read(cx).value().to_string();
            let enabled = !value.trim().is_empty() && value.trim() != account;
            prompt_modal(
                "app-settings-account-rename",
                "Rename account",
                Some("Every harness signed in here keeps its login and answers to the new name."),
                "Name",
                &app.account_rename_input,
                "Rename",
                enabled,
                crate::ui::handler(&view, |this, _, cx| this.confirm_rename_account(cx)),
                crate::ui::handler(&view, |this, _, cx| this.close_account_dialog(cx)),
                window,
                cx,
            )
        }
        Some(AccountDialog::Delete { account }) => confirm_modal(
            "app-settings-account-delete",
            "Delete account",
            &format!(
                "Delete {account}? Its stored credential and every harness signed in there go \
                 with it. Unlike forgetting a project, there is nothing left behind."
            ),
            "Delete",
            true,
            crate::ui::handler(&view, |this, _, cx| this.confirm_delete_account(cx)),
            crate::ui::handler(&view, |this, _, cx| this.close_account_dialog(cx)),
            window,
        ),
        Some(AccountDialog::SignOut {
            agent_type,
            account,
        }) => {
            let label = harness_label(app, &agent_type).to_string();
            confirm_modal(
                "app-settings-account-signout",
                "Sign out",
                &format!(
                    "Sign {account} out of {label}? {account} keeps its other harnesses — only \
                     this one's credential is removed."
                ),
                "Sign out",
                true,
                crate::ui::handler(&view, |this, _, cx| this.confirm_sign_out(cx)),
                crate::ui::handler(&view, |this, _, cx| this.close_account_dialog(cx)),
                window,
            )
        }
    }
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
    // Only the Running step is a full-screen TUI's terminal; every other step is a short
    // question and stays at the ordinary modal width — a 960px-wide modal holding one text
    // field would look absurd.
    let wide = matches!(login.step, LoginStep::Running { .. });

    let (title, body, footer) = match &login.step {
        LoginStep::Choosing { agent_type } => (
            "Add harness",
            choosing(app, agent_type.as_deref(), window, cx),
            choosing_footer(agent_type.is_some(), app, cx),
        ),
        LoginStep::Starting { agent_type } => (
            "Signing in",
            starting(app, agent_type),
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(ghost_button(
                    "app-settings-login-cancel-starting",
                    None,
                    "Cancel",
                    cx.listener(|this, _, _, cx| this.close_harness_login(cx)),
                ))
                .into_any_element(),
        ),
        LoginStep::Running { pane } => (
            "Signing in",
            running(app, *pane, &login.links, cx),
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

    if wide {
        modal_sized(
            "app-settings-login",
            theme::accent(),
            theme::LOGIN_MODAL_WIDTH,
            Some(theme::LOGIN_MODAL_HEIGHT),
            title,
            body,
            footer,
            crate::ui::handler(&view, |this, _, cx| this.close_harness_login(cx)),
            window,
        )
    } else {
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

/// Between `Choosing` and `Running`: `BeginHarnessLogin` is on its way and nothing has
/// answered yet. Without this step the picker — or, for a re-authentication, nothing at all —
/// sat on screen after the button was pressed, reading as though the click had done nothing.
fn starting(app: &AppState, agent_type: &str) -> AnyElement {
    div()
        .pt_3()
        .child(modal_note(&format!(
            "Starting {}\u{2026}",
            harness_label(app, agent_type)
        )))
        .into_any_element()
}

/// Step two: the harness's own login, in a real terminal.
///
/// This step draws in `modal_sized`'s fill shape (see `login()`), so this whole body — not just
/// the terminal box — is a fixed-size, non-scrolling flex column: the terminal gets `flex_1` and
/// `min_h(0)`, the pattern `ui/terminal.rs::pane` already expects, and actually fills the space
/// because its ancestors now resolve to a real height instead of a scrolling column's hugged one.
/// A box inside a scroller never resolves a height to measure, which is why the emulator — which
/// measures its own bounds to decide the geometry it reports to the harness — used to sit in a
/// hard-coded, cramped box instead.
///
/// The links list is capped rather than left to grow: a long list must not squeeze the terminal
/// down, so it is `flex_none` with a `max_h` and its own scroll once there are enough URLs to
/// need it.
fn running(
    app: &AppState,
    pane: PaneId,
    links: &[String],
    cx: &mut Context<AppState>,
) -> AnyElement {
    let mut body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap_3()
        .pt_3()
        .child(modal_note(
            "Finish the sign-in below. It may open a browser; come back here when it is done.",
        ))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .border_1()
                .border_color(theme::border())
                .child(crate::ui::terminal::pane(app, pane, cx)),
        );

    if !links.is_empty() {
        body = body
            .child(modal_note(
                "The harness's own output printed these — offered as buttons because a \
                 terminal is a poor place to click text.",
            ))
            .child(
                div()
                    .id("app-settings-login-links")
                    .flex_none()
                    .max_h(px(140.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(
                        links
                            .iter()
                            .enumerate()
                            .map(|(index, url)| login_link_row(index, url.clone(), cx)),
                    ),
            );
    }

    body.into_any_element()
}

/// One URL the login pane printed: a button that opens it, and a small icon that copies it.
/// Truncated so a long URL cannot blow out the modal's fixed width — the full string is still
/// the element's tooltip, via [`elided`].
fn login_link_row(index: usize, url: String, cx: &mut Context<AppState>) -> AnyElement {
    let open_url = url.clone();
    let copy_url = url.clone();

    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .id(ElementId::Name(
                    format!("app-settings-login-link-{index}").into(),
                ))
                .flex_1()
                .min_w(px(0.))
                .h(px(24.))
                .px_2()
                .flex()
                .items_center()
                .bg(theme::surface())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::accent())
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(elided(
                    ElementId::Name(format!("app-settings-login-link-{index}-text").into()),
                    url,
                    theme::accent(),
                    12.,
                ))
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&open_url))),
        )
        .child(icon_button(
            ElementId::Name(format!("app-settings-login-link-{index}-copy").into()),
            IconName::Copy,
            false,
            cx.listener(move |_, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_url.clone()));
            }),
        ))
        .into_any_element()
}
