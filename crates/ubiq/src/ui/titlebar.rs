//! The top row: what is open, where it lives, and the switches for the dock's three edge regions.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, ClickEvent, Context, Focusable, InteractiveElement as _, IntoElement, ParentElement,
    StatefulInteractiveElement as _, Styled, Window, div, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::notifications::{Level, UbiqLink};

use crate::app::{AppState, NavBack, NavForward};
use crate::state::MenuId;
use crate::state::ui_id::{self, UiId};
use crate::theme;
use crate::ui::ident::Identified as _;
use crate::ui::kit::{UbiqIcon, badge, field, icon_button, mono};
use crate::ui::navigator;
use crate::ui::project_menu;

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // The three switches report the dock's own regions rather than a flag beside it: a region the
    // user collapsed by dragging its last panel out has to read as closed here too.
    let (left, bottom, right) = app.regions_open(cx);
    // A pane runs in a project's folder, so with none open there is nothing a new one could be
    // started in and the action is not offered.
    let has_project = app.project(cx).is_some();
    // A temporary project (dropped in from outside the catalogue) offers "keep" rather than
    // settings — there is nothing to rename or recolour yet, only a decision to make it real.
    let temporary = app
        .project_snapshot(cx)
        .is_some_and(|snapshot| snapshot.record.temporary);

    div()
        .h(px(theme::titlebar_height()))
        .pr_1()
        .flex()
        .flex_none()
        .items_center()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .ui_id(ui_id::TITLEBAR)
        // The window's letter sits before the picker rather than inside it: one says which
        // window, the other which project. Both return an opaque `impl IntoElement` from
        // `ui::project_menu` (out of this change's scope), so a thin wrapper — matching the
        // row's own flex settings — is what carries the `TITLEBAR_PROJECT` name; `.ui_id()`
        // cannot be chained on an opaque type from outside the module that names it.
        .child(
            div()
                .h_full()
                .flex()
                .flex_none()
                .items_center()
                .ui_id(ui_id::TITLEBAR_PROJECT)
                .children(project_menu::window_badge(app, cx))
                .child(project_menu::render(app, window, cx)),
        )
        .when(has_project, |this| {
            let (icon, label) = if temporary {
                (IconName::Plus, "Keep this project")
            } else {
                (IconName::EllipsisVertical, "Project settings")
            };
            this.child(
                icon_button(
                    "project-settings",
                    icon,
                    app.workbench.project_settings.is_some(),
                    cx.listener(|this, _, _, cx| this.open_edit_project(cx)),
                )
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(label).build(window, cx)
                }),
            )
        })
        .children(repo_link(app, cx))
        // The project's runnable tools, beside its settings and its web link because that is
        // what they are: a property of the project rather than of a terminal. The triangle runs
        // the first tool the project offers; the chevron says what else there is. Both need a
        // folder to run in, the same reason the new-pane control does.
        .when(has_project, |this| {
            let first = app
                .workbench
                .run_tool_rows()
                .first()
                .map(|&at| app.workbench.tools[at].tool.name.clone());
            let tip: gpui::SharedString = match &first {
                Some(name) => format!("Run {name}").into(),
                None => "No tools for this project".into(),
            };
            this.child(
                split_pair()
                    .child(
                        icon_button(
                            "run-tool",
                            IconName::Play,
                            false,
                            cx.listener(|this, _, _, cx| this.run_first_tool(cx)),
                        )
                        .tooltip(move |window, cx| {
                            gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                        }),
                    )
                    .child(split_chevron(
                        "run-tool-menu",
                        None,
                        app.workbench.open_menu == Some(MenuId::RunTool),
                        "Run a tool",
                        cx.listener(|this, event: &ClickEvent, window, cx| {
                            let at = (f32::from(event.position().x), f32::from(event.position().y));
                            this.open_run_tool_menu(at, window, cx);
                        }),
                    )),
            )
        })
        .child(rule())
        // Add a project and its chevron, one split button: the chevron offers the same three ways
        // in the project picker's foot does — add, clone, remote — reached without opening the
        // picker first.
        .child(
            split_pair()
                .child(
                    icon_button(
                        "new-project",
                        IconName::Plus,
                        false,
                        cx.listener(|this, _, _, cx| this.choose_folder(None, cx)),
                    )
                    .h_full()
                    .ui_id(ui_id::TITLEBAR_NEW_PROJECT)
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new("Add a project").build(window, cx)
                    }),
                )
                .child(split_chevron(
                    "new-project-menu",
                    Some(ui_id::TITLEBAR_NEW_PROJECT_MENU),
                    app.workbench.open_menu == Some(MenuId::NewProject),
                    "Open a project",
                    cx.listener(|this, event: &ClickEvent, _, cx| {
                        let at = (f32::from(event.position().x), f32::from(event.position().y));
                        this.open_new_project_menu(at, cx);
                    }),
                )),
        )
        // Back and forward belong to the project they walk, so they sit beside it rather than
        // beside the field: settings, web, tools, a rule, add project, a rule, then the two arrows.
        .child(rule())
        .child(nav_control(
            "nav-back",
            IconName::ChevronLeft,
            nav_label(app, true, cx),
            ui_id::TITLEBAR_NAV_BACK,
            cx.listener(|this, _, window, cx| this.back(&NavBack, window, cx)),
        ))
        .child(nav_control(
            "nav-forward",
            IconName::ChevronRight,
            nav_label(app, false, cx),
            ui_id::TITLEBAR_NAV_FORWARD,
            cx.listener(|this, _, window, cx| this.forward(&NavForward, window, cx)),
        ))
        .child(div().flex_1().min_w(px(0.)))
        .child(command_field(app, window, cx))
        .child(div().flex_1().min_w(px(0.)))
        .child(
            div()
                .h_full()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(1.))
                .ui_id(ui_id::TITLEBAR_ACTIONS)
                // All three switches, in every mode. Every mode now has side panels of its own —
                // the IDE's explorer and chat, Git's refs and changes, KB's documents, the board's
                // task, the agents list, Teams' chat — and a mode that happens to have nothing in a
                // region still opens it onto that region's own furniture.
                .child(
                    icon_button(
                        "toggle-left",
                        UbiqIcon::TitlebarPanelLeft,
                        left,
                        cx.listener(|this, _, window, cx| {
                            this.toggle_region(crate::state::Region::Left, window, cx)
                        }),
                    )
                    .h_full()
                    .ui_id(ui_id::TITLEBAR_REGION_LEFT),
                )
                .child(
                    icon_button(
                        "toggle-bottom",
                        UbiqIcon::TitlebarPanelBottom,
                        bottom,
                        cx.listener(|this, _, window, cx| {
                            this.toggle_region(crate::state::Region::Bottom, window, cx)
                        }),
                    )
                    .h_full()
                    .ui_id(ui_id::TITLEBAR_REGION_BOTTOM),
                )
                .child(
                    icon_button(
                        "toggle-right",
                        UbiqIcon::TitlebarPanelRight,
                        right,
                        cx.listener(|this, _, window, cx| {
                            this.toggle_region(crate::state::Region::Right, window, cx)
                        }),
                    )
                    .h_full()
                    .ui_id(ui_id::TITLEBAR_REGION_RIGHT),
                )
                .child(
                    div()
                        .w(px(1.))
                        .h(px(18.))
                        .mx_1()
                        .flex_none()
                        .bg(theme::border()),
                )
                // The two shortcuts a project offers: a fresh conversation and a fresh shell. Both
                // need a folder to run in, the same reason the new-pane control does.
                .when(has_project, |this| {
                    // New agent and its chevron are one split button: no gap between them, and
                    // the chevron is half the width of a control so the pair reads as one thing
                    // with a second, smaller way in rather than as two buttons (`T-266`). Every
                    // other chevron on the strip is drawn the same way, for the same reason.
                    this.child(
                        split_pair()
                            .child(
                                icon_button(
                                    "new-agent",
                                    IconName::Bot,
                                    false,
                                    cx.listener(|this, _, window, cx| {
                                        this.open_new_agent_direct(window, cx)
                                    }),
                                )
                                .h_full()
                                .ui_id(ui_id::TITLEBAR_NEW_AGENT)
                                .tooltip(move |window, cx| {
                                    gpui_component::tooltip::Tooltip::new("New agent")
                                        .build(window, cx)
                                }),
                            )
                            .child(split_chevron(
                                "hidden-agents-menu",
                                Some(ui_id::TITLEBAR_HIDDEN_AGENTS),
                                app.workbench.open_menu == Some(MenuId::HiddenAgents),
                                "Agents not on screen",
                                cx.listener(|this, event: &ClickEvent, _, cx| {
                                    let at = (
                                        f32::from(event.position().x),
                                        f32::from(event.position().y),
                                    );
                                    this.open_hidden_agents_menu(at, cx);
                                }),
                            )),
                    )
                    // The terminal and its own chevron, the same pair: the button runs the
                    // default shell, the chevron says what else this machine can run here.
                    .child(
                        split_pair()
                            .child(
                                icon_button(
                                    "new-terminal",
                                    IconName::SquareTerminal,
                                    false,
                                    cx.listener(|this, _, window, cx| {
                                        this.new_terminal(window, cx)
                                    }),
                                )
                                .h_full()
                                .ui_id(ui_id::TITLEBAR_NEW_TERMINAL)
                                .tooltip(move |window, cx| {
                                    gpui_component::tooltip::Tooltip::new("New terminal")
                                        .build(window, cx)
                                }),
                            )
                            .child(split_chevron(
                                "new-terminal-menu",
                                Some(ui_id::TITLEBAR_NEW_TERMINAL_MENU),
                                app.workbench.open_menu == Some(MenuId::NewPane),
                                "Run in a new pane",
                                cx.listener(|this, event: &ClickEvent, _, cx| {
                                    let at = (
                                        f32::from(event.position().x),
                                        f32::from(event.position().y),
                                    );
                                    this.open_new_pane_menu(at, cx);
                                }),
                            )),
                    )
                })
                .child(
                    icon_button(
                        "search",
                        UbiqIcon::TitlebarSearch,
                        false,
                        cx.listener(|this, _, window, cx| this.reveal_search(window, cx)),
                    )
                    .h_full()
                    .ui_id(ui_id::TITLEBAR_SEARCH),
                )
                .child(bell(app, cx))
                // The balloon: say something about Ubiq itself. Beside the bell because both are
                // about the application rather than about the project — and always offered, even
                // in a build with no destination, where the modal is what says so.
                .child(
                    icon_button(
                        "feedback",
                        UbiqIcon::BoardComment,
                        app.workbench.feedback.is_some(),
                        cx.listener(|this, _, window, cx| this.open_feedback(window, cx)),
                    )
                    .h_full()
                    .ui_id(ui_id::TITLEBAR_FEEDBACK)
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new("Send feedback").build(window, cx)
                    }),
                )
                // Help: the same panel F1 and the overflow row open, on the page for whatever the
                // window is showing. **Never disabled and never hidden** — a build with no
                // content still opens it, on the page that says how to build some.
                .child(
                    icon_button(
                        "help",
                        UbiqIcon::TitlebarHelp,
                        false,
                        cx.listener(|this, _, window, cx| this.reveal_help(window, cx)),
                    )
                    .h_full()
                    .ui_id(ui_id::TITLEBAR_HELP)
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new("Help").build(window, cx)
                    }),
                )
                // Point at something: the same question Help asks the other way round — Help opens
                // the page for where you are standing, this waits for you to point. Beside it for
                // that reason, and always offered, like Help itself.
                .child(
                    icon_button(
                        "point-at",
                        UbiqIcon::TitlebarPointAt,
                        app.workbench.help_target.is_some(),
                        cx.listener(|this, _, _, cx| this.open_help_target(cx)),
                    )
                    .h_full()
                    .ui_id(ui_id::TITLEBAR_POINT_AT)
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new("Point at something (⇧F1)")
                            .build(window, cx)
                    }),
                )
                // Settings and the overflow chevron, one split button: settings is the thing in
                // there that is reached often enough to stand on the strip, and the chevron holds
                // the rest — remote hosts and connect, web export, window capture, help. See
                // `ui::overflow_menu`.
                .child(
                    split_pair()
                        .child(
                            icon_button(
                                "settings",
                                UbiqIcon::TitlebarSettings,
                                app.workbench.settings.open,
                                cx.listener(|this, _, _, cx| this.toggle_settings(cx)),
                            )
                            .h_full()
                            .ui_id(ui_id::TITLEBAR_SETTINGS)
                            .tooltip(move |window, cx| {
                                gpui_component::tooltip::Tooltip::new("Settings").build(window, cx)
                            }),
                        )
                        .child(split_chevron(
                            "overflow-menu",
                            Some(ui_id::TITLEBAR_OVERFLOW),
                            app.workbench.overflow_menu.is_some(),
                            "More",
                            cx.listener(|this, event: &ClickEvent, _window, cx| {
                                let at = event.position();
                                this.open_overflow_menu((at.x.into(), at.y.into()), cx);
                            }),
                        )),
                )
                .child(
                    icon_button(
                        "theme",
                        if app.workbench.theme_id.mode() == crate::theme::Mode::Dark {
                            UbiqIcon::TitlebarThemeLight
                        } else {
                            UbiqIcon::TitlebarThemeDark
                        },
                        false,
                        cx.listener(|this, _, _, cx| this.toggle_theme(cx)),
                    )
                    .h_full()
                    .ui_id(ui_id::TITLEBAR_THEME),
                ),
        )
}

