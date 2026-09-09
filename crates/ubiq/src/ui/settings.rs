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
use gpui_component::input::Input;
use gpui_component::{Icon, IconName};
use ubiq_proto::assist::{AiProviderInfo, AiProviderKind, AssistProvider, ModelRole};
use ubiq_proto::connectors::{
    AuthKind, CertReason, Connection, InstanceNeed, OAUTH_REDIRECT, OauthApp, ProviderId,
    TrustedCert, origin,
};
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::{AccountInfo, CliShortcutAction, LoginStatus, ProfileInfo};
use ubiq_proto::projects::IndexLevel;
use ubiq_proto::settings::AgentHome;

use crate::app::{AppState, HostEntry, HostId, HostRef, host_menu_rows, host_row_label};
use crate::state::settings::{
    AccountDialog, AiProviderForm, AssistInfo, CliShortcut, ConnectApp, ConnectStep,
    ConnectorDialog, LoginStep, MarkdownOpen, SettingsSection, connect_error_note, describe_status,
    magnitude,
};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{
    UbiqIcon, badge, card, check_box, choice_pill, column, confirm_modal, elided, field,
    ghost_button, heading, icon_button, label_block, menu::Picker, modal, modal_note, modal_sized,
    mono, nav_item, primary_button, prompt_modal, removable_tag, section_label, setting_row, slab,
    state_chip, status_dot,
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
        .border_l(px(theme::accent_edge()))
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
                .text_size(theme::font(Family::Chrome, Role::Title))
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

fn nav_icon(item: SettingsSection) -> Icon {
    match item {
        SettingsSection::Appearance => IconName::Palette.into(),
        SettingsSection::FileExplorer => IconName::Folder.into(),
        SettingsSection::Editor => IconName::File.into(),
        SettingsSection::Search => IconName::Search.into(),
        // The asterisk is Claude's own mark — a generic "Harnesses" section wears the honest
        // fallback instead.
        SettingsSection::Harnesses => UbiqIcon::HarnessAny.into(),
        SettingsSection::Isolation => UbiqIcon::Isolation.into(),
        SettingsSection::Assist => IconName::Cpu.into(),
        SettingsSection::Connectors => UbiqIcon::FamilyConnectors.into(),
        SettingsSection::Hosts => UbiqIcon::HostRemote.into(),
        SettingsSection::CommandLine => IconName::SquareTerminal.into(),
    }
}

fn body(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let content = match app.workbench.settings.nav {
        SettingsSection::Appearance => appearance(app, cx),
        SettingsSection::FileExplorer => file_explorer(app, cx),
        SettingsSection::Editor => editor(app, cx),
        SettingsSection::Search => search(app, cx),
        SettingsSection::Harnesses => harnesses(app, cx),
        SettingsSection::Isolation => isolation(app, cx),
        SettingsSection::Assist => assist(app, cx),
        SettingsSection::Connectors => connectors(app, cx),
        SettingsSection::Hosts => hosts_section(app, cx),
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

/// The base sizes the chrome and the conversation families are offered, in points.
///
/// A hand-picked ladder rather than every half point, the same bargain the status bar's content
/// ladder makes: a base size is chosen by eye. Both families' defaults are on it, so a window that
/// has never been touched shows a lit pill rather than nothing.
const BASE_SIZES: &[f32] = &[11.0, 11.5, 12.5, 13.5, 15.0, 17.0];

fn appearance(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let palette = app.workbench.theme_id;
    let scale = theme::text_scale();

    column(vec![
        heading(
            "Appearance",
            "The palette and the accent it is dressed in, how big each surface family's text is, \
             how tight the grid is drawn, and what the window's own chrome shows.",
        ),
        setting_row(
            "Palette",
            "Which family of neutrals the window is built out of. Each family has both grounds; \
             the ground below stays where it is.",
            palette_choice(palette, cx),
        ),
        setting_row(
            "Ground",
            "Dark or light \u{2014} the same flip the titlebar offers, within the family above.",
            ground_choice(palette, cx),
        ),
        setting_row(
            "Accent",
            "The one hue every accent-coloured token derives from. The first swatch is the \
             palette's own; project swatches are identity, not accent, and do not follow this.",
            accent_choice(palette, cx),
        ),
        setting_row(
            "Chrome text size",
            "The base size of the titlebar, the status bar, the rail, tabs, menus, modals, \
             settings and pickers. Growing it reflows the window.",
            size_choice("chrome", scale.chrome, cx, |this, size, cx| {
                this.set_chrome_font_size(size, cx)
            }),
        ),
        setting_row(
            "Conversation text size",
            "The base size of the transcript, the tool blocks, the composer and the agents \
             columns \u{2014} read as prose, at a size that has nothing to do with the size code \
             is read at.",
            size_choice("conversation", scale.conversation, cx, |this, size, cx| {
                this.set_conversation_font_size(size, cx)
            }),
        ),
        setting_row(
            "Content text size",
            "The editor, the viewer, the explorer tree, search results and the terminal panes. \
             This one belongs to the project rather than to the interface, so it is set from the \
             font-size dropdown at the right of the status bar and travels with the project it was \
             chosen for.",
            mono(
                format!("{:.0}\u{2009}px", app.content_font_size_or_default(cx)),
                theme::text_faint(),
            )
            .into_any_element(),
        ),
        setting_row(
            "Density",
            "How tight the grid is drawn \u{2014} chrome rows, the rail, tree indents and the \
             padding inside a pane. Regions you have dragged to a size keep it.",
            density_choice(cx),
        ),
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
        setting_row(
            "Capture the window",
            "The titlebar button and its keystroke that photograph this window into an untitled \
             tab. On Windows and X11 this switch is the only thing standing between the \
             application and a screenshot.",
            check_box(
                "app-settings-capture",
                app.workbench.settings.ui.capture_enabled,
                cx.listener(|this, _, _, cx| this.toggle_capture(cx)),
            )
            .into_any_element(),
        ),
        setting_row(
            "Cache token ring",
            "Show a second ring beside the total-token readout comparing cached tokens to the \
             total.",
            check_box(
                "app-settings-cache-ring",
                app.workbench.settings.ui.show_cache_ring,
                cx.listener(|this, _, _, cx| this.toggle_cache_ring(cx)),
            )
            .into_any_element(),
        ),
    ])
}

/// A row of pills, which is how every choice on this page is drawn.
fn pill_row(children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .children(children)
        .into_any_element()
}

/// The palette families, one pill each, labelled by the member with the ground in use.
///
/// A family is a palette and its counterpart, so listing only the palettes whose ground matches
/// the current one names each family exactly once — and picking one keeps the ground where it is.
fn palette_choice(current: theme::ThemeId, cx: &mut Context<AppState>) -> AnyElement {
    let ground = current.mode();
    pill_row(
        theme::ThemeId::all()
            .filter(|id| id.mode() == ground)
            .map(|id| {
                choice_pill(
                    ElementId::Name(format!("app-settings-palette-{}", id.slug()).into()),
                    id.name(),
                    id == current,
                    cx.listener(move |this, _, _, cx| this.set_palette(id, cx)),
                )
                .into_any_element()
            })
            .collect(),
    )
}

/// The two grounds. Picking the one already lit is not a flip, so it does nothing.
fn ground_choice(current: theme::ThemeId, cx: &mut Context<AppState>) -> AnyElement {
    let counterpart = current.counterpart();
    let dark = current.mode() == theme::Mode::Dark;
    let pill = |id: &'static str, label: &'static str, active: bool| {
        choice_pill(
            id,
            label,
            active,
            cx.listener(move |this, _, _, cx| {
                if !active {
                    this.set_palette(counterpart, cx);
                }
            }),
        )
        .into_any_element()
    };

    pill_row(vec![
        pill("app-settings-ground-dark", "Dark", dark),
        pill("app-settings-ground-light", "Light", !dark),
    ])
}

/// The accents, as swatches: the palette's own first, then every accent the build ships.
///
/// A swatch rather than a pill because the choice *is* the colour, and the name is on its hover.
/// Not a project swatch — `D19` keeps those out of this axis.
fn accent_choice(palette: theme::ThemeId, cx: &mut Context<AppState>) -> AnyElement {
    let current = theme::accent_id();
    let own = theme::palette_for(palette).palette.accent.primary;

    let mut swatches = vec![accent_swatch(
        "app-settings-accent-default",
        "The palette's own",
        own,
        current.is_none(),
        None,
        cx,
    )];
    swatches.extend(theme::AccentId::all().map(|id| {
        accent_swatch(
            ElementId::Name(format!("app-settings-accent-{}", id.slug()).into()),
            id.name(),
            id.seed(),
            current == Some(id),
            Some(id),
            cx,
        )
    }));

    pill_row(swatches)
}

fn accent_swatch(
    id: impl Into<ElementId>,
    name: &'static str,
    colour: gpui::Rgba,
    active: bool,
    accent: Option<theme::AccentId>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let label: SharedString = name.into();
    div()
        .id(id)
        .size(px(22.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .bg(colour)
        .border_1()
        .border_color(if active {
            theme::text()
        } else {
            theme::border()
        })
        .when(active, |this| {
            this.child(div().size(px(6.)).bg(theme::text()))
        })
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(label.clone()).build(window, cx)
        })
        .on_click(cx.listener(move |this, _, _, cx| this.set_accent(accent, cx)))
        .into_any_element()
}

/// One family's base size, as the ladder in [`BASE_SIZES`]. The nearest half point counts as the
/// entry, so a size written by another build still lights a pill.
fn size_choice(
    family: &'static str,
    current: f32,
    cx: &mut Context<AppState>,
    set: impl Fn(&mut AppState, f32, &mut Context<AppState>) + Copy + 'static,
) -> AnyElement {
    pill_row(
        BASE_SIZES
            .iter()
            .copied()
            .map(|size| {
                choice_pill(
                    ElementId::Name(format!("app-settings-{family}-size-{size}").into()),
                    format!("{size}"),
                    (current - size).abs() < 0.25,
                    cx.listener(move |this, _, _, cx| set(this, size, cx)),
                )
                .into_any_element()
            })
            .collect(),
    )
}

/// The three densities, one lit.
fn density_choice(cx: &mut Context<AppState>) -> AnyElement {
    let current = theme::density();
    pill_row(
        theme::Density::ALL
            .into_iter()
            .map(|density| {
                choice_pill(
                    ElementId::Name(format!("app-settings-density-{}", density.name()).into()),
                    density.name(),
                    density == current,
                    cx.listener(move |this, _, _, cx| this.set_density(density, cx)),
                )
                .into_any_element()
            })
            .collect(),
    )
}

fn file_explorer(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let on = app.workbench.settings.ui.explorer_preview;
    let host = &app.workbench.settings.host;
    column(vec![
        heading(
            "File explorer",
            "How a click on a file in the tree opens it, and where projects Ubiq fetches land.",
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
        folder_row(
            "Default project folder",
            "Where a clone lands unless the clone modal is pointed somewhere else.",
            "app-settings-projects-root",
            host.projects_root.as_deref(),
            false,
            cx,
        ),
        folder_row(
            "Ephemeral folder",
            "Where a throwaway clone lands \u{2014} and the only tree Ubiq will delete a project \
             folder from.",
            "app-settings-ephemeral-root",
            host.ephemeral_root.as_deref(),
            true,
            cx,
        ),
    ])
}

/// One host-owned folder, and the platform's own chooser for it.
///
/// **An empty value is drawn as a placeholder, never as a path.** The host resolves its own
/// default and the contract does not name one, so the interface saying where that is would be a
/// guess the user could not correct.
fn folder_row(
    label: &str,
    note: &str,
    id: &'static str,
    value: Option<&str>,
    ephemeral: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let (text, colour) = match value.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => (path.to_string(), theme::text_muted()),
        None => ("The host's own default".to_string(), theme::text_faint()),
    };

    setting_row(
        label,
        note,
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                elided(
                    ElementId::Name(format!("{id}-value").into()),
                    text,
                    colour,
                    theme::font(theme::Family::Chrome, theme::Role::Body),
                )
                .max_w(px(220.)),
            )
            .child(ghost_button(
                ElementId::Name(format!("{id}-choose").into()),
                None,
                "Choose\u{2026}",
                cx.listener(move |this, _, _, cx| this.choose_clone_root(ephemeral, cx)),
            ))
            .child(ghost_button(
                ElementId::Name(format!("{id}-clear").into()),
                None,
                "Clear",
                cx.listener(move |this, _, _, cx| {
                    this.set_clone_root(String::new(), ephemeral, cx)
                }),
            ))
            .into_any_element(),
    )
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

/// What every project search does: how much is indexed, what is skipped, and what it falls back
/// to. The two lists are comma-separated lines that commit on Enter and on blur — see the
/// subscriptions in `app.rs`, and `sync_settings_fields` for what fills them.
fn search(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
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
            "Keep an index",
            "What Ubiq remembers about a project so a search need not re-read it. Off walks every \
             file on every query. Full text reads only the files that could match. Adding symbols \
             also records what each file defines, which costs a parse of every file and is what \
             jumping to a definition needs. A project can override this in its own settings.",
            index_level_choice(app.workbench.settings.host.index_level, cx),
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

/// Whether Ubiq may write a line of prose for the user, and which backend writes it.
///
/// **Every sentence about the backend comes from the host.** Whether assistance can run, which
/// model is running and why it cannot are the host's answers — asked with `GetAssist` and never
/// inferred here — so this section names no platform and no vendor of its own. What it authors is
/// only what Ubiq does with the answer.
fn assist(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let state = app.workbench.settings.assist.clone();
    // Nothing has answered yet reads as "checking", never as a refusal: a window that has just
    // opened knows nothing about this machine, which is not the same as being told no.
    let chip = match &state {
        None => state_chip(
            "Checking what this host can do\u{2026}",
            theme::text_faint(),
            1.0,
        )
        .into_any_element(),
        Some(info) if info.available => {
            state_chip(assist_available(info), theme::success(), 1.0).into_any_element()
        }
        Some(_) => {
            state_chip("Assistance is unavailable here", theme::warning(), 1.0).into_any_element()
        }
    };

    // The on-device switch is offered only where that backend could actually answer. The setting
    // being off is the one negative answer the user can undo from here, so it keeps its controls
    // live — and a configured API provider is never gated by it, since whether one can answer is
    // not a question about this machine's own model.
    let switchable = state.as_ref().is_none_or(AssistInfo::switchable);

    let mut rows = vec![heading(
        "Assistance",
        "Ubiq writing a short line of prose it would otherwise invent mechanically \u{2014} a \
         commit message from what is staged. Off by default, and never more than a suggestion: it \
         fills an editable field, and it renames nothing.",
    )];

    // What the host last refused about a provider — a key it could not file, a list it could not
    // fetch. The same banner the harnesses and connectors sections read.
    if let Some(error) = app.workbench.settings.error.clone() {
        rows.push(error_banner(&error, cx));
    }

    rows.push(div().flex().flex_none().child(chip).into_any_element());
    rows.push(setting_row(
        "Suggestions",
        "Which backend writes one. Off calls no model at all, which is what a build with this \
         untouched does; a provider below writes one over its own API.",
        assist_provider_choice(app, switchable, cx),
    ));

    // A naming runs through the provider above, so it is dimmed and inert while that is off — the
    // same shape confinement takes where the platform cannot run it. The row that would make it
    // run is the one directly above, so nothing is trapped by this.
    let namable = !matches!(app.workbench.settings.host.assist, AssistProvider::Off);
    let mut naming = check_box(
        "app-settings-auto-name",
        app.workbench.settings.host.auto_name_conversations,
        cx.listener(move |this, _, _, cx| {
            if namable {
                this.toggle_auto_name_conversations(cx);
            }
        }),
    );
    if !namable {
        naming = naming.opacity(0.5);
    }
    rows.push(setting_row(
        "Name conversations",
        "Ubiq reads the opening exchange of a conversation and writes a title for its tab, with a \
         five-word summary on hover. Runs once per conversation, through the provider above, so \
         it does nothing while that is off.",
        naming.into_any_element(),
    ));

    // The host's own sentence about why, kept on screen rather than hidden: the harnesses section
    // makes the same choice for a harness that is not installed.
    if let Some(detail) = state.as_ref().and_then(|info| info.detail.clone()) {
        rows.push(note(&detail, theme::text_muted()));
    }

    rows.push(ai_providers(app, cx));

    column(rows)
}

/// What an available backend's chip says: the name the host gave it, and how much it can hold.
fn assist_available(info: &AssistInfo) -> String {
    match &info.limits {
        Some(limits) => format!(
            "{} \u{b7} {} tokens of context",
            limits.label, limits.context_tokens
        ),
        None => "Assistance is available on this host".to_string(),
    }
}

/// Every backend on offer, one lit: off, this platform's own model, and one pill per configured
/// API provider. The closed-choice shape [`index_level_choice`] draws.
///
/// **Only the on-device pill is gated by `switchable`.** Whether a local model can run is the
/// host's answer about that one backend, so a machine without one still offers Off and every
/// configured provider — a provider's availability is whether it is configured, which is a fact
/// this half holds. Without that split, a host with no local backend would dim the whole row and
/// leave a user unable to switch away from it.
fn assist_provider_choice(
    app: &AppState,
    switchable: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let current = app.workbench.settings.host.assist.clone();
    let mut pills = vec![
        assist_pill(
            "app-settings-assist-off".into(),
            "Off".into(),
            &current,
            AssistProvider::Off,
            true,
            cx,
        ),
        assist_pill(
            "app-settings-assist-on-device".into(),
            "On-device".into(),
            &current,
            AssistProvider::OnDevice,
            switchable,
            cx,
        ),
    ];
    for info in &app.workbench.settings.ai_providers {
        let provider_id = info.provider.id;
        pills.push(assist_pill(
            ElementId::Name(format!("app-settings-assist-{provider_id}").into()),
            info.provider.name.clone().into(),
            &current,
            AssistProvider::Api { provider_id },
            true,
            cx,
        ));
    }

    div()
        .flex()
        .flex_none()
        .items_center()
        .flex_wrap()
        .gap_1()
        .children(pills)
        .into_any_element()
}

/// One backend pill. `live: false` is the dead treatment every ruled-out control in this panel
/// gets: half opacity, and a listener that does nothing rather than one that lies.
fn assist_pill(
    id: ElementId,
    label: SharedString,
    current: &AssistProvider,
    provider: AssistProvider,
    live: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let pill = choice_pill(
        id,
        label,
        *current == provider,
        cx.listener(move |this, _, _, cx| {
            if live {
                this.set_assist_provider(provider.clone(), cx);
            }
        }),
    );

    if live {
        pill.into_any_element()
    } else {
        pill.opacity(0.5).into_any_element()
    }
}

/// The configured API providers, one row each, with a way to add the next one.
///
/// Drawn whether or not there are any, for the reason [`oauth_apps`] is: a section that vanishes
/// when the list is empty is a section with no way to add the first row, which is exactly the
/// state every user arrives in.
fn ai_providers(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let providers = app.workbench.settings.ai_providers.clone();
    let mut section = div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_4()
        .child(section_label("API providers"))
        .child(modal_note(
            "A model behind somebody else\u{2019}s API, reached with a key this machine keeps in \
             its own credential store \u{2014} never in the settings file, and never shown again \
             once it is filed. Several are ordinary: a local endpoint and a hosted one are two \
             providers, not a conflict.",
        ))
        .child(div().flex().items_center().gap_3().child(primary_button(
            "app-settings-ai-add",
            Some(IconName::Plus),
            "Add provider\u{2026}",
            cx.listener(|this, _, window, cx| this.open_ai_form(None, window, cx)),
        )));

    if providers.is_empty() {
        return section
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Meta))
                    .text_color(theme::text_faint())
                    .child(SharedString::from(
                        "No providers. Assistance runs on this platform\u{2019}s own model, or on \
                         nothing at all.",
                    )),
            )
            .into_any_element();
    }

    section = section.children(providers.iter().map(|info| ai_provider_row(info, cx)));
    section.into_any_element()
}

/// One provider: what it is, which models it names, whether a key is filed, and the three things
/// that can be done to it. Built on [`oauth_row`]'s shape, which answers the same kind of
/// question — a record the host holds, and a credential it only says the presence of.
fn ai_provider_row(info: &AiProviderInfo, cx: &mut Context<AppState>) -> AnyElement {
    let id = info.provider.id;
    // A provider with no model is unfinished rather than broken: it is what every provider looks
    // like for the moment between being saved and having one picked, so the row says which half
    // is missing and where the fix is.
    let unset = info.provider.fast_model.trim().is_empty();
    let models = match (&info.provider.smart_model, unset) {
        (_, true) => "no model chosen \u{b7} Edit picks one".to_string(),
        (Some(smart), false) => format!("{} \u{b7} {smart}", info.provider.fast_model),
        (None, false) => info.provider.fast_model.clone(),
    };
    // The contract names no URL for a kind's own endpoint — the host resolves it — so neither
    // does this row.
    let where_it_is = info
        .provider
        .base_url
        .clone()
        .unwrap_or_else(|| "the kind\u{2019}s own endpoint".to_string());
    let (chip, colour) = if info.has_key {
        ("key filed", theme::success())
    } else {
        ("no key", theme::text_faint())
    };

    setting_row(
        &format!(
            "{} \u{b7} {}",
            info.provider.name,
            info.provider.kind.label()
        ),
        &format!("{models} \u{b7} {where_it_is}"),
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(badge(chip, colour))
            // Warning rather than danger: nothing is wrong, something is not finished.
            .children(unset.then(|| badge("no model", theme::warning())))
            .child(ghost_button(
                ElementId::Name(format!("app-settings-ai-{id}-test").into()),
                None,
                "Test",
                cx.listener(move |this, _, _, cx| this.open_ai_test(id, cx)),
            ))
            .child(ghost_button(
                ElementId::Name(format!("app-settings-ai-{id}-edit").into()),
                None,
                "Edit",
                cx.listener(move |this, _, window, cx| this.open_ai_form(Some(id), window, cx)),
            ))
            .child(ghost_button(
                ElementId::Name(format!("app-settings-ai-{id}-remove").into()),
                None,
                "Remove",
                cx.listener(move |this, _, _, cx| this.open_remove_ai_provider(id, cx)),
            ))
            .into_any_element(),
    )
}

/// The three indexing levels, one lit.
///
/// Named for what they cost the user rather than for what they are: "Full text" and "Full text +
/// symbols" say what is kept, where `light` and `full` would only say which is bigger.
fn index_level_choice(current: IndexLevel, cx: &mut Context<AppState>) -> AnyElement {
    let pill = |id: &'static str, label: &'static str, level: IndexLevel| {
        choice_pill(
            id,
            label,
            current == level,
            cx.listener(move |this, _, _, cx| this.set_index_level(level, cx)),
        )
    };

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(pill("app-settings-index-none", "Off", IndexLevel::None))
        .child(pill(
            "app-settings-index-light",
            "Full text",
            IndexLevel::Light,
        ))
        .child(pill(
            "app-settings-index-full",
            "Full text + symbols",
            IndexLevel::Full,
        ))
        .into_any_element()
}

/// Confinement, the home a confined agent runs with, and the directories it may reach past its
/// policy. Every value here is host-owned, so each control writes the Host layer.
///
/// **Confinement is a macOS feature.** `confined_launch` in the harness library errors on every
/// other target, so this says so plainly and disables the toggle rather than offering a switch
/// that does nothing.
fn isolation(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let host = &app.workbench.settings.host;
    let supported = cfg!(target_os = "macos");

    let mut toggle = check_box(
        "app-settings-isolate-agents",
        host.isolate_agents,
        cx.listener(move |this, _, _, cx| {
            if supported {
                this.toggle_isolate_agents(cx);
            }
        }),
    );
    if !supported {
        toggle = toggle.opacity(0.5);
    }

    column(vec![
        heading(
            "Isolation",
            "What an agent may reach, and whose home it runs in. These are the host's own \
             settings, so they apply to every agent this host starts.",
        ),
        div()
            .flex()
            .flex_none()
            .child(match supported {
                true => state_chip(
                    "Confinement available on this machine",
                    theme::success(),
                    1.0,
                ),
                false => state_chip(
                    "Confinement is macOS only \u{2014} agents here run unconfined",
                    theme::warning(),
                    1.0,
                ),
            })
            .into_any_element(),
        setting_row(
            "Confine agents to their project",
            "An agent reads and writes only its project's folder, plus a throwaway configuration \
             of its own. Off, an agent can reach anywhere on the machine.",
            toggle.into_any_element(),
        ),
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(label_block(
                "The agent's home",
                "Which $HOME a confined agent runs with.",
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(home_cards(host, cx)),
            )
            .when(matches!(host.agent_home, AgentHome::Named(_)), |this| {
                this.child(
                    field(theme::border(), false)
                        .h(px(30.))
                        .w(px(300.))
                        .px_2()
                        .child(Input::new(&app.agent_home_input).appearance(false)),
                )
            })
            .into_any_element(),
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(label_block(
                "Extra grants",
                "Directories an agent may reach beyond its policy \u{2014} a toolchain installed \
                 somewhere unusual, a shared cache. Read-only unless you say otherwise.",
            ))
            .child(grant_chips(app, cx))
            .into_any_element(),
    ])
}

/// The three homes, one lit. The same card shape the kitchen sink draws the permission modes in.
fn home_cards(
    host: &ubiq_proto::settings::HostSettings,
    cx: &mut Context<AppState>,
) -> Vec<AnyElement> {
    const HOMES: [(&str, &str, &str); 3] = [
        (
            "inherit",
            "Your home",
            "Recommended. The agent reads and writes only what its policy grants, inside your own \
             home, so your toolchain works.",
        ),
        (
            "ephemeral",
            "A fresh one each run",
            "Nothing an agent leaves behind survives. Nothing it needs is there either: cargo, \
             npm and dotnet will not find their caches.",
        ),
        (
            "named",
            "A named home",
            "Kept under Ubiq's own state, so a second run finds what the first left. You \
             populate it yourself.",
        ),
    ];

    HOMES
        .iter()
        .map(|(key, label, note)| {
            let home = match *key {
                "ephemeral" => AgentHome::Ephemeral,
                "named" => AgentHome::Named(String::new()),
                _ => AgentHome::Inherit,
            };
            let selected =
                std::mem::discriminant(&host.agent_home) == std::mem::discriminant(&home);

            card(
                ElementId::Name(format!("app-settings-home-{key}").into()),
                if selected {
                    theme::accent()
                } else {
                    theme::border()
                },
                selected,
            )
            .px_3()
            .py_2()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(status_dot(
                        if selected {
                            theme::accent()
                        } else {
                            theme::text_faint()
                        },
                        if selected {
                            theme::accent_soft()
                        } else {
                            theme::surface()
                        },
                    ))
                    .child(label_block(label, note)),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.set_agent_home(home.clone(), cx)))
            .into_any_element()
        })
        .collect()
}

