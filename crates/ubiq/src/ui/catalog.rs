//! The skills and MCP servers catalog: two settings pages, each drawn twice — for the application's
//! layer in Settings and for one project's own layer in its dialog — and the four modals they
//! raise (the repository browser, the "Add source" question, the MCP form and the registry
//! search).
//!
//! One component per page with the layer as a parameter, in the shape of [`crate::ui::tools`]: the
//! rows and the actions are the same, and only which list is behind them differs. A project page
//! adds what it inherits below its own rows, greyed and without actions — a global is edited
//! where it was written.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, ElementId, Focusable, IntoElement, ParentElement, SharedString, Styled,
    Window, div, px,
};
use gpui_component::IconName;
use gpui_component::input::{Input, InputState, Textarea, TextareaState};
use ubiq_proto::catalog::{
    CatalogMcp, McpKind, McpParamKind, RegistryMcpInfo, RemoteSkillInfo, SkillInfo,
    SkillOriginInfo, SkillSourceInfo, SkillSourceKindInfo,
};
use ubiq_proto::ids::ProjectId;

use crate::app::AppState;
use crate::state::Layer;
use crate::state::catalog::{SkillBrowse, kind_label, mcp_ready, mcp_summary};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{
    badge, choice_pill, elided, field, ghost_button, heading, label_block, modal, modal_note,
    primary_button, section_label,
};

/// The most result rows a browser draws. A source with more says so and asks for a narrower query.
const RESULT_CAP: usize = 200;

// ── Shared furniture ────────────────────────────────────────────────

fn meta(text: impl Into<SharedString>, colour: gpui::Rgba) -> AnyElement {
    div()
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(colour)
        .child(text.into())
        .into_any_element()
}

/// What the host last refused, dismissible. The account section's banner shape.
fn error_banner(error: &str, prefix: &'static str, cx: &mut Context<AppState>) -> AnyElement {
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
            ElementId::Name(format!("{prefix}-error-dismiss").into()),
            None,
            "Dismiss",
            cx.listener(|this, _, _, cx| this.dismiss_catalog_error(cx)),
        ))
        .into_any_element()
}

/// A page's title row: a group label on the left, its actions on the right.
fn group_head(label: &str, actions: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .pt_2()
        .child(section_label(label))
        .child(div().flex().items_center().gap_1().children(actions))
        .into_any_element()
}

/// A folder or repository row: what it is, and the one action on it.
fn path_row(
    id: ElementId,
    text: String,
    remove_id: ElementId,
    on_remove: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .py_1()
        .child(elided(
            id,
            text,
            theme::text_muted(),
            theme::font(Family::Chrome, Role::Label),
        ))
        .child(ghost_button(remove_id, None, "Remove", on_remove))
        .into_any_element()
}

fn origin_badge(origin: &SkillOriginInfo) -> AnyElement {
    match origin {
        SkillOriginInfo::Installed { .. } => badge("Installed", theme::accent()).into_any_element(),
        SkillOriginInfo::Linked { .. } => badge("Linked", theme::info()).into_any_element(),
        SkillOriginInfo::Folder { .. } => badge("Folder", theme::text_faint()).into_any_element(),
    }
}

fn kind_colour(kind: McpKind) -> gpui::Rgba {
    match kind {
        McpKind::Stdio => theme::text_faint(),
        McpKind::Http => theme::accent(),
        McpKind::Sse => theme::info(),
    }
}