/// The middle of the titlebar: one field for finding a file and for running a command.
fn command_field(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let focused = app
        .command_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let bar = field(theme::border(), focused)
        .w(px(420.))
        .h_full()
        .px_2()
        .flex_none()
        .gap_2()
        .child(
            Icon::new(UbiqIcon::TitlebarSearch)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Body))
                .child(Input::new(&app.command_input).appearance(false)),
        )
        .child(
            mono("\u{2318}K", theme::text_faint())
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Micro)),
        )
        .ui_id(ui_id::TITLEBAR_COMMAND);
    // The navigator hangs off the field it is typed into: its key context and its handlers go on
    // this div, because the keyboard is in the input inside it.
    navigator::attach(bar, app, cx)
}

/// The 1px rule between groups of controls on the strip.
fn rule() -> gpui::Div {
    div()
        .w(px(1.))
        .h(px(18.))
        .mx_1()
        .flex_none()
        .bg(theme::border())
}

/// The container of a split button: the control and its chevron, flush, as one thing.
fn split_pair() -> gpui::Div {
    div().h_full().flex().flex_none().items_center()
}

/// The narrow chevron half of a split button — the way in that is smaller than the control it
/// hangs off, so the pair reads as one action with a second, quieter door beside it (`T-266`).
///
/// Its own helper rather than [`icon_button`] for the reason [`nav_control`] is one: the kit's
/// button is square by rule, and this one is deliberately not. Every chevron on the strip is drawn
/// through here, so they are one shape rather than a mixture of split halves and square buttons.
fn split_chevron(
    id: &'static str,
    mark: Option<UiId>,
    open: bool,
    tip: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let mut root = div()
        .id(id)
        .w(px(theme::split_chevron_width()))
        .h_full()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer();
    if let Some(mark) = mark {
        root = root.ui_id(mark);
    }
    if open {
        root = root.bg(theme::accent_soft());
    }
    root.hover(|this| this.bg(theme::hover()))
        .child(
            Icon::new(IconName::ChevronDown)
                .with_size(Size::Size(theme::icon_sm()))
                .text_color(if open {
                    theme::accent()
                } else {
                    theme::text_muted()
                }),
        )
        .on_click(on_click)
        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tip).build(window, cx))
}