/// One removable chip per grant, then the field a path is typed into. Clicking a chip flips it
/// between read-only and read-write; the `\u{d7}` drops it.
fn grant_chips(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let chips: Vec<AnyElement> = app
        .workbench
        .settings
        .host
        .extra_grants
        .iter()
        .enumerate()
        .map(|(index, grant)| {
            let (word, colour, fill) = match grant.write {
                true => ("rw", theme::warning(), theme::warning_soft()),
                false => ("ro", theme::text_muted(), theme::surface()),
            };
            removable_tag(
                ElementId::Name(format!("app-settings-grant-{index}").into()),
                ElementId::Name(format!("app-settings-grant-drop-{index}").into()),
                format!("{} \u{b7} {word}", grant.path),
                format!(
                    "{} \u{2014} {}. Click to make it {}.",
                    grant.path,
                    if grant.write {
                        "read-write"
                    } else {
                        "read-only"
                    },
                    if grant.write {
                        "read-only"
                    } else {
                        "read-write"
                    },
                ),
                fill,
                colour,
                colour,
                cx.listener(move |this, _, _, cx| this.toggle_grant_write(index, cx)),
                cx.listener(move |this, _, _, cx| this.remove_extra_grant(index, cx)),
            )
            .into_any_element()
        })
        .collect();

    div()
        .flex()
        .items_center()
        .gap_2()
        .flex_wrap()
        .children(chips)
        .child(
            field(theme::border(), false)
                .h(px(26.))
                .w(px(200.))
                .px_2()
                .child(Input::new(&app.grant_path_input).appearance(false)),
        )
        .child(icon_button(
            "app-settings-grant-add",
            IconName::Plus,
            false,
            cx.listener(|this, _, window, cx| this.add_extra_grant(window, cx)),
        ))
        .into_any_element()
}

