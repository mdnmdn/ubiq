//! Project settings: the sink's fixture page, and the dialog a real project raises.
//!
//! **The kit's modal is one question at `MODAL_WIDTH`; this layout is a form with a nav.** The
//! shape is the same shape: square, `surface_raised`, a coloured left edge. On the sink it is
//! drawn on the page so the whole of it can be looked at. Over the workbench it is the same
//! dialog, raised after a folder is chosen or from the titlebar's 3-dot, with only General
//! enabled and the path immutable.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, ElementId, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Rgba, SharedString, Stateful, StatefulInteractiveElement, Styled, Window,
    anchored, deferred, div, point, px, relative,
};
use gpui_component::input::{Input, InputState, Textarea, TextareaState};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::ids::ProjectId;
use ubiq_proto::kb::KbSourceState;
use ubiq_proto::messages::ProfileInfo;
use ubiq_proto::projects::{IndexChange, IndexLevel, MissionTermChange};
use ubiq_proto::settings::DronePreset;
use ubiq_proto::work::Status;

use crate::app::AppState;
use crate::state::git::head_label;
use crate::state::settings::ToolEditScope;
use crate::state::sink::{
    ColourField, PROJECT_ABOUT, PROJECT_ABOUT_LIMIT, PROJECT_BRANCH, PROJECT_COLOUR, PROJECT_MARK,
    PROJECT_NAME, PROJECT_PATH, ProjectNav, hex_string,
};
use crate::state::workbench::ProjectSettingsMode;
use crate::state::{Layer, RailMode, WindowRegistry};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::board::status_colour;
use crate::ui::hsv;
use crate::ui::kit::{
    UbiqIcon, check_box, choice_pill, elided, ghost_button, heading, icon_button, mono, nav_item,
    primary_button, setting_row, toggle_pill,
};
use crate::ui::rail::mode_icon;
use crate::ui::sink::style::{framed_active, input_on, textarea_on};

/// Which copy of the dialog is being drawn. The sink is a fixture; Live is create or edit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    Sink,
    Live,
}

impl Form {
    fn prefix(self) -> &'static str {
        match self {
            Form::Sink => "sink-project",
            Form::Live => "project-settings",
        }
    }
}

fn colour_of(app: &AppState, form: Form) -> ColourField {
    match form {
        Form::Sink => app.sink.project.colour,
        Form::Live => app
            .workbench
            .project_settings
            .as_ref()
            .map(|settings| settings.colour)
            .unwrap_or(ColourField::NONE),
    }
}

fn form_name(app: &AppState, form: Form) -> &Entity<InputState> {
    match form {
        Form::Sink => &app.sink_project_name,
        Form::Live => &app.rename_input,
    }
}

fn form_about(app: &AppState, form: Form) -> &Entity<TextareaState> {
    match form {
        Form::Sink => &app.sink_project_about,
        Form::Live => &app.project_form_about,
    }
}

fn form_hex(app: &AppState, form: Form) -> &Entity<InputState> {
    match form {
        Form::Sink => &app.sink_project_hex,
        Form::Live => &app.project_form_hex,
    }
}

fn form_path(app: &AppState, form: Form, cx: &gpui::App) -> String {
    match form {
        Form::Sink => PROJECT_PATH.to_string(),
        Form::Live => match app.workbench.project_settings.as_ref().map(|s| &s.mode) {
            Some(ProjectSettingsMode::Create { path }) => path.clone(),
            Some(ProjectSettingsMode::Edit { project }) => WindowRegistry::read(cx)
                .project(*project)
                .map(|entry| entry.record.path.clone())
                .unwrap_or_default(),
            None => String::new(),
        },
    }
}

/// Abbreviate a path under the user's home directory to `~`, the way a shell prompt does.
/// Only the home directory itself, or a path under it, is rewritten — `/Users/mdnother` is not
/// mistaken for a child of `/Users/mdn` by a naive prefix check. Display-only: nothing stored or
/// sent is ever the abbreviated form.
pub fn home_abbreviated(path: &str) -> String {
    let Ok(home) = std::env::var("HOME") else {
        return path.to_string();
    };
    let home = home.trim_end_matches('/');
    if home.is_empty() {
        return path.to_string();
    }
    if path == home {
        return "~".to_string();
    }
    match path.strip_prefix(home) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

fn form_mark(app: &AppState, form: Form, cx: &gpui::App) -> String {
    // An override typed into the Initials field wins outright, the same rule the rail's own
    // badge follows for a saved record — see `ui/rail.rs`'s `project_badges`.
    let override_text = app.project_initials_input.read(cx).value().to_string();
    if !override_text.is_empty() {
        return override_text;
    }
    match form {
        Form::Sink => PROJECT_MARK.to_string(),
        Form::Live => {
            let name = form_name(app, form).read(cx).value();
            let mut chars = name.chars().filter(|c| c.is_alphanumeric());
            match (chars.next(), chars.next()) {
                (Some(a), Some(b)) => format!("{a}{b}").to_lowercase(),
                (Some(a), None) => a.to_lowercase().to_string(),
                _ => "?".to_string(),
            }
        }
    }
}

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .id("sink-project")
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .bg(theme::app_bg())
        .p_6()
        .child(dialog(app, window, cx, Form::Sink))
        .into_any_element()
}