/// One labelled single-line box.
fn line_field(
    label: &str,
    note: &str,
    input: &gpui::Entity<InputState>,
    window: &Window,
    cx: &gpui::App,
) -> AnyElement {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
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

/// One labelled multi-line box.
fn area_field(
    label: &str,
    note: &str,
    input: &gpui::Entity<TextareaState>,
    window: &Window,
    cx: &gpui::App,
) -> AnyElement {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(label_block(label, note))
        .child(
            field(theme::border(), focused).p_2().w_full().child(
                Textarea::new(input)
                    .appearance(false)
                    .bordered(false)
                    .w_full()
                    .text_size(theme::font(Family::Chrome, Role::Body)),
            ),
        )
        .into_any_element()
}

// ── Skills ──────────────────────────────────────────────────────────

/// The Skills page for one layer: `None` is the application's, `Some` a project's.
pub fn skills_page(
    app: &AppState,
    scope: Option<ProjectId>,
    prefix: &'static str,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let catalog = &app.workbench.settings.catalog;
    let layer = catalog.layer(scope).cloned().unwrap_or_default();

    let mut rows = vec![heading(
        "Skills",
        match scope {
            None => {
                "A skill is a folder with a SKILL.md that tells an agent when and how to use it. \
                 Point at one, scan a folder of them, or install from a repository; an agent \
                 definition or a start then ticks the ones it should carry."
            }
            Some(_) => {
                "Skills only agents started in this project can carry, on top of the \
                 application\u{2019}s own below. One here with the same id as an application \
                 skill takes its place."
            }
        },
    )];
    if let Some(error) = catalog.error.as_deref() {
        rows.push(error_banner(error, prefix, cx));
    }

    rows.push(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(ghost_button(
                ElementId::Name(format!("{prefix}-add-folder").into()),
                Some(IconName::Plus),
                "Add skill folder\u{2026}",
                cx.listener(move |this, _, window, cx| {
                    this.browse_skill_folder(scope, false, window, cx)
                }),
            ))
            .child(ghost_button(
                ElementId::Name(format!("{prefix}-add-directory").into()),
                Some(IconName::Folder),
                "Add skills directory\u{2026}",
                cx.listener(move |this, _, window, cx| {
                    this.browse_skill_folder(scope, true, window, cx)
                }),
            ))
            .child(ghost_button(
                ElementId::Name(format!("{prefix}-browse").into()),
                Some(IconName::Search),
                "Browse repositories\u{2026}",
                cx.listener(move |this, _, window, cx| this.open_skill_browse(scope, window, cx)),
            ))
            .into_any_element(),
    );

    if layer.skills.is_empty() {
        rows.push(meta("No skills yet.", theme::text_faint()));
    } else {
        rows.push(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .children(
                    layer
                        .skills
                        .iter()
                        .map(|skill| skill_row(prefix, scope, skill, cx)),
                )
                .into_any_element(),
        );
    }

    if !layer.skill_folders.is_empty() {
        rows.push(group_head("Scanned folders", Vec::new()));
        rows.push(
            div()
                .flex()
                .flex_col()
                .children(layer.skill_folders.iter().enumerate().map(|(i, path)| {
                    let drop = path.clone();
                    path_row(
                        ElementId::Name(format!("{prefix}-folder-{i}").into()),
                        path.clone(),
                        ElementId::Name(format!("{prefix}-folder-{i}-remove").into()),
                        cx.listener(move |this, _, _, cx| {
                            this.remove_skill_folder(scope, drop.clone(), cx)
                        }),
                    )
                }))
                .into_any_element(),
        );
    }

    match scope {
        None => rows.push(sources_block(&layer.sources, prefix, cx)),
        Some(_) => {
            let own = layer.skills;
            let globals = &catalog.global.skills;
            rows.push(group_head("From the application", Vec::new()));
            if globals.is_empty() {
                rows.push(meta("The application has no skills.", theme::text_faint()));
            }
            rows.extend(globals.iter().map(|skill| {
                let shadowed = own.iter().any(|it| it.id == skill.id);
                inherited_skill_row(prefix, skill, shadowed)
            }));
        }
    }

    div()
        .flex()
        .flex_col()
        .gap_3()
        .w_full()
        .children(rows)
        .into_any_element()
}

fn skill_row(
    prefix: &'static str,
    scope: Option<ProjectId>,
    skill: &SkillInfo,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = skill.id.clone();
    // A skill found by scanning a folder leaves with the folder, which is its own row.
    let removable = !matches!(skill.origin, SkillOriginInfo::Folder { .. });
    let detail = skill
        .description
        .clone()
        .filter(|it| !it.is_empty())
        .or_else(|| match &skill.origin {
            SkillOriginInfo::Installed { url } => url.clone(),
            SkillOriginInfo::Linked { path } | SkillOriginInfo::Folder { path } => {
                Some(path.clone())
            }
        });

    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .py_1()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap_0p5()
                .min_w(px(0.))
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
                                .child(SharedString::from(skill.id.clone())),
                        )
                        .child(origin_badge(&skill.origin))
                        .children(
                            skill
                                .name
                                .as_deref()
                                .filter(|name| !name.is_empty() && *name != skill.id)
                                .map(|name| meta(format!("\u{2014} {name}"), theme::text_muted())),
                        ),
                )
                .children(detail.map(|detail| {
                    elided(
                        ElementId::Name(format!("{prefix}-skill-{}-detail", skill.id).into()),
                        detail,
                        theme::text_muted(),
                        theme::font(Family::Chrome, Role::Meta),
                    )
                })),
        )
        .children(removable.then(|| {
            ghost_button(
                ElementId::Name(format!("{prefix}-skill-{}-remove", skill.id).into()),
                None,
                "Remove",
                cx.listener(move |this, _, _, cx| this.remove_skill(scope, id.clone(), cx)),
            )
        }))
        .into_any_element()
}

