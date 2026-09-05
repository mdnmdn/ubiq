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
use ubiq_proto::connectors::{
    AuthKind, CertReason, Connection, InstanceNeed, OauthApp, ProviderId, TrustedCert, origin,
};
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::{AccountInfo, CliShortcutAction, LoginStatus};

use crate::app::AppState;
use crate::state::settings::{
    AccountDialog, CliShortcut, ConnectStep, ConnectorDialog, LoginStep, MarkdownOpen,
    SettingsSection, connect_error_note, describe_status,
};
use crate::theme;
use crate::ui::kit::{
    badge, check_box, choice_pill, column, confirm_modal, elided, field, ghost_button, heading,
    icon_button, label_block, modal, modal_note, modal_sized, mono, nav_item, primary_button,
    prompt_modal, section_label, setting_row, slab, state_chip,
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
        SettingsSection::Appearance => IconName::Palette,
        SettingsSection::FileExplorer => IconName::Folder,
        SettingsSection::Editor => IconName::File,
        SettingsSection::Search => IconName::Search,
        SettingsSection::Harnesses => IconName::Asterisk,
        SettingsSection::Connectors => IconName::Globe,
        SettingsSection::CommandLine => IconName::SquareTerminal,
    }
}

fn body(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let content = match app.workbench.settings.nav {
        SettingsSection::Appearance => appearance(app, cx),
        SettingsSection::FileExplorer => file_explorer(app, cx),
        SettingsSection::Editor => editor(app, cx),
        SettingsSection::Search => search(app),
        SettingsSection::Harnesses => harnesses(app, cx),
        SettingsSection::Connectors => connectors(app, cx),
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

fn appearance(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    column(vec![
        heading("Appearance", "What the window's own chrome shows."),
        setting_row(
            "Open projects in the rail",
            "The projects this window holds, as coloured badges under the mode icons \u{2014} the \
             most recent first, and only as many as the rail has room for.",
            check_box(
                "app-settings-rail-projects",
                app.workbench.settings.ui.rail_projects,
                cx.listener(|this, _, _, cx| this.toggle_rail_projects(cx)),
            )
            .into_any_element(),
        ),
    ])
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
    let vim = app.workbench.settings.ui.vim_mode;
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
        setting_row(
            "Vim mode",
            "Modal editing in the code editor and in every multi-line box \u{2014} the chat \
             composer, an agent's input, a task description. Single-line fields are unaffected. \
             The status bar reports the mode, and switches this on and off too.",
            check_box(
                "app-settings-vim-mode",
                vim,
                cx.listener(|this, _, _, cx| this.toggle_vim_mode(cx)),
            )
            .into_any_element(),
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
            if login.probe {
                "Starting shell"
            } else {
                "Signing in"
            },
            starting(app, agent_type, login.probe),
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
            if login.probe { "Shell" } else { "Signing in" },
            running(app, *pane, &login.links, login.probe, cx),
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
            if login.probe {
                "Shell closed"
            } else if *captured {
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

/// Step one's footer. Both actions are dead until a harness is picked, because the other half
/// of what they need — the name — is in a field this function cannot read without a window.
///
/// `Shell` sits beside `Sign in` rather than replacing it: it is a diagnostic, not another way
/// to sign in, which is why it wears the ghost treatment and a tooltip rather than the primary
/// button's weight.
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
            ghost_button(
                "app-settings-login-shell",
                None,
                "Shell",
                cx.listener(|this, _, _, cx| this.start_harness_shell(cx)),
            )
            .when(!picked, |button| button.opacity(0.5))
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new(
                    "A shell inside this login's sandbox \u{2014} for checking what it can \
                     reach. Signs nobody in.",
                )
                .build(window, cx)
            }),
        )
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
fn starting(app: &AppState, agent_type: &str, probe: bool) -> AnyElement {
    let text = if probe {
        format!(
            "Starting a shell in {}'s sandbox\u{2026}",
            harness_label(app, agent_type)
        )
    } else {
        format!("Starting {}\u{2026}", harness_label(app, agent_type))
    };
    div().pt_3().child(modal_note(&text)).into_any_element()
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
    probe: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let note = if probe {
        "A shell inside this login's sandbox \u{2014} for checking what it can reach. Signs \
         nobody in."
    } else {
        "Finish the sign-in below. It may open a browser; come back here when it is done."
    };
    let mut body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap_3()
        .pt_3()
        .child(modal_note(note))
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

/// The identities Ubiq holds at external services.
///
/// Built like [`harnesses`] and reading the same three lists — connections, pinned certificates
/// and configured OAuth applications — off `host`, which is where the host keeps them. Nothing
/// here is mirrored into interface state, and nothing here calls the network: a row's status is
/// whatever the host last said, never something a render asks for.
fn connectors(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let mut rows = vec![heading(
        "Connectors",
        "Identities at GitHub, GitLab and the rest \u{2014} used to read issues and open pull \
         requests. Several per provider is ordinary: a work account and a personal one are two \
         connections, not a conflict.",
    )];
    if let Some(error) = app.workbench.settings.error.clone() {
        rows.push(error_banner(&error, cx));
    }
    rows.push(
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(primary_button(
                "app-settings-connect",
                Some(IconName::Plus),
                "Connect\u{2026}",
                cx.listener(|this, _, window, cx| this.open_connect(window, cx)),
            ))
            .into_any_element(),
    );

    let connections = &app.workbench.settings.host.connections;
    if connections.is_empty() {
        rows.push(
            div()
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
                        .child(SharedString::from("No connections.")),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme::text_faint())
                        .child(SharedString::from(
                            "Connect one to let agents reach its issues and pull requests.",
                        )),
                )
                .into_any_element(),
        );
    } else {
        let now_ms = chrono::Utc::now().timestamp_millis();
        rows.push(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .children(
                    connections
                        .iter()
                        .map(|connection| connection_row(app, connection, now_ms, cx)),
                )
                .into_any_element(),
        );
    }

    rows.push(trusted_certs(app, cx));
    rows.push(oauth_apps(app, cx));
    column(rows)
}

/// How many connections live at an origin — what a "forget this certificate" question has to
/// say out loud, since a pin is instance-wide rather than per connection.
fn certificate_uses(app: &AppState, at: &str) -> usize {
    app.workbench
        .settings
        .host
        .connections
        .iter()
        .filter(|connection| {
            connection
                .instance
                .as_deref()
                .and_then(origin)
                .is_some_and(|from| from == at)
        })
        .count()
}

/// One connection: who it is, where, and what the host last said about its token.
///
/// The status is read out of what already arrived — never asked for here. A `CheckConnection`
/// from a render would be a network call on every frame, which is exactly what the `probe` flag
/// on that message exists to keep out of this path.
fn connection_row(
    app: &AppState,
    connection: &Connection,
    now_ms: i64,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = connection.id;
    let label = connection.label.clone();
    let status = app.workbench.settings.connection_status.get(&id);
    let pinned = connection
        .instance
        .as_deref()
        .and_then(origin)
        .is_some_and(|at| {
            app.workbench
                .settings
                .host
                .trusted_certs
                .iter()
                .any(|cert| cert.origin == at)
        });

    let chip = status.map(|status| {
        let (colour, text) = match status {
            LoginStatus::Valid { .. } => (theme::success(), "valid"),
            LoginStatus::Expired { .. } => (theme::danger(), "expired"),
            LoginStatus::Unknown => (theme::text_faint(), "unknown"),
            LoginStatus::Missing => (theme::text_faint(), "missing"),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(state_chip(text, colour, 1.0))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::text_faint())
                    .child(SharedString::from(describe_status(status, now_ms))),
            )
            .into_any_element()
    });

    let where_it_lives = connection
        .instance
        .clone()
        .unwrap_or_else(|| "cloud".to_string());

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
                .flex()
                .items_center()
                .gap_2()
                .flex_1()
                .min_w(px(0.))
                .child(badge(connection.provider.glyph(), theme::accent()))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme::text())
                        .child(SharedString::from(label.clone())),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme::text_muted())
                        .child(SharedString::from(connection.account.clone())),
                )
                // A self-hosted base URL can be long enough to push everything else off the
                // panel, so it is elided with the whole of it as the tooltip.
                .child(elided(
                    ElementId::Name(format!("app-settings-connection-{id}-instance").into()),
                    where_it_lives,
                    theme::text_faint(),
                    11.,
                ))
                .children(chip)
                .when(pinned, |row| row.child(badge("pinned", theme::warning()))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(ghost_button(
                    ElementId::Name(format!("app-settings-connection-{id}-check").into()),
                    None,
                    "Check",
                    cx.listener(move |this, _, _, cx| this.check_connection(id, cx)),
                ))
                .child(icon_button(
                    ElementId::Name(format!("app-settings-connection-{id}-rename").into()),
                    IconName::Replace,
                    false,
                    cx.listener({
                        let label = label.clone();
                        move |this, _, window, cx| {
                            this.open_rename_connection(id, label.clone(), window, cx)
                        }
                    }),
                ))
                .child(icon_button(
                    ElementId::Name(format!("app-settings-connection-{id}-disconnect").into()),
                    IconName::Delete,
                    false,
                    cx.listener(move |this, _, _, cx| this.open_disconnect(id, label.clone(), cx)),
                )),
        )
        .into_any_element()
}