/// The same dialog, over the window, after a folder is chosen or from the titlebar's 3-dot.
pub fn overlay(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let viewport = window.viewport_size();
    // The bottom rung of the window's overlay stack: everything this page raises is painted over
    // it, and a click inside any of them is outside *this* panel's bounds. `AppState::covered`
    // answers for all of them at once — dropping the page from under a question takes the project
    // that question is about with it.
    let panel =
        dialog(app, window, cx, Form::Live).on_mouse_down_out(cx.listener(|this, _, _, cx| {
            if this.covered(Layer::ProjectSettings) {
                return;
            }
            this.close_project_settings(cx)
        }));

    deferred(
        anchored().position(point(px(0.), px(0.))).child(
            div()
                .id("project-settings")
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
    form: Form,
) -> Stateful<gpui::Div> {
    let colour = current_rgba(app, form);
    let prefix = form.prefix();

    div()
        .id(ElementId::Name(format!("{prefix}-dialog").into()))
        .w(px(820.))
        .max_h(relative(1.))
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::accent_edge()))
        .border_color(colour)
        .shadow_lg()
        .child(header(app, form, colour, cx))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .child(nav(app, form, cx))
                .child(body(app, window, cx, form)),
        )
        .child(footer(app, form, cx))
}

fn header(
    app: &AppState,
    form: Form,
    colour: gpui::Rgba,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let prefix = form.prefix();
    let path = form_path(app, form, cx);
    let mark = form_mark(app, form, cx);
    let mut path_line = div().flex().items_center().gap_1().child(
        elided(
            ElementId::Name(format!("{prefix}-path").into()),
            path,
            theme::text_faint(),
            theme::font(theme::Family::Chrome, theme::Role::Meta),
        )
        .flex_none(),
    );
    if form == Form::Sink {
        path_line = path_line
            .child(
                mono("·", theme::text_faint()).text_size(theme::font(Family::Chrome, Role::Meta)),
            )
            .child(
                mono(PROJECT_BRANCH, theme::text_faint())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            );
    }

    let close = match form {
        Form::Sink => icon_button(
            ElementId::Name(format!("{prefix}-close").into()),
            IconName::Close,
            false,
            |_, _, _| {},
        ),
        Form::Live => icon_button(
            ElementId::Name(format!("{prefix}-close").into()),
            IconName::Close,
            false,
            cx.listener(|this, _, _, cx| this.close_project_settings(cx)),
        ),
    };

    div()
        .h(px(52.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(
            div()
                .size(px(28.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .bg(colour)
                .child(
                    mono(mark, theme::on_accent())
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                        .font_weight(FontWeight::SEMIBOLD),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Title))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child("Project settings"),
                )
                .child(path_line),
        )
        .child(close)
        .into_any_element()
}

fn nav(app: &AppState, form: Form, cx: &mut Context<AppState>) -> AnyElement {
    let current = match form {
        Form::Sink => app.sink.project.nav,
        Form::Live => app
            .workbench
            .project_settings
            .as_ref()
            .map(|settings| settings.nav)
            .unwrap_or(ProjectNav::General),
    };
    let prefix = form.prefix();
    // The live dialog is create-or-edit, not a page: it opens on General, and only an existing
    // project answers to Tools or Remote — a folder not yet in the catalogue has no record to
    // carry either.
    let live_record = form == Form::Live
        && matches!(
            app.workbench
                .project_settings
                .as_ref()
                .map(|settings| &settings.mode),
            Some(ProjectSettingsMode::Edit { .. })
        );
    let items: Vec<AnyElement> = ProjectNav::all()
        .iter()
        .copied()
        .map(|item| {
            // Tasks, Remote and the knowledge base are on Tools' footing exactly: all four attach
            // to a record, and a folder with no record yet has nothing to pin.
            let enabled = form == Form::Sink
                || item == ProjectNav::General
                || (matches!(
                    item,
                    ProjectNav::Tools | ProjectNav::Tasks | ProjectNav::Remote | ProjectNav::Kb
                ) && live_record);
            // The one count that is a live fact rather than fixture copy: it is how many sources
            // the section below lists.
            let count = match (item, form) {
                (ProjectNav::Kb, Form::Live) => app.kb(cx).map(|kb| kb.sources.len()),
                _ => item.count().map(|n| n as usize),
            };
            nav_item(
                ElementId::Name(format!("{prefix}-nav-{}", item.label()).into()),
                project_icon(item),
                item.label(),
                count,
                item == current,
                enabled,
                cx.listener(move |this, _, _, cx| this.set_sink_project_nav(item, cx)),
            )
        })
        .collect();

    div()
        .id(ElementId::Name(format!("{prefix}-nav").into()))
        .w(px(200.))
        .flex()
        .flex_none()
        .flex_col()
        .gap_1()
        .px_2()
        .py_3()
        .border_r_1()
        .border_color(theme::border())
        .children(items)
        .into_any_element()
}

fn project_icon(item: ProjectNav) -> Icon {
    match item {
        ProjectNav::General => Icon::new(IconName::Settings),
        ProjectNav::Tools => Icon::new(IconName::Play),
        // The rail's own Tasks mark, so the row that configures the board and the rail that opens
        // it read as the same thing — `Kb` below takes its icon for the same reason.
        ProjectNav::Tasks => Icon::new(UbiqIcon::ModeTasks),
        // Borrowed, not drawn. `Network` is already Integrations', and the question this panel
        // asks is *which machine*, which is the globe's.
        ProjectNav::Remote => Icon::new(IconName::Globe),
        // The rail's own KB mark, so the row that configures the screen and the rail that opens
        // it read as the same thing.
        ProjectNav::Kb => Icon::new(UbiqIcon::ModeKb),
        ProjectNav::Documentation => Icon::new(IconName::BookOpen),
        ProjectNav::Integrations => Icon::new(IconName::Network),
    }
}

fn body(app: &AppState, window: &Window, cx: &mut Context<AppState>, form: Form) -> AnyElement {
    let nav = match form {
        Form::Sink => app.sink.project.nav,
        Form::Live => app
            .workbench
            .project_settings
            .as_ref()
            .map(|settings| settings.nav)
            .unwrap_or(ProjectNav::General),
    };
    let content = match nav {
        ProjectNav::General => general(app, window, cx, form),
        ProjectNav::Tools => project_tools(app, cx, form),
        ProjectNav::Tasks => tasks(app, window, cx, form),
        ProjectNav::Remote => remote(app, cx, form),
        ProjectNav::Kb => kb(app, form, window, cx),
        ProjectNav::Documentation => documentation(),
        ProjectNav::Integrations => integrations(app, form, window, cx),
    };
    let prefix = form.prefix();

    div()
        .id(ElementId::Name(format!("{prefix}-body").into()))
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scroll()
        .px_5()
        .py_4()
        .child(content)
        .into_any_element()
}

/// The git repositories inside this project: the project's own root, always managed and never a
/// switchable row, then one row per [`ubiq_proto::git::GitNested`] the last working-tree walk
/// found. Ticking one sends the whole list at once, the same rule `search_excludes_row`'s controls
/// follow — see `AppState::set_project_managed_repos`.
///
/// Reads the window's own data for the project the *form* names, not for the one on screen — this
/// dialog is raised from the projects list as well, for a project the window holds without showing
/// — because which repositories exist is what a walk found and not what the shared record says. A
/// project this window does not hold has had no walk, and the list says so rather than claiming
/// the project holds nothing.
fn repos_row(app: &AppState, project: ProjectId, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let Some(open) = app.held_project(project) else {
        return Some(unwalked_repos_row());
    };
    let own_head = open.git.as_ref().map(|overview| head_label(&overview.head));
    let repos = open.git_repos.clone();

    let mut rows: Vec<AnyElement> = Vec::new();
    if let Some(head) = own_head {
        rows.push(
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
                        .child(
                            mono(".".to_string(), theme::text())
                                .text_size(theme::font(Family::Chrome, Role::Body)),
                        )
                        .child(
                            div()
                                .text_size(theme::font(Family::Chrome, Role::Meta))
                                .text_color(theme::text_faint())
                                .child(head),
                        ),
                )
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                        .text_color(theme::text_faint())
                        .child("this project's own repository"),
                )
                .into_any_element(),
        );
    }

    if repos.is_empty() {
        rows.push(
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_faint())
                .child("No other repositories found inside the project.")
                .into_any_element(),
        );
    } else {
        for repo in &repos {
            let rel_path = repo.rel_path.clone();
            let managed = repo.managed;
            let toggled = rel_path.clone();
            let id = ElementId::Name(format!("project-repo-{rel_path}").into());
            rows.push(
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
                            .child(
                                mono(rel_path.clone(), theme::text())
                                    .text_size(theme::font(Family::Chrome, Role::Body)),
                            )
                            .when(repo.submodule, |this| {
                                this.child(
                                    div()
                                        .text_size(theme::font(Family::Chrome, Role::Meta))
                                        .text_color(theme::text_faint())
                                        .child("submodule"),
                                )
                            }),
                    )
                    .child(check_box(
                        id,
                        managed,
                        cx.listener(move |this, _, _, cx| {
                            this.toggle_project_managed_repo(project, toggled.clone(), cx)
                        }),
                    ))
                    .into_any_element(),
            );
        }
    }

    Some(
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .py_3()
            .border_b_1()
            .border_color(theme::border())
            .child(label_line(
                "Repositories",
                "Every git repository the last walk found inside this project. A managed one \
                 colours the explorer and shows on the Git screen; an ignored one is only listed \
                 here.",
            ))
            .child(div().flex().flex_col().children(rows))
            .into_any_element(),
    )
}