/// What the press in one direction would land on, named the way the user reads places: a path
/// where there is one, and the project's name in front of it when it is not the one on screen.
fn nav_label(app: &AppState, back: bool, cx: &App) -> Option<String> {
    let dest = app.nav.peek(back)?;
    let label = dest.label();
    if Some(dest.project) == app.project(cx) {
        return Some(label);
    }
    let name = crate::state::WindowRegistry::read(cx)
        .project(dest.project)
        .map(|snapshot| snapshot.record.name.clone())?;
    Some(format!("{name} · {label}"))
}

/// Back and forward, flush in the row like every other piece of chrome.
///
/// Its own helper rather than [`icon_button`] because these two are the only controls with
/// nowhere to go: with no target they are drawn faint and answer neither the pointer nor a click.
fn nav_control(
    id: &'static str,
    icon: IconName,
    target: Option<String>,
    ui_id: UiId,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let live = target.is_some();
    div()
        .id(id)
        .ui_id(ui_id)
        .w(px(30.))
        .h_full()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .child(Icon::new(icon).with_size(Size::Small).text_color(if live {
            theme::text_muted()
        } else {
            theme::text_faint()
        }))
        .when_some(target, |this, label| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .on_click(on_click)
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(label.clone()).build(window, cx)
                })
        })
}