/// An epoch-second timestamp as a date. The host sends seconds and has no opinion about how a
/// date is written; this is that opinion, in one place.
fn on_day(epoch_seconds: i64) -> String {
    chrono::DateTime::from_timestamp(epoch_seconds, 0)
        .map(|at| at.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// A SHA-256 as `AB:CD:` groups, which is how every other tool prints one and therefore the only
/// form a user can check against what their administrator told them.
fn fingerprint(sha256: &str) -> String {
    sha256
        .to_uppercase()
        .as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).to_string())
        .collect::<Vec<_>>()
        .join(":")
}

/// The certificates the user has vouched for, one line each.
///
/// A pin is keyed by origin rather than by connection, so a line says how many connections stop
/// trusting the server if it goes — the number the confirmation repeats.
fn trusted_certs(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let certs = app.workbench.settings.host.trusted_certs.clone();
    if certs.is_empty() {
        return div().into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_4()
        .child(section_label("Trusted certificates"))
        .children(certs.iter().map(|cert| cert_row(app, cert, cx)))
        .into_any_element()
}

fn cert_row(app: &AppState, cert: &TrustedCert, cx: &mut Context<AppState>) -> AnyElement {
    let uses = certificate_uses(app, &cert.origin);
    let short: String = fingerprint(&cert.sha256).chars().take(23).collect();
    let origin = cert.origin.clone();

    div()
        .flex()
        .items_center()
        .gap_2()
        .py_1()
        .child(
            div()
                .w(px(200.))
                .text_size(px(11.))
                .text_color(theme::text())
                .child(SharedString::from(cert.origin.clone())),
        )
        .child(mono(short, theme::text_muted()).text_size(px(11.)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(11.))
                .text_color(theme::text_faint())
                .child(SharedString::from(format!(
                    "{} \u{b7} until {} \u{b7} {uses} connection{}",
                    cert.issuer,
                    on_day(cert.not_after),
                    if uses == 1 { "" } else { "s" }
                ))),
        )
        .child(ghost_button(
            ElementId::Name(format!("app-settings-cert-{origin}-forget").into()),
            None,
            "Forget",
            cx.listener(move |this, _, _, cx| this.open_forget_cert(origin.clone(), uses, cx)),
        ))
        .into_any_element()
}

/// The OAuth applications Ubiq authenticates *as*, where one was configured rather than built in.
///
/// The client id is public and rides the settings blob; only the secret is material, which is why
/// the row says whether one is set rather than showing anything.
fn oauth_apps(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let apps = app.workbench.settings.host.oauth_apps.clone();
    if apps.is_empty() {
        return div().into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_4()
        .child(section_label("OAuth applications"))
        .children(apps.iter().map(|entry| oauth_row(entry, cx)))
        .into_any_element()
}

fn oauth_row(entry: &OauthApp, cx: &mut Context<AppState>) -> AnyElement {
    let where_it_is = entry.origin.clone().unwrap_or_else(|| "cloud".to_string());
    let (provider, origin) = (entry.provider, entry.origin.clone());
    let clear_origin = origin.clone();
    let (chip, colour) = if entry.has_secret {
        ("secret set", theme::success())
    } else {
        ("no secret", theme::text_faint())
    };

    setting_row(
        &format!("{} \u{b7} {where_it_is}", entry.provider.label()),
        &format!(
            "Registered on this instance rather than built in, so {} is the id every \
             authorization URL carries.",
            entry.client_id
        ),
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(badge(chip, colour))
            .child(ghost_button(
                ElementId::Name(
                    format!("app-settings-oauth-{provider:?}-{where_it_is}-edit").into(),
                ),
                None,
                "Edit",
                cx.listener(move |this, _, window, cx| {
                    this.open_app_secret(provider, origin.clone(), window, cx)
                }),
            ))
            .child(ghost_button(
                ElementId::Name(
                    format!("app-settings-oauth-{provider:?}-{where_it_is}-clear").into(),
                ),
                None,
                "Clear",
                cx.listener(move |this, _, _, cx| {
                    this.clear_app_secret(provider, clear_origin.clone(), cx)
                }),
            ))
            .into_any_element(),
    )
}

/// The connect modal: pick a provider and a flow, then watch it run.
///
/// A near-twin of [`login`], and a modal for the same reason: a browser flow wants the whole of
/// the user's attention for the half-minute it takes. Every step is leavable, and leaving sends
/// `CancelConnect` — a flow that stored no token left nothing behind.
///
/// Which flows a provider offers is read off the table in `ubiq_proto::connectors` rather than
/// branched on here: an Azure DevOps Server connection is simply never shown a browser button.
pub fn connect(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(connect) = &app.workbench.settings.connect else {
        return div().into_any_element();
    };
    let view = cx.entity();

    let cancel = |id: &'static str, cx: &mut Context<AppState>| {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(ghost_button(
                id,
                None,
                "Cancel",
                cx.listener(|this, _, window, cx| this.cancel_connect(window, cx)),
            ))
            .into_any_element()
    };

    let (title, body, footer) = match &connect.step {
        ConnectStep::Choosing { provider, auth } => (
            "Connect",
            choosing_connector(app, *provider, *auth, window, cx),
            connect_footer(app, provider.is_some() && auth.is_some(), cx),
        ),
        ConnectStep::Starting | ConnectStep::Opening => (
            "Connecting",
            div()
                .pt_3()
                .child(modal_note(&format!(
                    "Starting a {} connection\u{2026}",
                    connect.provider.label()
                )))
                .into_any_element(),
            cancel("app-settings-connect-cancel-starting", cx),
        ),
        ConnectStep::DeviceCode {
            user_code,
            verification_url,
            expires_in,
        } => (
            "Enter this code",
            device_code(user_code, verification_url, *expires_in, cx),
            cancel("app-settings-connect-cancel-device", cx),
        ),
        ConnectStep::AwaitingCallback { port, url } => (
            "Waiting for the browser",
            div()
                .flex()
                .flex_col()
                .gap_3()
                .pt_3()
                .child(modal_note(&format!(
                    "Finish the sign-in in your browser. Ubiq is listening on port {port} for the \
                     answer; the link is here in case the browser did not open."
                )))
                .child(login_link_row(0, url.clone(), cx))
                .into_any_element(),
            cancel("app-settings-connect-cancel-callback", cx),
        ),
        ConnectStep::Exchanging => (
            "Connecting",
            div()
                .pt_3()
                .child(modal_note("Trading that for an identity\u{2026}"))
                .into_any_element(),
            cancel("app-settings-connect-cancel-exchanging", cx),
        ),
        ConnectStep::NeedSecret { prompt } => (
            "Paste a token",
            need_secret(app, prompt, window, cx),
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(ghost_button(
                    "app-settings-connect-cancel-secret",
                    None,
                    "Cancel",
                    cx.listener(|this, _, window, cx| this.cancel_connect(window, cx)),
                ))
                .child(primary_button(
                    "app-settings-connect-submit-secret",
                    None,
                    "Continue",
                    cx.listener(|this, _, window, cx| this.submit_connect_secret(window, cx)),
                ))
                .into_any_element(),
        ),
        ConnectStep::AwaitingCertificate => (
            "Waiting on a certificate",
            div()
                .pt_3()
                .child(modal_note(
                    "This server's certificate did not validate. The question is over this \
                     modal; nothing continues until it is answered.",
                ))
                .into_any_element(),
            cancel("app-settings-connect-cancel-cert", cx),
        ),
        ConnectStep::Failed { error } => (
            "Not connected",
            div()
                .pt_3()
                .child(modal_note(&connect_error_note(error)))
                .into_any_element(),
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(ghost_button(
                    "app-settings-connect-close",
                    None,
                    "Close",
                    cx.listener(|this, _, window, cx| this.cancel_connect(window, cx)),
                ))
                // Back to the picker with the fields as they were left: a wrong URL is corrected
                // by editing it, not by typing the whole form again.
                .child(primary_button(
                    "app-settings-connect-retry",
                    None,
                    "Try again",
                    cx.listener(|this, _, _, cx| this.retry_connect(cx)),
                ))
                .into_any_element(),
        ),
    };

    modal(
        "app-settings-connect-modal",
        theme::accent(),
        title,
        body,
        footer,
        crate::ui::handler(&view, |this, window, cx| this.cancel_connect(window, cx)),
        window,
    )
}