/// What the repositories section says for a project this window does not hold open: nothing has
/// walked it, so the list would be empty for a reason that has nothing to do with the project.
fn unwalked_repos_row() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .py_3()
        .border_b_1()
        .border_color(theme::border())
        .child(label_line(
            "Repositories",
            "Every git repository the last walk found inside this project. A managed one colours \
             the explorer and shows on the Git screen; an ignored one is only listed here.",
        ))
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_faint())
                .child("Open this project to list the repositories inside it."),
        )
        .into_any_element()
}

/// The project this form is editing, where there is one.
///
/// Only a project that already exists can carry an override: the Sink form and the Create mode are
/// both about a project with no record yet, and there is nothing to override until there is.
fn form_project(app: &AppState, form: Form, _cx: &gpui::App) -> Option<ProjectId> {
    match form {
        Form::Sink => None,
        Form::Live => match app.workbench.project_settings.as_ref().map(|s| &s.mode) {
            Some(ProjectSettingsMode::Edit { project }) => Some(*project),
            _ => None,
        },
    }
}

/// This project's own runnable tools, on top of the machine-wide rows.
///
/// Sent immediately on every save or remove, the same rule `index_row`'s pills follow — see
/// `AppState::set_project_tools`. Only a project that already exists can carry tools: the Sink
/// form and the Create mode are both about a project with no record yet.
fn project_tools(app: &AppState, cx: &mut Context<AppState>, form: Form) -> AnyElement {
    let Some(project) = form_project(app, form, cx) else {
        return div()
            .text_size(theme::font(Family::Chrome, Role::Label))
            .text_color(theme::text_faint())
            .child("Tools belong to a project in the catalogue — name this folder first.")
            .into_any_element();
    };
    crate::ui::tools::panel(app, cx, ToolEditScope::Project(project))
}