/// The bell, its unread badge, and the flash that says something just arrived.
///
/// Its own helper rather than a bare [`icon_button`] for the reason [`nav_control`] is one: this
/// control carries two things the kit's square button has no room for — a count over the glyph,
/// and a tint that alternates while the bell is flashing. The glyph itself is the kit's, the
/// button's rhythm is the row's 30px, and the badge is drawn over it rather than beside it so the
/// strip's spacing does not shift when a count appears.
fn bell(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let state = &app.notifications;
    let unread = state.wire.unread();
    // The flash is the level's colour, on the beat the blink is on. Off the beat, and at rest,
    // the bell reads as any other control in the row.
    let flashing = state.flash_on.then_some(state.flash_level).flatten();
    let tint = flashing.map(|level| match level {
        Level::Info => theme::info(),
        Level::Warning => theme::warning(),
        Level::Error => theme::danger(),
    });
    // A flashing bell that carries a link goes there instead of opening the list, so the tooltip
    // says where rather than repeating the name of a control the user is already looking at.
    let tip: gpui::SharedString = match state
        .flash_until
        .is_some()
        .then_some(state.flash_link.as_ref())
        .flatten()
    {
        Some(link) => format!("Notifications — go to {}", link_label(link)).into(),
        None => "Notifications".into(),
    };

    div()
        .relative()
        .flex()
        .flex_none()
        .h_full()
        .ui_id(ui_id::TITLEBAR_NOTIFICATIONS)
        .child(
            icon_button(
                "bell",
                UbiqIcon::TitlebarNotifications,
                state.open,
                cx.listener(|this, _, window, cx| this.toggle_notifications(window, cx)),
            )
            .h_full()
            .when_some(tint, |this, colour| {
                this.child(
                    div()
                        .absolute()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size_full()
                        .child(
                            Icon::new(UbiqIcon::TitlebarNotifications)
                                .with_size(Size::Small)
                                .text_color(colour),
                        ),
                )
            })
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
            }),
        )
        .when(unread > 0, |this| {
            // Past ninety-nine the number has stopped being information; that there are many is.
            let count = if unread > 99 {
                "99+".to_string()
            } else {
                unread.to_string()
            };
            this.child(
                div()
                    .absolute()
                    .top(px(3.))
                    .right(px(1.))
                    .px(px(3.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::danger_soft())
                    .child(badge(&count, theme::danger())),
            )
        })
}