fn harnesses(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let mut rows = vec![heading(
        "Harnesses",
        "Every agent runs on a harness. Register as many as you like — the same tool twice \
         with different credentials is normal, and each entry carries its own defaults.",
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
                "app-settings-add-harness",
                Some(IconName::Plus),
                "Add harness",
                cx.listener(|this, _, window, cx| this.open_harness_login(window, cx)),
            ))
            .child(ghost_button(
                "app-settings-add-profile",
                Some(IconName::Plus),
                "Add profile",
                cx.listener(|this, _, window, cx| this.open_profile_form(None, window, cx)),
            ))
            .into_any_element(),
    );
    rows.push(accounts(app, cx));
    rows.push(profiles(app, cx));
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
                .text_size(theme::font(Family::Chrome, Role::Meta))
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
        .text_size(theme::font(Family::Chrome, Role::Meta))
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
        .border_l(px(theme::accent_edge()))
        .border_color(theme::warning())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Label))
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
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .text_color(theme::text_muted())
                    .child(SharedString::from("No harnesses registered.")),
            )
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Meta))
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

/// The saved setups, one row each: what it is called, and what it starts.
///
/// Empty draws nothing at all — the `Add profile` button above already says the list can grow,
/// and a second empty-state beside the accounts' one would be two notices about one section.
fn profiles(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    if app.workbench.settings.profiles.is_empty() {
        return div().into_any_element();
    }
    let profiles = app.workbench.settings.profiles.clone();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(section_label("Profiles"))
        .children(profiles.iter().map(|profile| profile_row(app, profile, cx)))
        .into_any_element()
}