/// One application skill as a project page shows it: what it is, and no action.
fn inherited_skill_row(prefix: &'static str, skill: &SkillInfo, shadowed: bool) -> AnyElement {
    let note = match shadowed {
        true => " \u{2014} shadowed by this project\u{2019}s own",
        false => "",
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .py_1()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(if shadowed {
                    theme::text_faint()
                } else {
                    theme::text()
                })
                .child(SharedString::from(format!("{}{note}", skill.id))),
        )
        .child(origin_badge(&skill.origin))
        .children(
            skill
                .description
                .clone()
                .filter(|it| !it.is_empty())
                .map(|description| {
                    elided(
                        ElementId::Name(
                            format!("{prefix}-inherited-{}-description", skill.id).into(),
                        ),
                        description,
                        theme::text_muted(),
                        theme::font(Family::Chrome, Role::Meta),
                    )
                }),
        )
        .into_any_element()
}

/// The places skills are searched for: label and address, and the way to add or drop one.
fn sources_block(
    sources: &[SkillSourceInfo],
    prefix: &'static str,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let mut rows = vec![group_head(
        "Sources",
        vec![
            ghost_button(
                ElementId::Name(format!("{prefix}-add-source").into()),
                Some(IconName::Plus),
                "Add source\u{2026}",
                cx.listener(|this, _, window, cx| this.open_add_source(window, cx)),
            )
            .into_any_element(),
        ],
    )];
    if sources.is_empty() {
        rows.push(meta("No sources.", theme::text_faint()));
    }
    rows.extend(sources.iter().map(|source| {
        let drop = source.id.clone();
        path_row(
            ElementId::Name(format!("{prefix}-source-{}", source.id).into()),
            source_line(source),
            ElementId::Name(format!("{prefix}-source-{}-remove", source.id).into()),
            cx.listener(move |this, _, _, cx| this.remove_skill_source(drop.clone(), cx)),
        )
    }));
    div().flex().flex_col().children(rows).into_any_element()
}

/// `Label \u{b7} https://\u{2026}/repo \u{b7} path` — a source on one line.
fn source_line(source: &SkillSourceInfo) -> String {
    let label = source.label.clone().unwrap_or_else(|| source.id.clone());
    match &source.kind {
        SkillSourceKindInfo::Git { url, subpath, .. } => match subpath.as_deref() {
            Some(path) if !path.is_empty() => format!("{label} \u{b7} {url} \u{b7} {path}"),
            _ => format!("{label} \u{b7} {url}"),
        },
        SkillSourceKindInfo::Index { url } => format!("{label} \u{b7} {url}"),
    }
}