/// This project's indexing level: four choices, because "follow the default" is one of them and is
/// not the same as happening to match it today.
fn index_row(app: &AppState, project: ProjectId, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let record = WindowRegistry::read(cx).project(project)?.record.clone();
    let current = record.index;
    let default = app.workbench.settings.host.index_level;

    let pill =
        |id: String, label: String, want: Option<IndexLevel>, current: Option<IndexLevel>| {
            let change = match want {
                None => IndexChange::Inherit,
                Some(level) => IndexChange::Set(level),
            };
            choice_pill(
                ElementId::Name(id.into()),
                label,
                current == want,
                cx.listener(move |this, _, _, cx| this.set_project_index(project, change, cx)),
            )
        };

    let default_label = match default {
        IndexLevel::None => "Default (off)",
        IndexLevel::Light => "Default (full text)",
        IndexLevel::Full => "Default (full text + symbols)",
    };

    Some(
        setting_row(
            "Keep an index",
            "What Ubiq remembers about this project, overriding the application setting. Off walks \
             every file on every query — worth it for a project on a slow disk, or one you would \
             rather Ubiq kept nothing about.",
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .flex_wrap()
                .child(pill(
                    "project-index-default".to_string(),
                    default_label.to_string(),
                    None,
                    current,
                ))
                .child(pill(
                    "project-index-none".to_string(),
                    "Off".to_string(),
                    Some(IndexLevel::None),
                    current,
                ))
                .child(pill(
                    "project-index-light".to_string(),
                    "Full text".to_string(),
                    Some(IndexLevel::Light),
                    current,
                ))
                .child(pill(
                    "project-index-full".to_string(),
                    "Full text + symbols".to_string(),
                    Some(IndexLevel::Full),
                    current,
                ))
                .into_any_element(),
        )
        .into_any_element(),
    )
}

/// This project's own word for a mission: the application-wide default, or a word of its own —
/// [`MissionTermChange`]'s shape, the same pair of answers [`index_row`] draws for the indexing
/// level. "Default" sends `Inherit` at once, the way every pill here does; "Custom" seeds the
/// field with what is held today and reveals it, and typing into the field commits on Enter and
/// on blur through `AppState::set_project_mission_term_from_field`.
fn mission_term_row(
    app: &AppState,
    project: ProjectId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    let record = WindowRegistry::read(cx).project(project)?.record.clone();
    let default = app.workbench.settings.host.mission_term.clone();
    let overriding = record.mission_term.is_some();

    let mut column = div().flex().flex_col().gap_1p5().child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(choice_pill(
                "project-mission-term-default",
                format!("Default ({default})"),
                !overriding,
                cx.listener(move |this, _, _, cx| {
                    this.set_project_mission_term(project, MissionTermChange::Inherit, cx)
                }),
            ))
            .child(choice_pill(
                "project-mission-term-custom",
                "Custom",
                overriding,
                cx.listener(move |this, _, window, cx| {
                    this.begin_project_mission_term_override(project, window, cx)
                }),
            )),
    );
    if overriding {
        column = column.child(
            framed_active(
                theme::border(),
                input_on(&app.project_mission_term_input, window, cx),
            )
            .h(px(30.))
            .w(px(220.))
            .items_center()
            .px_2()
            .child(Input::new(&app.project_mission_term_input).appearance(false)),
        );
    }

    Some(
        setting_row(
            "Mission term",
            "What this project calls a mission, overriding the application-wide word. Follow the \
             default to always match it, even when it changes.",
            column.into_any_element(),
        )
        .into_any_element(),
    )
}

/// This project's search excludes: gitignore-style patterns and folders, on top of the
/// application's own list and the ignore rules the project already carries. Each removes with a
/// click; a folder picker and a field for the wildcards a picker cannot express both add one.
///
/// Sent immediately on every add or remove, the same rule `index_row`'s pills follow — see
/// `AppState::set_project_search_excludes`.
fn search_excludes_row(
    app: &AppState,
    project: ProjectId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    let excludes = WindowRegistry::read(cx)
        .project(project)?
        .record
        .search_excludes
        .clone();

    let rows: Vec<AnyElement> = if excludes.is_empty() {
        vec![
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_faint())
                .child("Nothing excluded beyond the application's own list.")
                .into_any_element(),
        ]
    } else {
        excludes
            .iter()
            .enumerate()
            .map(|(index, pattern)| {
                let removed = pattern.clone();
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .py_1()
                    .child(
                        mono(pattern.clone(), theme::text())
                            .text_size(theme::font(Family::Chrome, Role::Body)),
                    )
                    .child(icon_button(
                        ElementId::Name(format!("project-exclude-remove-{index}").into()),
                        IconName::Close,
                        false,
                        cx.listener(move |this, _, _, cx| {
                            this.remove_project_search_exclude(project, removed.clone(), cx)
                        }),
                    ))
                    .into_any_element()
            })
            .collect()
    };

    let exclude_input = &app.project_exclude_input;

    Some(
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .py_3()
            .border_b_1()
            .border_color(theme::border())
            .child(label_line(
                "Search excludes",
                "Folders and gitignore-style patterns this project's own searches skip. \
                 `*.log` or `**/build` both work.",
            ))
            .child(div().flex().flex_col().children(rows))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(ghost_button(
                        "project-exclude-add-folder",
                        Some(IconName::FolderOpen),
                        "Add folder\u{2026}",
                        cx.listener(|this, _, _, cx| this.choose_project_search_exclude(cx)),
                    ))
                    .child(
                        framed_active(theme::border(), input_on(exclude_input, window, cx))
                            .h(px(28.))
                            .w(px(200.))
                            .items_center()
                            .child(Input::new(exclude_input).appearance(false)),
                    ),
            )
            .into_any_element(),
    )
}