/// Step one: which provider, where it lives, and which flow.
///
/// The instance field asks for a **base URL**, not a host name: an on-premises install can live
/// under a path, and `origin` refuses anything without a scheme rather than guessing one.
fn choosing_connector(
    app: &AppState,
    chosen: Option<ProviderId>,
    auth: Option<AuthKind>,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let typed = app.connect_instance_input.read(cx).value().to_string();
    let self_hosted = !typed.trim().is_empty();
    let focused = |input: &gpui::Entity<gpui_component::input::InputState>| {
        input.read(cx).focus_handle(cx).is_focused(window)
    };

    let providers: Vec<AnyElement> = ProviderId::all()
        .iter()
        .copied()
        .map(|provider| {
            choice_pill(
                ElementId::Name(format!("app-settings-connect-provider-{provider:?}").into()),
                provider.label(),
                chosen == Some(provider),
                cx.listener(move |this, _, _, cx| this.pick_connect_provider(provider, cx)),
            )
            .into_any_element()
        })
        .collect();

    let mut body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(
            "Ubiq signs in on your behalf and keeps the token in the machine's credential \
             store. Nothing is written to a file you could paste into an issue.",
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Provider",
                    "Which service this identity is at.",
                ))
                .child(div().flex().flex_wrap().gap_2().children(providers)),
        );

    if let Some(provider) = chosen {
        if provider.instance_need() != InstanceNeed::Never {
            let note = match provider.instance_need() {
                InstanceNeed::Required => {
                    "The base URL of the install \u{2014} there is no hosted service for this one."
                }
                _ => "The base URL of a self-managed install. Empty is the provider's own cloud.",
            };
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(label_block("Instance", note))
                    .child(
                        field(theme::border(), focused(&app.connect_instance_input))
                            .h(px(30.))
                            .px_2()
                            .child(Input::new(&app.connect_instance_input).appearance(false)),
                    ),
            );
        }

        // Read off the provider table rather than branched on here: a provider with no flow at
        // this location offers nothing, and says so.
        let flows = provider.flows(self_hosted);
        let pills: Vec<AnyElement> = flows
            .iter()
            .copied()
            .map(|kind| {
                choice_pill(
                    ElementId::Name(format!("app-settings-connect-auth-{kind:?}").into()),
                    auth_label(kind),
                    auth == Some(kind),
                    cx.listener(move |this, _, _, cx| this.pick_connect_auth(kind, cx)),
                )
                .into_any_element()
            })
            .collect();

        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "How",
                    if flows.is_empty() {
                        "No flow works here. Check the instance URL."
                    } else {
                        "A pasted token needs no registered application; a browser flow does."
                    },
                ))
                .child(div().flex().flex_wrap().gap_2().children(pills)),
        );

        if let Some(kind) = auth
            && provider.needs_client_id(kind, self_hosted)
        {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(label_block(
                        "Application id",
                        "A browser flow on a self-managed install uses an application registered \
                         on that install \u{2014} whoever administers it has the id.",
                    ))
                    .child(
                        field(theme::border(), focused(&app.connect_client_id_input))
                            .h(px(30.))
                            .px_2()
                            .child(Input::new(&app.connect_client_id_input).appearance(false)),
                    ),
            );
        }
    }

    body.child(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(label_block(
                "Name",
                "What to call this identity \u{2014} \"work\", \"personal\". Freely renamed later.",
            ))
            .child(
                field(theme::border(), focused(&app.login_account_input))
                    .h(px(30.))
                    .px_2()
                    .child(Input::new(&app.login_account_input).appearance(false)),
            ),
    )
    .into_any_element()
}