/// The repository browser: search the sources, install a result into the layer that asked.
pub fn skill_browse(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let catalog = &app.workbench.settings.catalog;
    let Some(browse) = catalog.browse.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let installed: Vec<String> = catalog
        .layer(browse.scope)
        .map(|layer| layer.skills.iter().map(|it| it.id.clone()).collect())
        .unwrap_or_default();

    let mut pills = vec![
        choice_pill(
            "skill-browse-source-all",
            "All",
            browse.source.is_none(),
            cx.listener(|this, _, _, cx| this.pick_skill_source(None, cx)),
        )
        .into_any_element(),
    ];
    pills.extend(catalog.global.sources.iter().map(|source| {
        let id = source.id.clone();
        choice_pill(
            ElementId::Name(format!("skill-browse-source-{}", source.id).into()),
            source.label.clone().unwrap_or_else(|| source.id.clone()),
            browse.source.as_deref() == Some(source.id.as_str()),
            cx.listener(move |this, _, _, cx| this.pick_skill_source(Some(id.clone()), cx)),
        )
        .into_any_element()
    }));

    let search_focused = app
        .catalog_inputs
        .skill_search
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let mut body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(
            "Skills the configured repositories offer. Installing copies one into the layer, so \
             it keeps working when the repository is gone.",
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    field(theme::border(), search_focused)
                        .h(px(30.))
                        .px_2()
                        .flex_1()
                        .child(Input::new(&app.catalog_inputs.skill_search).appearance(false)),
                )
                .child(ghost_button(
                    "skill-browse-search",
                    Some(IconName::Search),
                    "Search",
                    cx.listener(|this, _, _, cx| this.search_skills(cx)),
                )),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .children(pills),
        );

    for problem in &browse.problems {
        body = body.child(meta(problem.clone(), theme::warning()));
    }
    if browse.loading {
        body = body.child(meta("Searching\u{2026}", theme::text_faint()));
    } else if browse.results.is_empty() {
        body = body.child(meta("No skills found.", theme::text_faint()));
    }
    body = body.children(
        browse
            .results
            .iter()
            .take(RESULT_CAP)
            .enumerate()
            .map(|(i, result)| result_row(i, result, &browse, &installed, cx)),
    );
    if browse.results.len() > RESULT_CAP {
        body = body.child(meta(
            format!(
                "{} more \u{2014} narrow the search to see them.",
                browse.results.len() - RESULT_CAP
            ),
            theme::text_faint(),
        ));
    }

    let footer = ghost_button(
        "skill-browse-close",
        None,
        "Close",
        cx.listener(|this, _, _, cx| this.close_skill_browse(cx)),
    )
    .into_any_element();

    modal(
        "skill-browse",
        theme::accent(),
        "Browse skill repositories",
        body.into_any_element(),
        footer,
        crate::ui::dismiss(&view, Layer::SkillBrowse, |this, _, cx| {
            this.close_skill_browse(cx)
        }),
        window,
    )
}

fn result_row(
    index: usize,
    result: &RemoteSkillInfo,
    browse: &SkillBrowse,
    installed: &[String],
    cx: &mut Context<AppState>,
) -> AnyElement {
    let key = SkillBrowse::key(result);
    let busy = browse.installing.as_deref() == Some(key.as_str());
    let have = installed.contains(&result.id);
    let install = result.clone();
    let action = if have {
        meta("Installed", theme::success())
    } else if busy {
        meta("Installing\u{2026}", theme::text_faint())
    } else {
        ghost_button(
            ElementId::Name(format!("skill-browse-install-{index}").into()),
            Some(IconName::Plus),
            "Install",
            cx.listener(move |this, _, _, cx| this.install_skill(install.clone(), cx)),
        )
        .into_any_element()
    };

    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .py_1()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap_0p5()
                .min_w(px(0.))
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
                                .child(SharedString::from(result.id.clone())),
                        )
                        .child(meta(result.source.clone(), theme::text_faint())),
                )
                .children(result.description.clone().filter(|it| !it.is_empty()).map(
                    |description| {
                        elided(
                            ElementId::Name(
                                format!("skill-browse-result-{index}-description").into(),
                            ),
                            description,
                            theme::text_muted(),
                            theme::font(Family::Chrome, Role::Meta),
                        )
                    },
                )),
        )
        .child(action)
        .into_any_element()
}

