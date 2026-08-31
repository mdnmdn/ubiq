//! The file tree panel.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement, Rgba,
    SharedString, StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::{GitStatus, Row};
use crate::theme;
use crate::ui::empty::empty_panel;
use crate::ui::kit::{badge, icon_button, mono, panel, panel_header};

/// The colour a row's name and dot take from its git state. Status is never shown by wording alone.
///
/// Nothing to say about a file is the muted default, which is also what every row looks like until
/// something reads a repository.
pub fn git_colour(status: Option<GitStatus>) -> Rgba {
    match status {
        None => theme::text_muted(),
        Some(GitStatus::Modified) => theme::warning(),
        Some(GitStatus::Untracked) => theme::success(),
        Some(GitStatus::Conflict) => theme::danger(),
        Some(GitStatus::Staged) => theme::info(),
        Some(GitStatus::Ignored) => theme::text_faint(),
    }
}

fn name_colour(status: Option<GitStatus>, readable: bool) -> Rgba {
    match status {
        None if readable => theme::text(),
        // Something the host will not open reads as unavailable rather than as unremarkable.
        None => theme::text_faint(),
        Some(GitStatus::Ignored) => theme::text_faint(),
        Some(other) => git_colour(Some(other)),
    }
}

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    // With no project there is no folder to list. The panel stays, so the emptiness is explained
    // and the width the user dragged survives a project opening.
    if app.project(cx).is_none() {
        return panel()
            .border_r_1()
            .border_color(theme::border())
            .child(panel_header("Explorer", div()))
            .child(empty_panel("No project open"))
            .into_any_element();
    }

    // The tree belongs to the project; the filter belongs to the window, because there is one
    // field above N trees.
    let Some(explorer) = app.explorer(cx) else {
        return div().into_any_element();
    };
    let selected = explorer.selected.clone();
    let mut rows = Vec::new();
    for row in explorer.rows(&app.workbench.file_filter) {
        rows.push(tree_row(row, selected.as_deref(), cx));
    }

    panel()
        .border_r_1()
        .border_color(theme::border())
        .child(panel_header(
            "Explorer",
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(icon_button(
                    "explorer-new",
                    IconName::Plus,
                    false,
                    |_, _, _| {},
                ))
                .child(icon_button(
                    "explorer-collapse",
                    IconName::ChevronsUpDown,
                    false,
                    cx.listener(|this, _, _, cx| {
                        if let Some(open) = this.open_project_mut(cx) {
                            open.explorer.collapse_all();
                        }
                        cx.notify();
                    }),
                )),
        ))
        .child(
            div().pr_3().pb_2().flex().flex_none().child(
                div()
                    .w_full()
                    .h(px(32.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(theme::surface())
                    .border_l(px(theme::ACCENT_EDGE))
                    .border_color(theme::border())
                    .child(
                        Icon::new(IconName::Search)
                            .with_size(Size::XSmall)
                            .text_color(theme::text_faint()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(12.5))
                            .child(Input::new(&app.file_filter).appearance(false)),
                    )
                    .child(
                        mono("\u{2318}P", theme::text_faint())
                            .text_size(px(10.5))
                            .px_1()
                            .bg(theme::surface_raised()),
                    ),
            ),
        )
        .child(
            div()
                .id("explorer-tree")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .pr_2()
                .overflow_y_scroll()
                .children(rows),
        )
        .into_any_element()
}

fn tree_row(row: Row, selected: Option<&str>, cx: &mut Context<AppState>) -> AnyElement {
    let is_selected = selected == Some(row.path.as_str());
    let path = row.path.clone();
    let is_dir = row.is_dir;

    let mut line = div()
        .id(ElementId::Name(format!("tree-{}", row.path).into()))
        .h(px(26.))
        .pr_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        // The indent is drawn, not padded, so the accent bar of a selected row stays flush left.
        .child(div().w(px(6.0 + row.depth as f32 * 14.0)).flex_none())
        .child(if is_dir {
            Icon::new(if row.expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(Size::XSmall)
            .text_color(theme::text_muted())
            .into_any_element()
        } else {
            div()
                .size(px(14.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .child(div().size(px(6.)).rounded_full().bg(git_colour(row.git)))
                .into_any_element()
        })
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(13.))
                .text_color(name_colour(row.git, row.readable))
                .child(SharedString::from(row.name.clone())),
        );

    if let Some(status) = row.git {
        line = line.child(badge(status.badge(), git_colour(row.git)));
    }

    if is_selected {
        line = line
            .bg(theme::accent_soft())
            .border_l_2()
            .border_color(theme::accent());
    }

    // A row the host will not follow is drawn and does nothing: there is nothing behind it to
    // list or to open.
    if !row.readable {
        return line.into_any_element();
    }

    line.on_click(cx.listener(move |this, _, _window, cx| {
        if is_dir {
            this.toggle_folder(path.clone(), cx);
        } else {
            this.select_file(path.clone(), cx);
        }
    }))
    .into_any_element()
}