/// One saved setup: `reviewer — Codex · gpt-5 · plan`, and the way back into its form.
///
/// Every field after the harness is optional and an empty one is left out rather than drawn as an
/// empty pill. A profile naming a harness this machine does not have reads faint, the same way a
/// harness row that is not installed does.
fn profile_row(app: &AppState, profile: &ProfileInfo, cx: &mut Context<AppState>) -> AnyElement {
    let available = app
        .workbench
        .agent_types
        .iter()
        .any(|info| info.id == profile.agent_type && info.available);

    let mut parts = vec![harness_label(app, &profile.agent_type).to_string()];
    parts.extend(profile.account.clone().filter(|it| !it.is_empty()));
    parts.extend(profile.model.clone().filter(|it| !it.is_empty()));
    parts.extend(
        profile
            .mode
            .as_deref()
            .filter(|it| !it.is_empty())
            .map(|mode| mode_label(app, &profile.agent_type, mode).to_string()),
    );

    let edit = profile.clone();
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
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(if available {
                            theme::text()
                        } else {
                            theme::text_faint()
                        })
                        .child(SharedString::from(profile.id.clone())),
                )
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                        .text_color(theme::text_muted())
                        .child(SharedString::from(format!(
                            "\u{2014} {}",
                            parts.join(" \u{b7} ")
                        ))),
                ),
        )
        .child(ghost_button(
            ElementId::Name(format!("app-settings-profile-{}-edit", profile.id).into()),
            None,
            "Edit",
            cx.listener(move |this, _, window, cx| {
                this.open_profile_form(Some(edit.clone()), window, cx)
            }),
        ))
        .into_any_element()
}

/// A mode's display name, through the harness's own list — falling back to the raw value when
/// the harness is gone or no longer offers it.
fn mode_label<'a>(app: &'a AppState, agent_type: &str, mode: &'a str) -> &'a str {
    app.workbench
        .agent_types
        .iter()
        .find(|info| info.id == agent_type)
        .and_then(|info| info.modes.iter().find(|choice| choice.value == mode))
        .map(|choice| choice.name.as_str())
        .unwrap_or(mode)
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
                    .text_size(theme::font(Family::Chrome, Role::Body))
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
                .text_size(theme::font(Family::Chrome, Role::Meta))
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
            .text_size(theme::font(Family::Chrome, Role::Meta))
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
                        .text_size(theme::font(Family::Chrome, Role::Body))
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
/// The profile form: a name, and every question a start answers.
///
/// It is [`crate::ui::new_agent::body`] with a name field above it — a profile is a saved answer
/// to the same questions, so it asks them with the same rows rather than with a second set that
/// would drift. What it drops is what a saved setup does not have: the profile half of the target
/// picker, and the Start button.
pub fn profile_form(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(form) = app.workbench.settings.profile_form.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let named = !app.profile_id_input.read(cx).value().trim().is_empty();
    let ready = named && !form.agent_type.is_empty();

    let body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(crate::ui::kit::label_hint(
                    "app-settings-profile-name-hint",
                    "Name",
                    "What to call this setup. Saving over an existing name replaces it.",
                ))
                .child(
                    field(
                        theme::border(),
                        app.profile_id_input
                            .read(cx)
                            .focus_handle(cx)
                            .is_focused(window),
                    )
                    .h(px(30.))
                    .px_2()
                    .child(Input::new(&app.profile_id_input).appearance(false)),
                ),
        )
        .child(crate::ui::new_agent::body(app, window, cx))
        .into_any_element();

    // The same action row the New agent modal draws: what is not built yet on the left, what the
    // form is for on the right.
    let footer = crate::ui::new_agent::footer_row(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(ghost_button(
                "app-settings-profile-cancel",
                None,
                "Cancel",
                cx.listener(|this, _, _, cx| this.close_profile_form(cx)),
            ))
            .child(
                primary_button(
                    "app-settings-profile-save",
                    None,
                    "Save",
                    cx.listener(|this, _, _, cx| this.save_new_agent_profile(cx)),
                )
                .when(!ready, |button| button.opacity(0.5)),
            )
            .into_any_element(),
    );

    crate::ui::new_agent::confirmable(
        div().child(modal(
            "app-settings-profile",
            theme::accent(),
            "Profile",
            body,
            footer,
            crate::ui::handler(&view, |this, _, cx| this.close_profile_form(cx)),
            window,
        )),
        cx,
    )
    .into_any_element()
}

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
                // Not "Abort": a harness whose login is its ordinary screen (grok) never ends
                // by itself, so this button is how a finished sign-in gets recorded. It stops
                // the harness and the modal stays up to say what was captured — the X beside
                // the title is still the way out that reports nothing.
                .child(ghost_button(
                    "app-settings-login-abort",
                    None,
                    if login.probe { "Abort" } else { "Done" },
                    cx.listener(|this, _, _, cx| this.finish_harness_login(cx)),
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
                        // Every harness, installed or not: one that is missing is exactly the
                        // case a custom command fixes, so it reads faint — the way a profile
                        // naming an absent harness does — and stays pickable.
                        .children(app.workbench.agent_types.iter().map(|agent_type| {
                            let id = agent_type.id.clone();
                            choice_pill(
                                ElementId::Name(
                                    format!("app-settings-login-harness-{}", agent_type.id).into(),
                                ),
                                &agent_type.label,
                                chosen == Some(agent_type.id.as_str()),
                                cx.listener(move |this, _, window, cx| {
                                    this.pick_login_harness(id.clone(), window, cx)
                                }),
                            )
                            .when(!agent_type.available, |pill| pill.opacity(0.55))
                        })),
                ),
        )
        .when(chosen.is_some(), |body| {
            body.child(login_command(app, window, cx))
        })
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