/// How a flow reads in the picker. The wire's own names are about mechanism; these are about
/// what the user is about to do.
fn auth_label(kind: AuthKind) -> &'static str {
    match kind {
        AuthKind::Token => "Paste a token",
        AuthKind::Device => "Enter a code",
        AuthKind::Oauth => "Open a browser",
        AuthKind::Probe => "Check",
    }
}

/// Step one's footer. The confirm is dead until both pills are picked and the name is typed —
/// the name is read out of the field here, since that is the only place with a `cx` to read it.
fn connect_footer(app: &AppState, picked: bool, cx: &mut Context<AppState>) -> AnyElement {
    let named = !app.login_account_input.read(cx).value().trim().is_empty();
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "app-settings-connect-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, window, cx| this.cancel_connect(window, cx)),
        ))
        .child(
            primary_button(
                "app-settings-connect-start",
                None,
                "Connect",
                cx.listener(|this, _, _, cx| this.start_connect(cx)),
            )
            .when(!(picked && named), |button| button.opacity(0.5)),
        )
        .into_any_element()
}

/// The device flow: a code to type somewhere else. Drawn large and monospaced because it is
/// transcribed by hand, and offered to the clipboard beside it.
fn device_code(
    user_code: &str,
    verification_url: &str,
    expires_in: u64,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let copy = user_code.to_string();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(&format!(
            "Open the link below and type this code. It is good for about {} minutes.",
            expires_in / 60
        )))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    slab(theme::accent())
                        .px_3()
                        .py_2()
                        .child(mono(user_code.to_string(), theme::text()).text_size(px(22.))),
                )
                .child(icon_button(
                    "app-settings-connect-code-copy",
                    IconName::Copy,
                    false,
                    cx.listener(move |_, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                    }),
                )),
        )
        .child(login_link_row(0, verification_url.to_string(), cx))
        .into_any_element()
}