/// "Add source": a repository address and, optionally, the path inside it to scan.
pub fn add_source(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let ready = !app
        .catalog_inputs
        .source_url
        .read(cx)
        .value()
        .trim()
        .is_empty();
    let body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(
            "A git repository Ubiq scans for SKILL.md files. It is fetched on the machine the \
             host runs on, when a search or an install asks.",
        ))
        .child(line_field(
            "Repository",
            "The clone address.",
            &app.catalog_inputs.source_url,
            window,
            cx,
        ))
        .child(line_field(
            "Path",
            "Only look under this folder of the repository. Empty scans all of it.",
            &app.catalog_inputs.source_subpath,
            window,
            cx,
        ))
        .into_any_element();
    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "skill-source-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.close_add_source(cx)),
        ))
        .child(
            primary_button(
                "skill-source-save",
                None,
                "Add",
                cx.listener(|this, _, _, cx| this.save_add_source(cx)),
            )
            .when(!ready, |button| button.opacity(0.5)),
        )
        .into_any_element();
    modal(
        "skill-source",
        theme::accent(),
        "Add source",
        body,
        footer,
        crate::ui::dismiss(&view, Layer::SkillSource, |this, _, cx| {
            this.close_add_source(cx)
        }),
        window,
    )
}

// ── MCP servers ─────────────────────────────────────────────────────

/// The MCP servers page for one layer: `None` is the application's, `Some` a project's.
pub fn mcp_page(
    app: &AppState,
    scope: Option<ProjectId>,
    prefix: &'static str,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let catalog = &app.workbench.settings.catalog;
    let layer = catalog.layer(scope).cloned().unwrap_or_default();

    let mut rows = vec![heading(
        "MCP servers",
        match scope {
            None => {
                "Servers an agent can be given beside Ubiq\u{2019}s own. A local one is a command \
                 the harness starts; a remote one is a URL. An agent definition or a start then \
                 ticks the ones it should carry."
            }
            Some(_) => {
                "Servers only agents started in this project can be given, on top of the \
                 application\u{2019}s own below. One here with the same id as an application \
                 server takes its place."
            }
        },
    )];
    if let Some(error) = catalog.error.as_deref() {
        rows.push(error_banner(error, prefix, cx));
    }
    rows.push(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(ghost_button(
                ElementId::Name(format!("{prefix}-add").into()),
                Some(IconName::Plus),
                "Add MCP server",
                cx.listener(move |this, _, window, cx| this.open_mcp_form(scope, None, window, cx)),
            ))
            .child(ghost_button(
                ElementId::Name(format!("{prefix}-registry").into()),
                Some(IconName::Search),
                "Search registry\u{2026}",
                cx.listener(move |this, _, window, cx| this.open_mcp_registry(scope, window, cx)),
            ))
            .into_any_element(),
    );

    if layer.mcps.is_empty() {
        rows.push(meta("No MCP servers yet.", theme::text_faint()));
    } else {
        rows.push(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .children(layer.mcps.iter().map(|mcp| mcp_row(prefix, scope, mcp, cx)))
                .into_any_element(),
        );
    }

    if scope.is_some() {
        let own = layer.mcps;
        let globals = &catalog.global.mcps;
        rows.push(group_head("From the application", Vec::new()));
        if globals.is_empty() {
            rows.push(meta(
                "The application has no MCP servers.",
                theme::text_faint(),
            ));
        }
        rows.extend(globals.iter().map(|mcp| {
            let shadowed = own.iter().any(|it| it.id == mcp.id);
            inherited_mcp_row(prefix, mcp, shadowed)
        }));
    }

    div()
        .flex()
        .flex_col()
        .gap_3()
        .w_full()
        .children(rows)
        .into_any_element()
}