/// The project's knowledge base: one row per configured source, and the one way to add another.
///
/// **A knowledge base hangs off a record**, so the sink's fixture page and the create form draw
/// nothing here: there is no project for a source to belong to, and a form that collected sources
/// before the project existed would have nowhere to send them.
///
/// Every edit is sent as it is made, the same rule `search_excludes_row` follows — the whole list
/// rides one message, so a removal and a filter edit are each one fact rather than three.
fn kb(app: &AppState, form: Form, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let live_record = form == Form::Live
        && matches!(
            app.workbench
                .project_settings
                .as_ref()
                .map(|settings| &settings.mode),
            Some(ProjectSettingsMode::Edit { .. })
        );
    if !live_record {
        return div().into_any_element();
    }

    let sources = app.kb(cx).map(|kb| &kb.sources);
    let rows: Vec<AnyElement> = match sources {
        Some(sources) if !sources.is_empty() => sources
            .iter()
            .map(|view| {
                let id = view.id();
                let filter = app.kb_filter_inputs.get(&id);
                let (word, colour) = state_line(&view.status.state);
                let access = access_word(&view.status.source);
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .py_1p5()
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .gap_1()
                            .child(elided(
                                crate::ui::eid("project-kb-name", id),
                                view.name().to_string(),
                                theme::text(),
                                theme::font(Family::Chrome, Role::Body),
                            ))
                            .child(
                                mono(view.origin(), theme::text_faint())
                                    .text_size(theme::font(Family::Chrome, Role::Meta)),
                            ),
                    )
                    // What Ubiq may do to this source, visible without opening anything: a row
                    // that does not say it is one the reader has to remember for.
                    .child(
                        div()
                            .flex_none()
                            .text_size(theme::font(Family::Chrome, Role::Meta))
                            .text_color(theme::text_faint())
                            .child(SharedString::from(access)),
                    )
                    .children(filter.map(|input| {
                        framed_active(theme::border(), input_on(input, window, cx))
                            .h(px(26.))
                            .w(px(180.))
                            .items_center()
                            .child(Input::new(input).appearance(false))
                    }))
                    .children(word.map(|word| {
                        div()
                            .flex_none()
                            .text_size(theme::font(Family::Chrome, Role::Meta))
                            .text_color(colour)
                            .child(SharedString::from(word))
                    }))
                    .child(icon_button(
                        crate::ui::eid("project-kb-remove", id),
                        IconName::Close,
                        false,
                        cx.listener(move |this, _, _, cx| this.remove_kb_source(id, cx)),
                    ))
                    .into_any_element()
            })
            .collect(),
        _ => vec![
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_faint())
                .child("No sources configured. Add one below.")
                .into_any_element(),
        ],
    };

    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(heading(
            "Knowledge base",
            "The sources this project's documents are read from. A folder is read where it lies; a \
             git repository is cloned and kept up to date by Ubiq; a wiki is Ubiq's own, and starts \
             empty. The filter beside each one limits what its tree shows.",
        ))
        .child(div().flex().flex_col().children(rows))
        .child(
            div().flex().items_center().pt_3().child(primary_button(
                "project-kb-add-source",
                Some(IconName::Plus),
                "Add source",
                cx.listener(|this, _, window, cx| this.open_kb_source_form(window, cx)),
            )),
        )
        .into_any_element()
}

/// The read-only / read-write marker one settings row carries.
///
/// Read through [`KbSource::is_writable`] rather than off `access`, so the row cannot disagree with
/// what the host will actually allow: a wiki is writable whatever `access` was last saved as.
fn access_word(source: &ubiq_proto::kb::KbSource) -> &'static str {
    match source.is_writable() {
        true => "read-write",
        false => "read-only",
    }
}

/// A source's state as the settings row says it: a colour from the status group, and the word that
/// colour is about. `Ready` says nothing — every row would carry it.
fn state_line(state: &KbSourceState) -> (Option<String>, Rgba) {
    match state {
        KbSourceState::Ready => (None, theme::text_faint()),
        KbSourceState::Pending => (Some("pending".into()), theme::text_faint()),
        KbSourceState::Syncing { detail } => (Some(detail.clone()), theme::warning()),
        KbSourceState::Failed { error } => (Some(error.clone()), theme::danger()),
    }
}

fn general(app: &AppState, window: &Window, cx: &mut Context<AppState>, form: Form) -> AnyElement {
    let picked = colour_of(app, form);
    let colour = picked.swatch;
    let custom = picked.custom;
    let about = form_about(app, form).read(cx).value();
    let used = about.chars().count();
    let current = current_rgba(app, form);
    let prefix = form.prefix();
    let name_input = form_name(app, form);
    let about_input = form_about(app, form);
    let path_note = match form {
        Form::Sink => "Set when the project was opened. Move it from the project switcher.",
        Form::Live => "Set when the folder was chosen. It cannot be changed here.",
    };

    let swatches: Vec<AnyElement> = (0..theme::project_colour_count())
        .map(|index| {
            let mut swatch = div()
                .id(ElementId::Name(format!("{prefix}-swatch-{index}").into()))
                .size(px(22.))
                .flex_none()
                .cursor_pointer()
                .bg(theme::project_colour(index));
            if custom.is_none() && index == colour {
                swatch = swatch.border_2().border_color(theme::text());
            }
            swatch
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_sink_project_colour(index, window, cx)
                }))
                .into_any_element()
        })
        .collect();

    let label = match custom {
        Some(rgb) => hex_string(rgb),
        None => colour_name(colour).to_string(),
    };

    let mut colour_block = div()
        .flex()
        .flex_col()
        .gap_1p5()
        .py_3()
        .border_b_1()
        .border_color(theme::border())
        .child(label_line(
            "Color",
            "Tints the title chip, the active rails and this project's icon, so windows \
             stay distinguishable when several projects are open.",
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(swatches)
                .child(icon_button(
                    ElementId::Name(format!("{prefix}-custom-colour").into()),
                    IconName::Palette,
                    picked.picker_open || custom.is_some(),
                    cx.listener(|this, _, window, cx| this.toggle_sink_colour_picker(window, cx)),
                ))
                .child(
                    div()
                        .size(px(22.))
                        .flex_none()
                        .bg(current)
                        .border_1()
                        .border_color(theme::border()),
                )
                .child(
                    mono(label, theme::text_muted())
                        .text_size(theme::font(Family::Chrome, Role::Meta)),
                ),
        );
    if picked.picker_open {
        colour_block = colour_block.child(colour_picker(app, window, cx, form));
    }

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .py_3()
                .border_b_1()
                .border_color(theme::border())
                .child(label_line(
                    "Project name",
                    "Shown in the title chip, the project switcher and every agent prompt.",
                ))
                .child(
                    framed_active(theme::border(), input_on(name_input, window, cx))
                        .h(px(30.))
                        .items_center()
                        .child(Input::new(name_input).appearance(false)),
                ),
        )
        .child(colour_block)
        .children((form == Form::Live).then(|| modes_block(app, cx)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .py_3()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(label_line(
                            "Description",
                            "Given to every agent as project context. Two lines about what this \
                             codebase is and what matters in it beat a paragraph of history.",
                        ))
                        .child(
                            mono(format!("{used}/{PROJECT_ABOUT_LIMIT}"), theme::text_faint())
                                .text_size(theme::font(Family::Chrome, Role::Meta)),
                        ),
                )
                .child(
                    framed_active(theme::border(), textarea_on(about_input, window, cx))
                        .p_2()
                        .child(
                            Textarea::new(about_input)
                                .appearance(false)
                                .bordered(false)
                                .w_full()
                                .text_size(theme::font(Family::Chrome, Role::Body)),
                        ),
                ),
        )
        .child(setting_row(
            "Project path",
            path_note,
            framed_active(
                theme::border(),
                input_on(&app.project_path_input, window, cx),
            )
            .h(px(30.))
            .w(px(320.))
            .items_center()
            .child(
                Input::new(&app.project_path_input)
                    .appearance(false)
                    .readonly(true)
                    .text_size(theme::font(Family::Chrome, Role::Body)),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            "Rail initials",
            "Up to three letters for the rail's badge, in place of the name's own first letter. \
             Empty leaves it derived from the name.",
            framed_active(
                theme::border(),
                input_on(&app.project_initials_input, window, cx),
            )
            .h(px(30.))
            .w(px(64.))
            .items_center()
            .child(
                Input::new(&app.project_initials_input)
                    .appearance(false)
                    .text_size(theme::font(Family::Chrome, Role::Body)),
            )
            .into_any_element(),
        ))
        .children(form_project(app, form, cx).and_then(|project| index_row(app, project, cx)))
        .children(
            form_project(app, form, cx)
                .and_then(|project| search_excludes_row(app, project, window, cx)),
        )
        .children(form_project(app, form, cx).and_then(|project| repos_row(app, project, cx)))
        .into_any_element()
}