/// The custom command for the picked harness: a ghost button that opens a field, the field
/// itself, and the one line the host answered when `Check` was pressed.
///
/// Empty means "whatever the library would run", which is what the placeholder says. Closing the
/// field is what saves it, so an emptied field closed again is how an override is removed.
fn login_command(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let login = app.workbench.settings.login.as_ref();
    let open = login.is_some_and(|it| it.command_open);
    let focused = app
        .login_command_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().child(ghost_button(
            "app-settings-login-command-toggle",
            None,
            if open {
                "Hide command"
            } else {
                "Custom command"
            },
            cx.listener(|this, _, _, cx| this.toggle_login_command(cx)),
        )))
        .when(open, |body| {
            body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        field(theme::border(), focused)
                            .flex_1()
                            .h(px(30.))
                            .px_2()
                            .child(Input::new(&app.login_command_input).appearance(false)),
                    )
                    .child(ghost_button(
                        "app-settings-login-command-check",
                        None,
                        "Check",
                        cx.listener(|this, _, _, cx| this.check_login_command(cx)),
                    )),
            )
            .children(
                login
                    .and_then(|it| it.command_check.as_ref())
                    .map(|(ok, detail)| {
                        note(
                            detail,
                            if *ok {
                                theme::success()
                            } else {
                                theme::danger()
                            },
                        )
                    }),
            )
        })
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
            // `flex()` is not decoration: `terminal::pane` returns a `flex_1` column, and under
            // the dock its parent is a flex container so it fills the panel. A plain block
            // wrapper gives it no flex context, so it hugs its content and the emulator draws as
            // a one-line black band across the top of an otherwise empty box.
            div()
                .flex()
                .flex_col()
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
                .border_l(px(theme::accent_edge()))
                .border_color(theme::accent())
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(elided(
                    ElementId::Name(format!("app-settings-login-link-{index}-text").into()),
                    url,
                    theme::accent(),
                    theme::font(theme::Family::Chrome, theme::Role::Label),
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
            .child(ghost_button(
                "app-settings-app-registration",
                Some(IconName::Plus),
                "App registration\u{2026}",
                cx.listener(|this, _, window, cx| this.open_app_form(None, window, cx)),
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
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text_muted())
                        .child(SharedString::from("No connections.")),
                )
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Meta))
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

/// `Local`, every attached remote, and every saved host with no live connection — one dropdown to
/// set which host a message naming neither a pane nor a project reaches, and a list below it to
/// manage what is remembered.
fn hosts_section(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let remotes = app.remote_hosts();
    let saved = app.workbench.settings.host.remote_hosts.clone();
    let rows = host_menu_rows(&remotes, &saved, &app.workbench.settings.failed_hosts);
    let items: Vec<String> = rows
        .iter()
        .map(|(entry, status)| host_row_label(entry, *status))
        .collect();
    let active = app.active_host();
    let selected = rows
        .iter()
        .position(|(entry, _)| match (entry, active) {
            (HostEntry::Local, HostRef::Local) => true,
            (HostEntry::Remote { host, .. }, HostRef::Remote(active)) => *host == active,
            _ => false,
        })
        .unwrap_or(0);
    let open = app.workbench.settings.host_picker_open;

    column(vec![
        heading(
            "Hosts",
            "The local machine, always attached, plus any host reached over \u{201c}Connect to a \
             remote host\u{201d} in the titlebar. Picking one here only changes where a new pane \
             or a new project lands by default \u{2014} it never moves one that already exists.",
        ),
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(label_block("Active host", ""))
            .child(
                Picker::new(
                    "app-settings-host-picker",
                    items.get(selected).cloned().unwrap_or_default(),
                )
                .items(items)
                .selected(selected)
                .open(open)
                .on_toggle(crate::ui::handler(&view, |this, _, cx| {
                    this.toggle_host_picker(cx)
                }))
                .on_pick({
                    let view = view.clone();
                    let rows = rows.clone();
                    move |index, window, cx| {
                        let Some((entry, _)) = rows.get(index).cloned() else {
                            return;
                        };
                        view.update(cx, |this, cx| this.pick_host_menu_entry(entry, window, cx));
                    }
                })
                .on_dismiss(crate::ui::handler(&view, |this, _, cx| {
                    this.toggle_host_picker(cx)
                })),
            )
            .into_any_element(),
        div()
            .flex()
            .flex_col()
            .gap_2()
            .pt_4()
            .child(section_label("Saved hosts"))
            .children(if saved.is_empty() {
                Some(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                        .text_color(theme::text_faint())
                        .child(SharedString::from(
                            "None yet. Connecting to a host from the titlebar saves it here.",
                        ))
                        .into_any_element(),
                )
            } else {
                None
            })
            .children(saved.iter().map(|host| {
                // A live connection is labelled with the saved name when it was reconnected from
                // here and with the address when it was dialled fresh, so a row matches on
                // either — see `AppState::address_of_host`, which reads the same pairing back the
                // other way round.
                let attached = remotes
                    .iter()
                    .find(|(_, label)| *label == host.name || *label == host.address)
                    .map(|(id, _)| *id);
                host_row(host, attached, &app.workbench.settings.failed_hosts, cx)
            }))
            .into_any_element(),
    ])
}

/// One saved host: its address, whether it is attached or last failed to reach, and a way to let
/// it go. The token it was dialled with is never shown here, because it was never kept — see
/// [`ubiq_proto::settings::HostSettings::remote_hosts`].
///
/// `attached` names the live connection to this host, when there is one. Disconnect closes every
/// pane it was running and drops the socket, but leaves the saved record alone — Forget is the
/// other way round, and a host can be either without being the other.
fn host_row(
    host: &ubiq_proto::settings::SavedRemoteHost,
    attached: Option<HostId>,
    failed: &std::collections::HashSet<String>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let address = host.address.clone();
    let (chip, colour) = if attached.is_some() {
        ("attached", theme::text_faint())
    } else if failed.contains(&host.address) {
        ("last attempt failed", theme::danger())
    } else {
        ("saved", theme::text_faint())
    };

    setting_row(
        &host.name,
        &host.address,
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(badge(chip, colour))
            .children(attached.map(|id| {
                ghost_button(
                    ElementId::Name(format!("app-settings-host-{address}-disconnect").into()),
                    None,
                    "Disconnect",
                    cx.listener(move |this, _, _, cx| {
                        this.disconnect_host(id, cx);
                    }),
                )
            }))
            .child(ghost_button(
                ElementId::Name(format!("app-settings-host-{address}-forget").into()),
                None,
                "Forget",
                cx.listener(move |this, _, _, cx| this.forget_remote_host(address.clone(), cx)),
            ))
            .into_any_element(),
    )
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
                    .text_size(theme::font(Family::Chrome, Role::Meta))
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
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text())
                        .child(SharedString::from(label.clone())),
                )
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                        .text_color(theme::text_muted())
                        .child(SharedString::from(connection.account.clone())),
                )
                // A self-hosted base URL can be long enough to push everything else off the
                // panel, so it is elided with the whole of it as the tooltip.
                .child(elided(
                    ElementId::Name(format!("app-settings-connection-{id}-instance").into()),
                    where_it_lives,
                    theme::text_faint(),
                    theme::font(theme::Family::Chrome, theme::Role::Meta),
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
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text())
                .child(SharedString::from(cert.origin.clone())),
        )
        .child(mono(short, theme::text_muted()).text_size(theme::font(Family::Chrome, Role::Meta)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Meta))
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

/// The application registrations Ubiq authenticates *as*, where one was registered rather than
/// built in.
///
/// Drawn whether or not there are any: a section that vanishes when the list is empty is a section
/// with no way to add the first row, which is exactly the state a user arrives in. The client id is
/// public and rides the settings blob; only the secret is material, which is why the row says
/// whether one is set rather than showing anything.
fn oauth_apps(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let apps = app.workbench.settings.host.oauth_apps.clone();
    let mut section = div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_4()
        .child(section_label("App registrations"))
        .child(modal_note(&format!(
            "An application registered at the provider, which Ubiq then signs in through. Register \
             it with {OAUTH_REDIRECT} as its callback \u{2014} that is the address this machine \
             listens on, and a browser flow fails at the provider without it."
        )));

    if apps.is_empty() {
        return section
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Meta))
                    .text_color(theme::text_faint())
                    .child(SharedString::from(
                        "No registrations. Connections use whatever application this build ships, \
                         where it ships one.",
                    )),
            )
            .into_any_element();
    }

    section = section.children(apps.iter().map(|entry| oauth_row(entry, cx)));
    section.into_any_element()
}