fn mcp_row(
    prefix: &'static str,
    scope: Option<ProjectId>,
    mcp: &CatalogMcp,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let (edit, drop) = (mcp.clone(), mcp.id.clone());
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .py_1()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap_0p5()
                .min_w(px(0.))
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
                                .child(SharedString::from(mcp.id.clone())),
                        )
                        .child(badge(kind_label(mcp.kind), kind_colour(mcp.kind))),
                )
                .child(elided(
                    ElementId::Name(format!("{prefix}-mcp-{}-summary", mcp.id).into()),
                    mcp_summary(mcp),
                    theme::text_muted(),
                    theme::font(Family::Chrome, Role::Meta),
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(ghost_button(
                    ElementId::Name(format!("{prefix}-mcp-{}-edit", mcp.id).into()),
                    None,
                    "Edit",
                    cx.listener(move |this, _, window, cx| {
                        this.open_mcp_form(scope, Some(edit.clone()), window, cx)
                    }),
                ))
                .child(ghost_button(
                    ElementId::Name(format!("{prefix}-mcp-{}-remove", mcp.id).into()),
                    None,
                    "Remove",
                    cx.listener(move |this, _, _, cx| {
                        this.remove_catalog_mcp(scope, drop.clone(), cx)
                    }),
                )),
        )
        .into_any_element()
}

fn inherited_mcp_row(prefix: &'static str, mcp: &CatalogMcp, shadowed: bool) -> AnyElement {
    let note = match shadowed {
        true => " \u{2014} shadowed by this project\u{2019}s own",
        false => "",
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .py_1()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(if shadowed {
                    theme::text_faint()
                } else {
                    theme::text()
                })
                .child(SharedString::from(format!("{}{note}", mcp.id))),
        )
        .child(badge(kind_label(mcp.kind), kind_colour(mcp.kind)))
        .child(elided(
            ElementId::Name(format!("{prefix}-inherited-{}-summary", mcp.id).into()),
            mcp_summary(mcp),
            theme::text_muted(),
            theme::font(Family::Chrome, Role::Meta),
        ))
        .into_any_element()
}

/// The MCP form: Local or Remote, the fields of that kind, and a paste box that fills them.
pub fn mcp_form(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(form) = app.workbench.settings.catalog.form.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let inputs = &app.catalog_inputs;
    let editing = form.previous_id.is_some();
    let ready = app.mcp_form_record(cx).as_ref().is_some_and(mcp_ready) && !form.saving;
    let scope = form.scope;

    let kinds: Vec<AnyElement> = [
        ("Local", McpKind::Stdio, !form.remote),
        ("Remote", McpKind::Http, form.remote),
    ]
    .into_iter()
    .map(|(label, kind, active)| {
        choice_pill(
            ElementId::Name(format!("mcp-form-kind-{label}").into()),
            label,
            active,
            cx.listener(move |this, _, _, cx| this.pick_mcp_kind(kind, cx)),
        )
        .into_any_element()
    })
    .collect();
    let transports: Vec<AnyElement> = [("HTTP", McpKind::Http), ("SSE", McpKind::Sse)]
        .into_iter()
        .map(|(label, kind)| {
            choice_pill(
                ElementId::Name(format!("mcp-form-transport-{label}").into()),
                label,
                form.kind() == kind,
                cx.listener(move |this, _, _, cx| this.pick_mcp_kind(kind, cx)),
            )
            .into_any_element()
        })
        .collect();

    let mut body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(paste_block(app, &form, window, cx))
        .child(line_field(
            "Id",
            "What a definition or a start ticks it by. Letters, digits, dots, dashes and \
             underscores.",
            &inputs.mcp_id,
            window,
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Kind",
                    "Local starts a command on the host\u{2019}s machine; remote connects to a URL.",
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .children(kinds)
                        .when(form.remote, |row| {
                            row.child(meta("over", theme::text_faint()))
                                .children(transports)
                        }),
                ),
        );

    body = match form.remote {
        false => body
            .child(line_field(
                "Command",
                "The program to start \u{2014} a bare name or a path.",
                &inputs.mcp_command,
                window,
                cx,
            ))
            .child(area_field(
                "Arguments",
                "One per line.",
                &inputs.mcp_args,
                window,
                cx,
            ))
            .child(area_field(
                "Environment",
                "One KEY=VALUE per line.",
                &inputs.mcp_env,
                window,
                cx,
            )),
        true => body
            .child(line_field(
                "URL",
                "The server\u{2019}s endpoint.",
                &inputs.mcp_url,
                window,
                cx,
            ))
            .child(area_field(
                "Headers",
                "One Name: value per line \u{2014} where a token goes.",
                &inputs.mcp_headers,
                window,
                cx,
            )),
    };
    body = body.child(line_field(
        "Description",
        "Optional \u{2014} what the server is for, shown beside it in the pickers.",
        &inputs.mcp_description,
        window,
        cx,
    ));

    if !form.hints.is_empty() {
        body = body.child(hints_block(&form));
    }
    if let Some(error) = form.error.as_deref() {
        body = body.child(meta(error.to_string(), theme::danger()));
    }

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "mcp-form-registry",
            Some(IconName::Search),
            "Search registry\u{2026}",
            cx.listener(move |this, _, window, cx| this.open_mcp_registry(scope, window, cx)),
        ))
        .child(ghost_button(
            "mcp-form-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.close_mcp_form(cx)),
        ))
        .child(
            primary_button(
                "mcp-form-save",
                None,
                if form.saving {
                    "Saving\u{2026}"
                } else {
                    "Save"
                },
                cx.listener(|this, _, _, cx| this.save_mcp_form(cx)),
            )
            .when(!ready, |button| button.opacity(0.5)),
        )
        .into_any_element();

    modal(
        "mcp-form",
        theme::accent(),
        if editing {
            "Edit MCP server"
        } else {
            "Add MCP server"
        },
        body.into_any_element(),
        footer,
        crate::ui::dismiss(&view, Layer::McpForm, |this, _, cx| this.close_mcp_form(cx)),
        window,
    )
}