/// Which rail modes this project shows: every mode as its own icon, lit when it is on screen.
/// The last one lit cannot be turned off, so the rail is never empty.
fn modes_block(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let tiles: Vec<AnyElement> = RailMode::every()
        .map(|mode| {
            let on = app.mode_enabled(mode, cx);
            let (fg, bg) = if on {
                (theme::accent(), theme::accent_soft())
            } else {
                (theme::text_faint(), theme::surface())
            };
            div()
                .id(ElementId::Name(
                    format!("project-mode-{}", mode.label().to_lowercase()).into(),
                ))
                .w(px(64.))
                .h(px(52.))
                .flex()
                .flex_none()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .bg(bg)
                .border_1()
                .border_color(if on { theme::accent() } else { theme::border() })
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(
                    Icon::new(mode_icon(mode))
                        .with_size(Size::Medium)
                        .text_color(fg),
                )
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Micro))
                        .text_color(fg)
                        .child(SharedString::from(mode.label())),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_mode(mode, cx)))
                .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .py_3()
        .border_b_1()
        .border_color(theme::border())
        .child(label_line(
            "Modes",
            "Which destinations this project shows in the rail. The last one on cannot be \
             turned off.",
        ))
        .child(div().flex().flex_wrap().gap_2().children(tiles))
        .into_any_element()
}

/// The kit's HSV surface, wired to whichever copy of the form is being drawn.
///
/// The control itself is `kit::colour_picker` — project settings is its first caller and the theme
/// editor is the second, which is why it is no longer written here.
fn colour_picker(
    app: &AppState,
    window: &Window,
    cx: &mut Context<AppState>,
    form: Form,
) -> AnyElement {
    let picked = colour_of(app, form);
    let hex_input = form_hex(app, form);
    let view = cx.entity();
    crate::ui::kit::colour_picker(
        form.prefix(),
        picked.hue,
        picked.sat,
        picked.val,
        current_rgba(app, form),
        hex_input,
        input_on(hex_input, window, cx),
        hsv(&view, |this, hue, sat, val, window, cx| {
            this.set_sink_project_hsv(hue, sat, val, window, cx)
        }),
    )
}

fn current_rgba(app: &AppState, form: Form) -> Rgba {
    let picked = colour_of(app, form);
    match picked.custom {
        Some(rgb) => theme::rgba_of(rgb),
        None => theme::project_colour(picked.swatch),
    }
}