/// The token step. The field is plain: this kit has no masked input, so a pasted token is on
/// screen until the modal closes.
// ponytail: unmasked secret field. Masking belongs in `kit::field`, not here, and nothing else
// needs it yet.
fn need_secret(
    app: &AppState,
    prompt: &str,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let focused = app
        .connect_secret_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(prompt))
        .child(
            field(theme::border(), focused)
                .h(px(30.))
                .px_2()
                .child(Input::new(&app.connect_secret_input).appearance(false)),
        )
        .into_any_element()
}

/// The certificate a flow stopped on, as something to read rather than click through.
///
/// Every field here is public — a certificate is what a server hands anyone who connects — and
/// all of it is on screen because the point is that the user checks it against what their
/// administrator told them. Dismissing is declining: the flow stays stopped.
pub fn certificate(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(prompt) = &app.workbench.settings.cert else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let cert = prompt.cert.clone();
    let copy = fingerprint(&cert.sha256);

    let reason = match cert.reason {
        CertReason::UnknownIssuer if cert.self_signed => {
            "This certificate signs for itself: nothing vouches for it but the server offering it."
        }
        CertReason::UnknownIssuer => "Nothing this machine trusts vouches for this certificate.",
        CertReason::HostnameMismatch => {
            "This certificate is for a different name than the one being connected to."
        }
        CertReason::Expired => "This certificate has expired.",
        CertReason::NotYetValid => "This certificate is not valid yet.",
    };

    let detail = |label: &str, value: String| {
        div()
            .flex()
            .gap_2()
            .child(
                div()
                    .w(px(120.))
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(theme::text_faint())
                    .child(SharedString::from(label.to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(px(11.))
                    .text_color(theme::text())
                    .child(SharedString::from(value)),
            )
    };

    let body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(reason))
        .child(modal_note(&format!(
            "It belongs to {}. A pin is keyed by the server, so every connection to it \u{2014} \
             now and later \u{2014} shares this answer.",
            prompt.origin
        )))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(detail("Subject", cert.subject.clone()))
                .child(detail(
                    "Alternative names",
                    if cert.sans.is_empty() {
                        "none".to_string()
                    } else {
                        cert.sans.join(", ")
                    },
                ))
                .child(detail("Issuer", cert.issuer.clone()))
                .child(detail("Valid from", on_day(cert.not_before)))
                .child(detail("Valid to", on_day(cert.not_after))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    slab(theme::warning())
                        .flex_1()
                        .min_w(px(0.))
                        .px_2()
                        .py_2()
                        .child(mono(copy.clone(), theme::text()).text_size(px(11.))),
                )
                .child(icon_button(
                    "app-settings-cert-copy",
                    IconName::Copy,
                    false,
                    cx.listener(move |_, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                    }),
                )),
        )
        .into_any_element();

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "app-settings-cert-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.cancel_certificate(cx)),
        ))
        .child(primary_button(
            "app-settings-cert-trust",
            None,
            "Trust this certificate",
            cx.listener(|this, _, _, cx| this.trust_certificate(cx)),
        ))
        .into_any_element();

    modal(
        "app-settings-cert",
        theme::warning(),
        "Check this certificate",
        body,
        footer,
        crate::ui::handler(&view, |this, _, cx| this.cancel_certificate(cx)),
        window,
    )
}