/// The paste box, its button, and the choice a paste of several servers leaves.
fn paste_block(
    app: &AppState,
    form: &crate::state::catalog::McpForm,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let mut block = div().flex().flex_col().gap_2().child(area_field(
        "Paste a configuration",
        "From another harness\u{2019}s settings \u{2014} JSON with mcpServers, servers or \
         context_servers, or a Codex TOML table. It fills the form below.",
        &app.catalog_inputs.mcp_paste,
        window,
        cx,
    ));
    block = block.child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(ghost_button(
                "mcp-form-fill",
                None,
                "Fill from paste",
                cx.listener(|this, _, _, cx| this.parse_mcp_paste(cx)),
            ))
            .children(
                form.parsing
                    .then(|| meta("Reading\u{2026}", theme::text_faint())),
            ),
    );
    if !form.picks.is_empty() {
        block = block
            .child(meta(
                format!(
                    "{} servers found \u{2014} pick one to fill the form.",
                    form.picks.len()
                ),
                theme::text_muted(),
            ))
            .children(form.picks.iter().enumerate().map(|(i, pick)| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .text_size(theme::font(Family::Chrome, Role::Body))
                                    .text_color(theme::text())
                                    .child(SharedString::from(pick.id.clone())),
                            )
                            .child(badge(kind_label(pick.kind), kind_colour(pick.kind)))
                            .child(elided(
                                ElementId::Name(format!("mcp-form-pick-{i}-summary").into()),
                                mcp_summary(pick),
                                theme::text_muted(),
                                theme::font(Family::Chrome, Role::Meta),
                            )),
                    )
                    .child(ghost_button(
                        ElementId::Name(format!("mcp-form-pick-{i}").into()),
                        None,
                        "Use",
                        cx.listener(move |this, _, _, cx| this.use_mcp_pick(i, cx)),
                    ))
                    .into_any_element()
            }))
            .child(div().child(ghost_button(
                "mcp-form-pick-all",
                Some(IconName::Plus),
                format!("Add all {}", form.picks.len()),
                cx.listener(|this, _, _, cx| this.add_all_mcp_picks(cx)),
            )));
    }
    block.into_any_element()
}