/// Where this project's folder actually is: on this machine, or behind a drone on another one.
///
/// **Nothing here dials.** Saving writes `runs_on` and stops; the drone is launched or adopted
/// when the project is next opened — `AppState::ensure_drone` — which is the moment there is
/// something for one to serve. That also means a machine that is asleep when this is saved costs
/// nothing: the row stays local until an open asks for it.
///
/// The record's three states are the three the form has: runs here, on a drone, and — when the
/// pill says drone but no profile is picked yet — nothing said at all.
fn remote(app: &AppState, cx: &mut Context<AppState>, form: Form) -> AnyElement {
    let Some(project) = form_project(app, form, cx) else {
        return div()
            .text_size(theme::font(Family::Chrome, Role::Label))
            .text_color(theme::text_faint())
            .child("Where a project lives is a record's, not a folder's — name this folder first.")
            .into_any_element();
    };
    let field = match form {
        Form::Sink => app.sink.project.drone.clone(),
        Form::Live => app
            .workbench
            .project_settings
            .as_ref()
            .map(|settings| settings.drone.clone())
            .unwrap_or_default(),
    };
    let profiles = app.workbench.settings.host.ssh_profiles.clone();
    let prefix = form.prefix();

    let where_row = setting_row(
        "Where this folder is",
        "A project that runs on a drone keeps its row here — the name, the colour and this \
         setting are this machine's. What the drone contributes is the folder and the terminals \
         in it, and a drone that is not answering leaves the row saying so rather than taking it \
         away.",
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(choice_pill(
                ElementId::Name(format!("{prefix}-runs-here").into()),
                "Runs here",
                !field.on_drone,
                cx.listener(|this, _, _, cx| this.set_project_on_drone(false, cx)),
            ))
            .child(choice_pill(
                ElementId::Name(format!("{prefix}-runs-on-drone").into()),
                "On a drone",
                field.on_drone,
                cx.listener(|this, _, _, cx| this.set_project_on_drone(true, cx)),
            ))
            .into_any_element(),
    );

    let mut rows = div().flex().flex_col().child(where_row);

    if field.on_drone {
        let picker: AnyElement = if profiles.is_empty() {
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_faint())
                .child("No SSH profiles yet \u{2014} add one in application settings.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .flex_wrap()
                .children(profiles.iter().map(|profile| {
                    let id = profile.id;
                    choice_pill(
                        ElementId::Name(format!("{prefix}-drone-profile-{id}").into()),
                        profile.name.clone(),
                        field.profile == Some(id),
                        cx.listener(move |this, _, _, cx| this.set_project_drone_profile(id, cx)),
                    )
                }))
                .into_any_element()
        };
        rows = rows.child(setting_row(
            "Reached through",
            "One of the saved SSH profiles. The profile says where and how to connect; the \
             secret stays the host's and never reaches this dialog.",
            picker,
        ));

        rows = rows.child(setting_row(
            "Folder on that machine",
            "An absolute path as that machine spells it \u{2014} it is never resolved here, which \
             is why no folder chooser opens for it.",
            div()
                .w(px(280.))
                .flex_none()
                .child(Input::new(&app.project_remote_root_input).appearance(false))
                .into_any_element(),
        ));

        let preset_pill = |id: &str, label: &str, want: DronePreset| {
            choice_pill(
                ElementId::Name(format!("{prefix}-drone-{id}").into()),
                label.to_string(),
                field.preset == want,
                cx.listener(move |this, _, _, cx| this.set_project_drone_preset(want, cx)),
            )
        };
        rows = rows.child(setting_row(
            "Lifetime",
            "Attached dies with this window's connection. Session survives a dropped link for ten \
             minutes. Managed keeps running until it is stopped, which is what a long build or a \
             harness left thinking wants.",
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .flex_wrap()
                .child(preset_pill("attached", "Attached", DronePreset::Attached))
                .child(preset_pill("session", "Session", DronePreset::Session))
                .child(preset_pill("managed", "Managed", DronePreset::Managed))
                .into_any_element(),
        ));
    }

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(heading(
            "Remote",
            "The machine this project's folder is on, and the drone that serves it.",
        ))
        .child(rows)
        .child(div().flex().justify_end().pt_3().child(primary_button(
            ElementId::Name(format!("{prefix}-drone-save").into()),
            None,
            "Save",
            cx.listener(move |this, _, _, cx| this.save_project_drone(project, cx)),
        )))
        .into_any_element()
}

/// The task board's lanes: every one of them, with what this project has said about it.
///
/// Every lane is listed, hidden ones included — a page that dropped the lanes it had hidden would
/// be a page with no way back. The two switches are independent facets rather than a mode: a lane
/// can be hidden, can shut itself when it is empty, or both.
///
/// Sent on the click, the same rule `index_row`'s pills follow — see `AppState::set_project_lanes`.
/// A lane preference hangs off a record, so the Sink form and the Create mode draw the explanation
/// instead: there is no project for a lane to belong to yet.
fn tasks(app: &AppState, window: &Window, cx: &mut Context<AppState>, form: Form) -> AnyElement {
    let heading_block = heading(
        "Tasks",
        "Which lanes this project's board draws, and which of them get out of the way when they \
         are empty.",
    );

    let Some(project) = form_project(app, form, cx) else {
        return div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading_block)
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Label))
                    .text_color(theme::text_faint())
                    .child("Lanes belong to a project in the catalogue — name this folder first.")
                    .into_any_element(),
            )
            .into_any_element();
    };

    let Some(record) = WindowRegistry::read(cx)
        .project(project)
        .map(|snapshot| snapshot.record.clone())
    else {
        return div().child(heading_block).into_any_element();
    };

    let rows: Vec<AnyElement> = Status::all()
        .into_iter()
        .map(|status| {
            let pref = record.lane(status);
            let key = status as u32;
            let name = status.label();
            let mut title: String = name.to_string();
            if let Some(first) = title.get_mut(0..1) {
                first.make_ascii_uppercase();
            }

            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_6()
                .py_2p5()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .size(px(7.))
                                .flex_none()
                                .rounded_full()
                                .bg(status_colour(status)),
                        )
                        .child(
                            div()
                                .text_size(theme::font(Family::Chrome, Role::Body))
                                .text_color(if pref.hidden {
                                    theme::text_faint()
                                } else {
                                    theme::text()
                                })
                                .child(SharedString::from(title)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1p5()
                        .child(toggle_pill(
                            ("project-lane-shown", key),
                            "Shown",
                            theme::accent(),
                            !pref.hidden,
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_lane_hidden(project, status, cx)
                            }),
                        ))
                        .child(toggle_pill(
                            ("project-lane-collapse", key),
                            "Shut when empty",
                            theme::accent(),
                            pref.collapse_when_empty,
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_lane_collapse(project, status, cx)
                            }),
                        )),
                )
                .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(heading_block)
        .children(rows)
        .child(
            div()
                .pt_2()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(
                    "A hidden lane still holds whatever work is in it. Nothing is deleted and \
                     nothing is moved — the board simply stops drawing it.",
                ),
        )
        .children(mission_term_row(app, project, window, cx))
        .into_any_element()
}

fn documentation() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(heading(
            "Documentation",
            "The files a new agent is handed as the project's own briefing.",
        ))
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text_muted())
                .child("Four documents in the fixture. A real project lists what it indexed."),
        )
        .into_any_element()
}