fn oauth_row(entry: &OauthApp, cx: &mut Context<AppState>) -> AnyElement {
    let where_it_is = entry.origin.clone().unwrap_or_else(|| "cloud".to_string());
    let id = entry.id;
    let name = entry.name.clone();
    let (chip, colour) = if entry.has_secret {
        ("secret set", theme::success())
    } else {
        ("no secret", theme::text_faint())
    };

    setting_row(
        &format!("{} \u{b7} {}", entry.name, entry.provider.label()),
        &format!("{where_it_is} \u{b7} {}", entry.client_id),
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(badge(chip, colour))
            .child(ghost_button(
                ElementId::Name(format!("app-settings-oauth-{id}-edit").into()),
                None,
                "Edit",
                cx.listener(move |this, _, window, cx| this.open_app_form(Some(id), window, cx)),
            ))
            .child(ghost_button(
                ElementId::Name(format!("app-settings-oauth-{id}-delete").into()),
                None,
                "Delete",
                cx.listener(move |this, _, _, cx| this.open_delete_app(id, name.clone(), cx)),
            ))
            .into_any_element(),
    )
}

/// One read-only value with a copy button — the redirect URL, which the user has to paste into the
/// provider's own form and cannot guess.
fn copyable(id: &'static str, value: &str, cx: &mut Context<AppState>) -> AnyElement {
    let copy = value.to_string();
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h(px(30.))
                .px_2()
                .flex()
                .items_center()
                .bg(theme::surface())
                .border_1()
                .border_color(theme::border())
                .child(
                    mono(value.to_string(), theme::text())
                        .text_size(theme::font(Family::Chrome, Role::Meta)),
                ),
        )
        .child(icon_button(
            ElementId::Name(format!("{id}-copy").into()),
            IconName::Copy,
            false,
            cx.listener(move |_, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
            }),
        ))
        .into_any_element()
}

/// The application-registration form: name it, say which provider and where, and give it the id
/// the provider issued.
///
/// The provider picker calls `above_modal`, without which the list paints under the panel holding
/// it and reads as a control that does nothing.
pub fn app_form(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(form) = app.workbench.settings.app_form.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let focused = |input: &gpui::Entity<gpui_component::input::InputState>| {
        input.read(cx).focus_handle(cx).is_focused(window)
    };
    let named = !app.login_account_input.read(cx).value().trim().is_empty();
    let identified = !app
        .connect_client_id_input
        .read(cx)
        .value()
        .trim()
        .is_empty();

    let providers: Vec<&str> = ProviderId::all()
        .iter()
        .map(|provider| provider.label())
        .collect();
    let picked = ProviderId::all()
        .iter()
        .position(|provider| *provider == form.provider)
        .unwrap_or(0);

    let text_field = |label: &str,
                      note: &str,
                      input: &gpui::Entity<gpui_component::input::InputState>|
     -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(label_block(label, note))
            .child(
                field(theme::border(), focused(input))
                    .h(px(30.))
                    .px_2()
                    .child(Input::new(input).appearance(false)),
            )
            .into_any_element()
    };

    let body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(text_field(
            "Name",
            "What to call this registration \u{2014} \"team ci\", \"personal\". Several may exist \
             for one provider and one install.",
            &app.login_account_input,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Provider",
                    "Which service the application is registered at.",
                ))
                .child(
                    Picker::new("app-settings-app-form-provider", form.provider.label())
                        .items(providers)
                        .selected(picked)
                        .open(form.open)
                        .above_modal()
                        .on_toggle(crate::ui::handler(&view, |this, _, cx| {
                            this.toggle_app_provider_picker(cx)
                        }))
                        .on_pick({
                            let view = view.clone();
                            move |index, window, cx| {
                                let provider = ProviderId::all()[index];
                                view.update(cx, |this, cx| {
                                    this.pick_app_provider(provider, window, cx)
                                });
                            }
                        })
                        .on_dismiss(crate::ui::handler(&view, |this, _, cx| {
                            this.toggle_app_provider_picker(cx)
                        })),
                ),
        )
        .child(text_field(
            "URL",
            "The base URL the application is registered at. The provider\u{2019}s own cloud to \
             start with; replace it with a self-managed install.",
            &app.connect_instance_input,
        ))
        .child(text_field(
            "Client id",
            "What the provider issued when the application was registered. Public \u{2014} it \
             travels in the query string of every authorization URL.",
            &app.connect_client_id_input,
        ))
        .child(text_field(
            "Client secret",
            "Only for a confidential application. Kept in the machine\u{2019}s credential store, \
             never in the settings file, and never shown again.",
            &app.connect_secret_input,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Callback URL",
                    "Register this at the provider, exactly as it reads. It is the address this \
                     machine listens on, and a browser flow fails without it.",
                ))
                .child(copyable(
                    "app-settings-app-form-redirect",
                    OAUTH_REDIRECT,
                    cx,
                )),
        )
        .into_any_element();

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "app-settings-app-form-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, window, cx| this.close_app_form(window, cx)),
        ))
        .child(
            primary_button(
                "app-settings-app-form-save",
                None,
                "Save",
                cx.listener(|this, _, window, cx| this.save_app_form(window, cx)),
            )
            .when(!(named && identified), |button| button.opacity(0.5)),
        )
        .into_any_element();

    modal(
        "app-settings-app-form-modal",
        theme::accent(),
        if form.id.is_some() {
            "Edit app registration"
        } else {
            "App registration"
        },
        body,
        footer,
        crate::ui::handler(&view, |this, window, cx| this.close_app_form(window, cx)),
        window,
    )
}

/// One labelled box in a form modal — [`app_form`]'s own field shape, as a function because the
/// provider form's fields are built in two places (a picker replaces two of them on an edit) and a
/// closure holding the render context would be alive across both.
fn form_field(
    label: &str,
    note: &str,
    input: &gpui::Entity<gpui_component::input::InputState>,
    focused: bool,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(label_block(label, note))
        .child(
            field(theme::border(), focused)
                .h(px(30.))
                .px_2()
                .child(Input::new(input).appearance(false)),
        )
        .into_any_element()
}