/// Where a link goes, in the few words a tooltip has room for. The ids name things this row
/// cannot resolve without the catalogue in front of it, so each says what kind of place it is.
fn link_label(link: &UbiqLink) -> &'static str {
    match link {
        UbiqLink::Pane(_) => "the pane",
        UbiqLink::Project(_) => "the project",
        UbiqLink::Agent(_) => "the conversation",
        UbiqLink::File { .. } => "the file",
        UbiqLink::Url(_) => "the page",
    }
}

/// The repository's page on its provider, when the project's default remote names one.
///
/// Nothing is drawn for a project that is not a repository, has no remote, or has a remote no
/// browser could open — a local path, or a host this does not know the page shape of.
fn repo_link(app: &AppState, cx: &App) -> Option<impl IntoElement> {
    let overview = app.open_project(cx)?.git.as_ref()?;
    let remote = overview.remotes.iter().find(|remote| remote.is_default)?;
    let url = ubiq_proto::git::web_url(&remote.url)?;
    let label: gpui::SharedString = format!("Open {url}").into();
    Some(
        icon_button(
            "repo-link",
            UbiqIcon::TitlebarBrowser,
            false,
            move |_, _, cx| cx.open_url(&url),
        )
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(label.clone()).build(window, cx)
        }),
    )
}