fn integrations(
    app: &AppState,
    form: Form,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(heading(
            "Integrations",
            "MCP servers and accounts this project adds on top of the application defaults.",
        ))
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text_muted())
                .child("One integration in the fixture. Wiring it is the host's."),
        )
        .children(project_profiles(app, form, window, cx))
        .into_any_element()
}

/// The setups written inside this project: offered when a start is aimed here, and nowhere else.
///
/// The app-wide settings screen lists the global profiles and never these — "visible only in the
/// project they were created in" is the ask, and a second listing of them under Settings would
/// contradict it. A profile of the same name as a global one shadows it here, which is the rule
/// the host resolves a launch by.
///
/// Absent entirely for a folder that is not yet a project in the catalogue: a profile is filed
/// under a project id, so there is nowhere to put one until the project exists.
fn project_profiles(
    app: &AppState,
    form: Form,
    window: &Window,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    let _ = window;
    let project = form_project(app, form, cx)?;
    let profiles: Vec<ProfileInfo> = app
        .workbench
        .settings
        .project_profiles
        .iter()
        .filter(|it| it.project == Some(project))
        .cloned()
        .collect();

    let rows: Vec<AnyElement> = profiles
        .iter()
        .map(|profile| {
            let edit = profile.clone();
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .py_1()
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text())
                        .child(SharedString::from(format!(
                            "{} \u{2014} {}",
                            profile.id, profile.agent_type
                        ))),
                )
                .child(ghost_button(
                    ElementId::Name(format!("project-profile-{}-edit", profile.id).into()),
                    None,
                    "Edit",
                    cx.listener(move |this, _, window, cx| {
                        this.open_profile_form(Some(edit.clone()), None, window, cx)
                    }),
                ))
                .into_any_element()
        })
        .collect();

    Some(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_size(theme::font(Family::Chrome, Role::Label))
                            .text_color(theme::text_muted())
                            .child("Profiles"),
                    )
                    .child(ghost_button(
                        "project-profile-add",
                        None,
                        "Add profile",
                        cx.listener(move |this, _, window, cx| {
                            this.open_profile_form(None, Some(project), window, cx)
                        }),
                    )),
            )
            .children(rows)
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Meta))
                    .text_color(theme::text_faint())
                    .child(
                        "A setup saved here is offered when an agent starts in this project, and \
                         is not listed in the application's own settings. Forgetting the project \
                         deletes it.",
                    ),
            )
            .into_any_element(),
    )
}

fn footer(app: &AppState, form: Form, cx: &mut Context<AppState>) -> AnyElement {
    let prefix = form.prefix();
    let (status, cancel, confirm) = match form {
        Form::Sink => {
            let name = app.sink_project_name.read(cx).value();
            let about = app.sink_project_about.read(cx).value();
            let dirty = name.as_ref() != PROJECT_NAME
                || about.as_ref() != PROJECT_ABOUT
                || app.sink.project.colour.swatch != PROJECT_COLOUR
                || app.sink.project.colour.custom.is_some();
            (
                if dirty {
                    "Unsaved changes"
                } else {
                    "No unsaved changes"
                },
                ghost_button(
                    ElementId::Name(format!("{prefix}-cancel").into()),
                    None,
                    "Cancel",
                    cx.listener(|this, _, window, cx| this.reset_sink_project(window, cx)),
                )
                .into_any_element(),
                primary_button(
                    ElementId::Name(format!("{prefix}-save").into()),
                    None,
                    "Save changes",
                    |_, _, _| {},
                )
                .into_any_element(),
            )
        }
        Form::Live => {
            let creating = matches!(
                app.workbench.project_settings.as_ref().map(|s| &s.mode),
                Some(ProjectSettingsMode::Create { .. })
            );
            // Editing a temporary project is its first real save: the dialog is the same "Edit"
            // mode, but the button should still say what is about to happen.
            let temporary = match app.workbench.project_settings.as_ref().map(|s| &s.mode) {
                Some(ProjectSettingsMode::Edit { project }) => WindowRegistry::read(cx)
                    .project(*project)
                    .is_some_and(|entry| entry.record.temporary),
                _ => false,
            };
            let label = if creating || temporary {
                "Create"
            } else {
                "Save changes"
            };
            (
                if creating {
                    "New project"
                } else {
                    "The path cannot be changed"
                },
                ghost_button(
                    ElementId::Name(format!("{prefix}-cancel").into()),
                    None,
                    "Cancel",
                    cx.listener(|this, _, _, cx| this.close_project_settings(cx)),
                )
                .into_any_element(),
                primary_button(
                    ElementId::Name(format!("{prefix}-save").into()),
                    None,
                    label,
                    cx.listener(|this, _, _, cx| this.commit_project_settings(cx)),
                )
                .into_any_element(),
            )
        }
    };

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_2()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_faint())
                .child(SharedString::from(status)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(cancel)
                .child(confirm),
        )
        .into_any_element()
}

fn label_line(label: &str, note: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w(px(0.))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text())
                        .child(SharedString::from(label.to_string())),
                )
                .when(label == "Description", |this| {
                    this.child(
                        div()
                            .text_size(theme::font(Family::Chrome, Role::Meta))
                            .text_color(theme::text_faint())
                            .child("optional"),
                    )
                }),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(SharedString::from(note.to_string())),
        )
        .into_any_element()
}

fn colour_name(index: usize) -> &'static str {
    match index {
        0 => "blue",
        1 => "violet",
        2 => "teal",
        3 => "gold",
        4 => "pink",
        5 => "green",
        6 => "red",
        7 => "orange",
        8 => "lime",
        9 => "cyan",
        10 => "indigo",
        11 => "magenta",
        12 => "brown",
        13 => "slate",
        14 => "olive",
        15 => "rose",
        _ => "custom",
    }
}