/// The rename, disconnect or forget-certificate question over the connectors section, drawn
/// from the same place the connect modal is so it layers above it.
pub fn connector_dialog(
    app: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    match app.workbench.settings.connector.clone() {
        None => div().into_any_element(),
        Some(ConnectorDialog::Rename { label, .. }) => {
            let value = app.account_rename_input.read(cx).value().to_string();
            let enabled = !value.trim().is_empty() && value.trim() != label;
            prompt_modal(
                "app-settings-connection-rename",
                "Rename connection",
                Some(
                    "The name is yours; everything that references this connection keeps working.",
                ),
                "Name",
                &app.account_rename_input,
                "Rename",
                enabled,
                crate::ui::handler(&view, |this, _, cx| this.confirm_rename_connection(cx)),
                crate::ui::handler(&view, |this, _, cx| this.close_connector_dialog(cx)),
                window,
                cx,
            )
        }
        Some(ConnectorDialog::Disconnect { label, .. }) => confirm_modal(
            "app-settings-connection-disconnect",
            "Disconnect",
            &format!(
                "Disconnect {label}? Its stored token goes with it. Any certificate you pinned \
                 for that server stays \u{2014} it belongs to the server, and forgetting it is \
                 its own action below."
            ),
            "Disconnect",
            true,
            crate::ui::handler(&view, |this, _, cx| this.confirm_disconnect(cx)),
            crate::ui::handler(&view, |this, _, cx| this.close_connector_dialog(cx)),
            window,
        ),
        Some(ConnectorDialog::ForgetCert { origin, uses }) => confirm_modal(
            "app-settings-cert-forget",
            "Forget certificate",
            &format!(
                "Stop trusting the certificate at {origin}? {uses} connection{} live there, and \
                 the next request to it validates normally \u{2014} which is what failed before \
                 you vouched for it.",
                if uses == 1 { "" } else { "s" }
            ),
            "Forget",
            true,
            crate::ui::handler(&view, |this, _, cx| this.confirm_forget_cert(cx)),
            crate::ui::handler(&view, |this, _, cx| this.close_connector_dialog(cx)),
            window,
        ),
        Some(ConnectorDialog::AppSecret { .. }) => {
            let enabled = !app.connect_secret_input.read(cx).value().trim().is_empty();
            prompt_modal(
                "app-settings-oauth-secret",
                "Client secret",
                Some(
                    "The secret of the application Ubiq authenticates as \u{2014} not your own \
                     credential. It is kept in the credential store, never in the settings file.",
                ),
                "Secret",
                &app.connect_secret_input,
                "Save",
                enabled,
                crate::ui::handler(&view, |this, window, cx| {
                    this.confirm_app_secret(window, cx)
                }),
                crate::ui::handler(&view, |this, _, cx| this.close_connector_dialog(cx)),
                window,
                cx,
            )
        }
    }
}