/// The API-provider form: which protocol, where, with which key, on which two models.
///
/// A [`modal`] rather than a [`prompt_modal`] because it asks several questions, and built on
/// [`app_form`]'s shape for the same reason that one is built on the login modal's — it is the
/// same kind of question, and a second visual language for it would be a second thing to learn.
///
/// **The key is write-only.** The box is always empty when the form opens, because the host never
/// sends a key and this half has none to draw. On an edit that is also how the stored key is kept:
/// blank means `key: None`, and only a typed one replaces what is filed.
///
/// **The model questions only exist on an edit.** A provider's model list is the provider's own
/// answer, asked for by id, and an add has no id until the host mints one — so an add asks no
/// model question at all, and needs none: writing the record is what files the key and makes the
/// host list that provider's models, so an edit opens on a full list. A provider with no model is
/// a real record in the meantime, and its row says so. On an edit each model is a box with a
/// picker beside it: the list fills the box, and typing in it is equally valid — see
/// [`model_row`].
pub fn ai_form(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(form) = app.workbench.settings.ai_form.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let editing = form.id.is_some();
    let named = !app.ai_name_input.read(cx).value().trim().is_empty();
    let keyed = !app.ai_key_input.read(cx).value().trim().is_empty();
    // A name is the only thing the host requires of a record, and a key is what makes an add
    // writable at all; an edit already has one filed, so a blank key box there is a choice rather
    // than an omission. A model is not required: a provider with none is exactly what one looks
    // like between being given a key and having its models listed.
    let ready = named && (editing || keyed);

    // Read up front, because the ring is drawn by the parent and every field wants the same
    // answer: which box holds the keyboard.
    let focused = |input: &gpui::Entity<gpui_component::input::InputState>| {
        input.read(cx).focus_handle(cx).is_focused(window)
    };
    let name_focus = focused(&app.ai_name_input);
    let url_focus = focused(&app.ai_base_url_input);
    let key_focus = focused(&app.ai_key_input);
    let fast_focus = focused(&app.ai_fast_model_input);
    let smart_focus = focused(&app.ai_smart_model_input);

    let kinds: Vec<AnyElement> = AiProviderKind::ALL
        .iter()
        .map(|kind| {
            let kind = *kind;
            choice_pill(
                ElementId::Name(format!("app-settings-ai-kind-{}", kind.code()).into()),
                kind.label(),
                form.kind == kind,
                cx.listener(move |this, _, _, cx| this.pick_ai_kind(kind, cx)),
            )
            .into_any_element()
        })
        .collect();

    let models: AnyElement = match form.id {
        Some(provider_id) => {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let listed = match app.workbench.settings.ai_models.get(&provider_id) {
                Some(list) => format!(
                    "{} models \u{b7} listed {} ago",
                    list.models.len(),
                    magnitude(now_ms - list.fetched_at_ms)
                ),
                None => "Never listed \u{2014} press Refresh, or type the id yourself.".to_string(),
            };
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(model_row(
                    app,
                    &form,
                    ModelRole::Fast,
                    fast_focus,
                    &view,
                    window,
                    cx,
                ))
                .child(model_row(
                    app,
                    &form,
                    ModelRole::Smart,
                    smart_focus,
                    &view,
                    window,
                    cx,
                ))
                .child(note(&listed, theme::text_faint()))
                .into_any_element()
        }
        // No model question at all. An add has no id, so there is no list to pick from, and
        // nothing worth typing from memory either: saving is what makes the list exist.
        None => note(
            "No model yet. Saving this files the key and asks the provider what it can run; Edit \
             then chooses the fast model from that list, and a smart one if you want it \u{2014} \
             or takes a model id typed by hand, for a provider that will not list its own. Until \
             one is set the provider reports itself unavailable, which is what its row says.",
            theme::text_faint(),
        ),
    };

    let body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(
            "A model behind an API, and the key to reach it. The key is filed in this \
             machine\u{2019}s credential store under the record\u{2019}s own id, and is never \
             read back \u{2014} not by this form, and not by anything else.",
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Kind",
                    "Which wire protocol the endpoint speaks. Everything a deployment varies \
                     \u{2014} the host, the path, the deployment name \u{2014} is the URL below \
                     rather than a kind of its own.",
                ))
                .child(div().flex().flex_wrap().gap_2().children(kinds)),
        )
        .child(form_field(
            "Name",
            "What to call it \u{2014} \"local\", \"work\". Several of one kind are ordinary, so \
             the name is what tells them apart.",
            &app.ai_name_input,
            name_focus,
        ))
        .child(form_field(
            "Base URL",
            "The endpoint, when it is not the kind\u{2019}s own. Empty means the kind\u{2019}s \
             own, which the host resolves.",
            &app.ai_base_url_input,
            url_focus,
        ))
        .child(form_field(
            "API key",
            if editing {
                "Leave it blank to keep the key already filed. Typing one replaces it."
            } else {
                "Required. It crosses once, on its way to this machine\u{2019}s credential store."
            },
            &app.ai_key_input,
            key_focus,
        ))
        .child(models)
        .into_any_element();

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "app-settings-ai-form-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, window, cx| this.close_ai_form(window, cx)),
        ))
        .child(
            primary_button(
                "app-settings-ai-form-save",
                None,
                "Save",
                cx.listener(|this, _, window, cx| this.save_ai_form(window, cx)),
            )
            .when(!ready, |button| button.opacity(0.5)),
        )
        .into_any_element();

    modal(
        "app-settings-ai-form-modal",
        theme::accent(),
        if editing {
            "Edit provider"
        } else {
            "Add provider"
        },
        body,
        footer,
        crate::ui::handler(&view, |this, window, cx| this.close_ai_form(window, cx)),
        window,
    )
}

/// One model question on an edit: the box the id lives in, the picker that fills it, and — on the
/// fast row — the button that re-asks the provider for its list.
///
/// The box is [`form_field`]'s own shape with the picker beside it rather than instead of it,
/// because the list is a convenience and the field is the way through: a provider whose models
/// cannot be listed is configured by typing an id, and there has to be somewhere to type it.
fn model_row(
    app: &AppState,
    form: &AiProviderForm,
    role: ModelRole,
    focused: bool,
    view: &gpui::Entity<AppState>,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let (label, about, input) = match role {
        ModelRole::Fast => (
            "Fast model",
            "What every subject uses unless it asks for the other one. Pick one from the \
             list, or type the id the provider answers to.",
            &app.ai_fast_model_input,
        ),
        ModelRole::Smart => (
            "Smart model",
            "The capable, slower one, for a subject that will not fit in the fast model. Leave \
             it empty and the fast model answers those too.",
            &app.ai_smart_model_input,
        ),
    };

    let mut row = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            field(theme::border(), focused)
                .h(px(30.))
                .flex_1()
                .min_w(px(0.))
                .px_2()
                .child(Input::new(input).appearance(false)),
        )
        .child(model_picker(app, form, role, view, window, cx));
    // One Refresh for the two rows: it re-asks for the provider's whole list, which is what both
    // pickers read, so a second button would be the same call under another name.
    if role == ModelRole::Fast {
        row = row.child(ghost_button(
            "app-settings-ai-models-refresh",
            None,
            "Refresh",
            cx.listener(|this, _, _, cx| this.refresh_ai_models(cx)),
        ));
    }

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(label_block(label, about))
        .child(row)
        .into_any_element()
}

/// One model picker: the provider's cached list, filtered by what is typed above it.
///
/// [`Picker::above_modal`] because it is drawn inside one, and [`Picker::search`] because a
/// provider's list runs to hundreds of rows — the caller filters and the kit draws the field,
/// which is the split that method documents. The smart picker's first row is None, meaning the
/// fast model, and it is kept whatever is typed: it is the way back rather than a model.
///
/// **The list is a convenience over the field beside it, not a replacement for it**, because a
/// provider that will not list its models must still be usable — Azure OpenAI addresses
/// deployments rather than models, and a listing can fail outright at a proxy or on a key scoped
/// to inference. So picking a row writes an id into the box, which is the only store either way,
/// and which row reads as selected is whatever that box currently holds.
fn model_picker(
    app: &AppState,
    form: &AiProviderForm,
    role: ModelRole,
    view: &gpui::Entity<AppState>,
    window: &Window,
    cx: &gpui::App,
) -> AnyElement {
    let (value_input, search_input) = match role {
        ModelRole::Fast => (&app.ai_fast_model_input, &app.ai_fast_search),
        ModelRole::Smart => (&app.ai_smart_model_input, &app.ai_smart_search),
    };
    let chosen = value_input.read(cx).value().trim().to_string();
    let query = search_input.read(cx).value().trim().to_lowercase();

    // What each row is called, and what picking it stores. `None` is the smart picker's own first
    // row; every other row is a model the provider said it has.
    let mut rows: Vec<(String, Option<String>)> = Vec::new();
    if role == ModelRole::Smart {
        rows.push(("None (use the fast model)".to_string(), None));
    }
    rows.extend(
        form.id
            .and_then(|id| app.workbench.settings.ai_models.get(&id))
            .map(|list| list.models.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|model| {
                query.is_empty()
                    || model.id.to_lowercase().contains(&query)
                    || model.label.to_lowercase().contains(&query)
            })
            .map(|model| (model.label.clone(), Some(model.id.clone()))),
    );

    // An empty box is the None row on the smart picker — its own first row, at index 0 — and is
    // nothing at all on the fast one, where no row means "no model".
    let selected = if chosen.is_empty() {
        (role == ModelRole::Smart).then_some(0)
    } else {
        rows.iter()
            .position(|(_, id)| id.as_deref() == Some(chosen.as_str()))
    };
    let picks: Vec<Option<String>> = rows.iter().map(|(_, id)| id.clone()).collect();
    let items: Vec<String> = rows.into_iter().map(|(label, _)| label).collect();

    // The trigger names the list rather than the value: the value is already on screen in the box
    // beside it, and a trigger repeating it would draw one fact twice.
    let mut picker = Picker::new(
        ElementId::Name(format!("app-settings-ai-model-{}", role.code()).into()),
        "Pick\u{2026}",
    )
    .items(items)
    .open(form.open == Some(role))
    .above_modal()
    .search(
        search_input,
        search_input.read(cx).focus_handle(cx).is_focused(window),
    )
    .on_toggle(crate::ui::handler(view, move |this, _, cx| {
        this.toggle_ai_model_picker(role, cx)
    }))
    .on_pick({
        let view = view.clone();
        move |index, window, cx| {
            let Some(model) = picks.get(index).cloned() else {
                return;
            };
            view.update(cx, |this, cx| {
                this.pick_ai_model(role, model, window, cx);
            });
        }
    })
    .on_dismiss(crate::ui::handler(view, move |this, _, cx| {
        this.toggle_ai_model_picker(role, cx)
    }));
    if let Some(index) = selected {
        picker = picker.selected(index);
    }

    picker.into_any_element()
}

/// The removal question. Danger, because the key in the machine's credential store goes with the
/// record and there is nothing left behind to reattach a re-added provider to.
pub fn ai_remove(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(provider_id) = app.workbench.settings.ai_remove else {
        return div().into_any_element();
    };
    let view = cx.entity();
    // A provider another window has already removed leaves the question naming nothing, which is
    // still answerable — the host has already forgotten it.
    let name = app
        .workbench
        .settings
        .ai_provider(provider_id)
        .map(|info| info.provider.name.clone())
        .unwrap_or_else(|| "this provider".to_string());

    confirm_modal(
        "app-settings-ai-remove",
        "Remove provider",
        &format!(
            "Remove {name}? The key filed under it goes from this machine\u{2019}s credential \
             store with it, so adding it again is a new record and a re-typed key. If assistance \
             names this provider, it falls back to calling no model at all."
        ),
        "Remove",
        true,
        crate::ui::handler(&view, |this, _, cx| this.confirm_remove_ai_provider(cx)),
        crate::ui::handler(&view, |this, _, cx| this.close_remove_ai_provider(cx)),
        window,
    )
}

