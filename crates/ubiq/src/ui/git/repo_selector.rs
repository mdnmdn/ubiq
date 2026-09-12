//! The repository selector on the Git chrome strip: which repository this view is showing, and
//! the list of the project's other repositories (submodules or nested ones) to switch to.

use gpui::{
    AnyElement, App, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, prelude::FluentBuilder,
    px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size, scroll::ScrollableElement};

use crate::app::AppState;
use crate::state::MenuId;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{UbiqIcon, mono, section_label};
use ubiq_proto::git::GitHead;

/// How wide the repo selector panel is.
const PANEL_WIDTH: f32 = 360.0;

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let open = app.workbench.open_menu == Some(MenuId::GitRepo);
    let current = current_repo_label(app, cx);
    let live = current.is_some();
    let label = current.unwrap_or_else(|| "not a repository".to_string());

    let mut trigger = div()
        .id("git-repo-selector")
        .relative()
        .h_full()
        .min_w(px(100.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .text_color(theme::text())
        .text_size(theme::font(Family::Chrome, Role::Body))
        .child(
            Icon::new(UbiqIcon::GitBranch)
                .with_size(Size::XSmall)
                .text_color(theme::text_muted()),
        )
        .child(mono(
            label,
            if live {
                theme::text()
            } else {
                theme::text_faint()
            },
        ))
        .when(live, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(
                    Icon::new(IconName::ChevronDown)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_muted()),
                )
                .on_click(cx.listener(|this, _, _, cx| this.open_menu(MenuId::GitRepo, cx)))
        });

    if open && live {
        trigger = trigger.child(panel(app, window, cx));
    }

    trigger
}

fn current_repo_label(app: &AppState, cx: &App) -> Option<String> {
    let overview = app.open_project(cx)?.git.as_ref()?;
    if overview.scoped_to.is_empty() {
        app.project_snapshot(cx)
            .map(|project| project.record.name.clone())
    } else {
        Some(overview.scoped_to.clone())
    }
}

fn panel(app: &AppState, _window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let project_id = app.project(cx).unwrap();
    let open_project = app.open_project(cx).unwrap();
    let overview = open_project.git.as_ref().unwrap();
    let git_repos = &open_project.git_repos;

    let mut rows: Vec<RepoRow> = Vec::new();

    // Main repository
    rows.push(RepoRow {
        name: crate::state::git::head_label(&overview.head),
        scoped_to: overview.scoped_to.clone(),
        is_current: true,
        nested: None,
    });

    // Nested repositories (submodules and independent)
    for nested in git_repos {
        let head = match &nested.head {
            GitHead::Branch(name) => name.clone(),
            GitHead::Detached { short_id } => format!("detached {short_id}"),
            GitHead::Unborn(name) => format!("{name} (unborn)"),
        };
        let is_submodule = nested.submodule;
        let rel_path = nested.rel_path.clone();
        rows.push(RepoRow {
            name: head,
            scoped_to: rel_path.clone(),
            is_current: false,
            nested: Some((is_submodule, rel_path)),
        });
    }

    deferred(
        anchored().snap_to_window_with_margin(px(8.)).child(
            div()
                .id("git-repo-panel")
                .w(px(PANEL_WIDTH))
                .flex()
                .flex_col()
                .bg(theme::surface_raised())
                .border_l(px(theme::accent_edge()))
                .border_color(theme::accent())
                .shadow_lg()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_menu(cx)))
                .child(
                    div()
                        .h(px(32.))
                        .px_3()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .border_b_1()
                        .border_color(theme::border())
                        .child(section_label("Repositories"))
                        .child(div().flex_1().min_w(px(0.)))
                        .child(
                            mono(format!("{} repositories", rows.len()), theme::text_faint())
                                .text_size(theme::font(Family::Chrome, Role::Meta)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scrollbar()
                        .children(rows.iter().map(|row| repo_row(row, project_id, cx))),
                ),
        ),
    )
    .priority(1)
    .into_any_element()
}

struct RepoRow {
    name: String,
    scoped_to: String,
    is_current: bool,
    nested: Option<(bool, String)>,
}

fn repo_row(
    row: &RepoRow,
    _project_id: ubiq_proto::ids::ProjectId,
    cx: &mut Context<AppState>,
) -> AnyElement {
    div()
        .id(ElementId::from(format!("repo-{}", row.name)))
        .h(px(32.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .when(row.is_current, |this| this.bg(theme::accent_soft()))
        .child(
            div()
                .w(px(8.))
                .h(px(8.))
                .flex_none()
                .rounded_full()
                .bg(if row.is_current {
                    theme::accent()
                } else {
                    theme::border()
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(
                    mono(row.name.clone(), theme::text())
                        .text_size(theme::font(Family::Chrome, Role::Body)),
                )
                .when(!row.scoped_to.is_empty(), |this| {
                    this.child(
                        mono(row.scoped_to.clone(), theme::text_muted())
                            .text_size(theme::font(Family::Chrome, Role::Meta)),
                    )
                }),
        )
        .children(row.nested.as_ref().map(|(is_submodule, path)| {
            mono(
                format!("{} {}", if *is_submodule { "📦" } else { "📁" }, path),
                theme::text_faint(),
            )
            .text_size(theme::font(Family::Chrome, Role::Micro))
        }))
        .on_click(cx.listener(move |this, _, _, cx| {
            // Switch to this repository's git view
            // For now, we just close the menu - the actual switching would require
            // tracking which repo's git view is active
            this.close_menu(cx);
        }))
        .into_any_element()
}