/// What a registry pick still needs a value for: the variables and headers it left empty.
fn hints_block(form: &crate::state::catalog::McpForm) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(section_label("Needs a value"))
        .children(form.hints.iter().map(|hint| {
            let place = match hint.kind {
                McpParamKind::Env => "environment",
                McpParamKind::Header => "header",
                McpParamKind::Arg => "argument",
            };
            let mut line = format!("{} ({place}", hint.name);
            if hint.required {
                line.push_str(", required");
            }
            if hint.secret {
                line.push_str(", secret");
            }
            line.push(')');
            if let Some(description) = hint.description.as_deref().filter(|it| !it.is_empty()) {
                line.push_str(" \u{2014} ");
                line.push_str(description);
            }
            meta(
                line,
                if hint.required {
                    theme::warning()
                } else {
                    theme::text_muted()
                },
            )
        }))
        .into_any_element()
}

/// The MCP registry search: a query, the servers it finds, and the ways to run each.
pub fn mcp_registry(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(registry) = app.workbench.settings.catalog.registry.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let focused = app
        .catalog_inputs
        .registry_search
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    let mut body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(
            "Servers the public MCP registry lists. Pick a way to run one and it fills the form \
             \u{2014} nothing is installed until you save.",
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    field(theme::border(), focused)
                        .h(px(30.))
                        .px_2()
                        .flex_1()
                        .child(Input::new(&app.catalog_inputs.registry_search).appearance(false)),
                )
                .child(ghost_button(
                    "mcp-registry-search",
                    Some(IconName::Search),
                    "Search",
                    cx.listener(|this, _, _, cx| this.search_mcp_registry(cx)),
                )),
        );
    if let Some(error) = registry.error.as_deref() {
        body = body.child(meta(error.to_string(), theme::danger()));
    }
    if registry.results.is_empty() && !registry.loading && registry.error.is_none() {
        body = body.child(meta("No servers found.", theme::text_faint()));
    }
    body = body.children(
        registry
            .results
            .iter()
            .enumerate()
            .map(|(i, server)| registry_row(i, server, cx)),
    );
    if registry.loading {
        body = body.child(meta("Searching\u{2026}", theme::text_faint()));
    } else if registry.next_cursor.is_some() {
        body = body.child(div().child(ghost_button(
            "mcp-registry-more",
            None,
            "Load more",
            cx.listener(|this, _, _, cx| this.more_mcp_registry(cx)),
        )));
    }

    let footer = ghost_button(
        "mcp-registry-close",
        None,
        "Close",
        cx.listener(|this, _, _, cx| this.close_mcp_registry(cx)),
    )
    .into_any_element();

    modal(
        "mcp-registry",
        theme::accent(),
        "Search the MCP registry",
        body.into_any_element(),
        footer,
        crate::ui::dismiss(&view, Layer::McpRegistry, |this, _, cx| {
            this.close_mcp_registry(cx)
        }),
        window,
    )
}

fn registry_row(index: usize, server: &RegistryMcpInfo, cx: &mut Context<AppState>) -> AnyElement {
    let title = server.title.clone().unwrap_or_else(|| server.name.clone());
    div()
        .flex()
        .flex_col()
        .gap_1()
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
                        .child(SharedString::from(title)),
                )
                .children(
                    server
                        .version
                        .clone()
                        .map(|version| meta(version, theme::text_faint())),
                ),
        )
        .children(
            server
                .description
                .clone()
                .filter(|it| !it.is_empty())
                .map(|description| {
                    elided(
                        ElementId::Name(format!("mcp-registry-{index}-description").into()),
                        description,
                        theme::text_muted(),
                        theme::font(Family::Chrome, Role::Meta),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1()
                .children(server.options.iter().enumerate().map(|(o, option)| {
                    ghost_button(
                        ElementId::Name(format!("mcp-registry-{index}-option-{o}").into()),
                        Some(IconName::Plus),
                        option.label.clone(),
                        cx.listener(move |this, _, window, cx| {
                            this.pick_registry_option(index, o, window, cx)
                        }),
                    )
                    .into_any_element()
                }))
                .when(server.options.is_empty(), |row| {
                    row.child(meta("No way to run it is listed.", theme::text_faint()))
                }),
        )
        .into_any_element()
}