/// The provider test: one short answer from one provider, drawn as it arrives.
///
/// **The streaming is the point.** A check that shows a spinner until the whole answer lands says
/// nothing about how long the first token took, which is most of what a user is checking. So the
/// body draws whatever `SuggestChunk` has delivered so far, and a "waiting" line before the first
/// one — see [`crate::state::settings::AiTest`].
pub fn ai_test(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(test) = app.workbench.settings.ai_test.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let running = test.suggest_id.is_some();
    let ran = !test.answer.is_empty() || test.error.is_some();

    let roles = div()
        .flex()
        .items_center()
        .gap_1()
        .child(test_role_pill(
            "app-settings-ai-test-fast",
            "Fast",
            ModelRole::Fast,
            test.role,
            true,
            cx,
        ))
        .child(test_role_pill(
            "app-settings-ai-test-smart",
            "Smart",
            ModelRole::Smart,
            test.role,
            test.has_smart,
            cx,
        ));

    let answer: AnyElement = match &test.error {
        Some(error) => note(error, theme::danger()),
        None if !test.answer.is_empty() => slab(theme::accent())
            .p_2()
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .text_color(theme::text())
                    .child(SharedString::from(test.answer.clone())),
            )
            .into_any_element(),
        None if running => note("Waiting for the first token\u{2026}", theme::text_faint()),
        None => note(
            "Nothing yet. Run the test to see what this provider answers.",
            theme::text_faint(),
        ),
    };

    let body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(
            "One short answer from this provider, to the host\u{2019}s own prompt. It is the one \
             subject that names its backend instead of using the selected one, because the \
             provider just configured is what is being checked \u{2014} and it streams, so a \
             first token arrives long before the last.",
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Model",
                    "Which of the provider\u{2019}s two models answers. Smart is offered only \
                     where one is configured.",
                ))
                .child(roles),
        )
        .child(div().flex().items_center().gap_2().child(primary_button(
            "app-settings-ai-test-run",
            None,
            if ran { "Rerun" } else { "Run test" },
            cx.listener(|this, _, _, cx| this.run_ai_test(cx)),
        )))
        .child(answer)
        .into_any_element();

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "app-settings-ai-test-close",
            None,
            "Close",
            cx.listener(|this, _, _, cx| this.close_ai_test(cx)),
        ))
        .into_any_element();

    modal(
        "app-settings-ai-test-modal",
        theme::accent(),
        &format!("Test {}", test.name),
        body,
        footer,
        crate::ui::handler(&view, |this, _, cx| this.close_ai_test(cx)),
        window,
    )
}

/// One of the test's two role pills. Smart is drawn dead rather than dropped where the provider
/// configured no smart model: a row that vanishes reads as gone, not as unset.
fn test_role_pill(
    id: &'static str,
    label: &'static str,
    role: ModelRole,
    current: ModelRole,
    live: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let pill = choice_pill(
        id,
        label,
        current == role,
        cx.listener(move |this, _, _, cx| this.pick_ai_test_role(role, cx)),
    );

    if live {
        pill.into_any_element()
    } else {
        pill.opacity(0.5).into_any_element()
    }
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
            choosing_connector(
                app,
                *provider,
                *auth,
                connect.app,
                connect.app_open,
                window,
                cx,
            ),
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

/// Step one: which provider, which application, and which flow.
///
/// Where the identity lives is the *application's* fact rather than a second thing to type: a
/// registration knows its own instance, so picking one sets both and neither can disagree with the
/// other. Typing a base URL is still offered, because a pasted token needs no application at all
/// and a self-hosted install must stay reachable without one registered — and that field asks for a
/// base URL, not a host name, since an on-premises install can live under a path.
#[allow(clippy::too_many_arguments)]
fn choosing_connector(
    app: &AppState,
    chosen: Option<ProviderId>,
    auth: Option<AuthKind>,
    picked_app: ConnectApp,
    app_open: bool,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let typed = app.connect_instance_input.read(cx).value().to_string();
    let registration = match picked_app {
        ConnectApp::Registration(id) => app
            .workbench
            .settings
            .host
            .oauth_apps
            .iter()
            .find(|held| held.id == id),
        _ => None,
    };
    let self_hosted = match picked_app {
        ConnectApp::Bundled => false,
        ConnectApp::Registration(_) => registration.is_some_and(|held| held.origin.is_some()),
        ConnectApp::Instance => !typed.trim().is_empty(),
    };
    // Built before the borrow below: the picker wants `cx` mutably, and `focused` holds it.
    let application =
        chosen.map(|provider| application_picker(app, provider, picked_app, app_open, cx));
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
        body = body.children(application);

        if picked_app == ConnectApp::Instance && provider.instance_need() != InstanceNeed::Never {
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
            && picked_app == ConnectApp::Instance
        {
            // Nothing registered and nothing built in: the browser flow has no application to open
            // as, so it is refused here rather than at an authorization URL with no id in it.
            if app.workbench.settings.host.oauth_apps.is_empty() {
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(modal_note(
                            "A browser flow on a self-managed install needs an application \
                             registered on that install. Register one, or paste a token instead.",
                        ))
                        .child(ghost_button(
                            "app-settings-connect-register",
                            Some(IconName::Plus),
                            "App registration\u{2026}",
                            cx.listener(|this, _, window, cx| this.open_app_form(None, window, cx)),
                        )),
                );
            }
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

/// Which application the flow authenticates as, as a list of what is actually available.
///
/// "Default" is offered only where the *host* said this build ships a registered application for
/// the provider — the interface never guesses at one, because a browser sent to an authorization
/// URL with no client id in it fails at the provider rather than here.
fn application_picker(
    app: &AppState,
    provider: ProviderId,
    picked: ConnectApp,
    open: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    let bundled = app.workbench.settings.bundled.contains(&provider);
    let registrations: Vec<OauthApp> = app
        .workbench
        .settings
        .host
        .oauth_apps
        .iter()
        .filter(|held| held.provider == provider)
        .cloned()
        .collect();

    let mut choices = Vec::new();
    let mut items: Vec<String> = Vec::new();
    if bundled {
        choices.push(ConnectApp::Bundled);
        items.push("Default".to_string());
    }
    for held in &registrations {
        choices.push(ConnectApp::Registration(held.id));
        items.push(format!(
            "{} \u{b7} {}",
            held.name,
            held.origin.clone().unwrap_or_else(|| "cloud".to_string())
        ));
    }
    choices.push(ConnectApp::Instance);
    items.push("Another instance\u{2026}".to_string());

    let selected = choices.iter().position(|choice| *choice == picked);
    let label = selected
        .and_then(|index| items.get(index).cloned())
        .unwrap_or_else(|| "Another instance\u{2026}".to_string());
    let note = if bundled {
        "The application Ubiq signs in through. \"Default\" is the one this build ships."
    } else {
        "The application Ubiq signs in through. This build ships none for this provider, so a \
         browser flow needs a registration."
    };

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(label_block("Application", note))
        .child(
            Picker::new("app-settings-connect-app", label)
                .items(items)
                .selected(selected.unwrap_or(choices.len() - 1))
                .open(open)
                .above_modal()
                .on_toggle(crate::ui::handler(&view, |this, _, cx| {
                    this.toggle_connect_app_picker(cx)
                }))
                .on_pick({
                    let view = view.clone();
                    move |index, _, cx| {
                        let Some(choice) = choices.get(index).copied() else {
                            return;
                        };
                        view.update(cx, |this, cx| this.pick_connect_app(choice, cx));
                    }
                })
                .on_dismiss(crate::ui::handler(&view, |this, _, cx| {
                    this.toggle_connect_app_picker(cx)
                })),
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
                .child(slab(theme::accent()).px_3().py_2().child(
                    mono(user_code.to_string(), theme::text()).text_size(theme::font_display()),
                ))
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
                    .text_size(theme::font(Family::Chrome, Role::Meta))
                    .text_color(theme::text_faint())
                    .child(SharedString::from(label.to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(theme::font(Family::Chrome, Role::Meta))
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
                        .child(
                            mono(copy.clone(), theme::text())
                                .text_size(theme::font(Family::Chrome, Role::Meta)),
                        ),
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
        Some(ConnectorDialog::DeleteApp { name, .. }) => confirm_modal(
            "app-settings-oauth-delete",
            "Delete registration",
            &format!(
                "Delete {name}? Its client secret goes with it. Connections made under it keep \
                 working \u{2014} each holds its own client id \u{2014} but nothing can be \
                 connected under it again."
            ),
            "Delete",
            true,
            crate::ui::handler(&view, |this, _, cx| this.confirm_delete_app(cx)),
            crate::ui::handler(&view, |this, _, cx| this.close_connector_dialog(cx)),
            window,
        ),
    }
}
