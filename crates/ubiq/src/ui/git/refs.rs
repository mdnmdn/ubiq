//! The ref sidebar: local branches, remotes, tags, stashes and submodules, each section shutting
//! on its own.
//!
//! A row is the file list's row — the same chrome the explorer and the picker draw — because a ref
//! is read the way a path is: one line, elided, marked when it is the one selected. What is
//! different is what the row carries at each end: a dot saying whether this is what HEAD points
//! at, and the tracking counts at the far end.
//!
//! Every row here is the host's: `state::git::ref_rows` turns a `GitRefs` reply, plus the
//! overview's submodules, into what this module draws.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};

use crate::app::AppState;
use crate::state::git::{RefRow, RefSection, RefTreeKind};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::eid;
use crate::ui::kit::{
    disclosure, elided, elided_with, file_row, mono, panel, row_font, status_dot, twisty,
};

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };

    let mut body = div()
        .id("git-refs")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_scroll();

    // Grouped once here rather than filtered once per section below — five passes over `refs`
    // become one.
    let groups = git.grouped_refs();
    for section in RefSection::all() {
        let open = git.is_open(section);
        let rows = &groups[section.slot()];
        body = body.child(disclosure(
            eid("git-section", section_id(section)),
            section.label(),
            mono(format!("{}", rows.len()), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Meta)),
            open,
            cx.listener(move |this, _, _, cx| this.toggle_git_section(section, cx)),
        ));

        if !open {
            continue;
        }
        if matches!(section, RefSection::Local | RefSection::Remotes) {
            for item in git.ref_tree(section) {
                body = body.child(tree_row(section, &item, &git.refs, git.selected_ref, cx));
            }
        } else {
            for &(index, row) in rows {
                body = body.child(ref_row(
                    index,
                    row,
                    0,
                    &row.name,
                    None,
                    git.selected_ref == Some(index),
                    cx,
                ));
            }
        }
    }

    panel().child(body).into_any_element()
}

fn tree_row(
    section: RefSection,
    item: &crate::state::git::RefTreeRow,
    refs: &[RefRow],
    selected_ref: Option<usize>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    match &item.kind {
        RefTreeKind::Folder { path, open, leaves } => {
            let path = path.clone();
            file_row(
                eid("git-folder", path.clone()),
                item.depth,
                false,
                false,
                false,
                row_font(),
            )
            .child(twisty(
                eid("git-folder-twisty", path.clone()),
                *open,
                cx.listener({
                    let path = path.clone();
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_git_folder(section, path.clone(), cx);
                    }
                }),
            ))
            .child(elided(
                eid("git-folder-name", path.clone()),
                item.label.clone(),
                theme::text_muted(),
                theme::font(theme::Family::Chrome, theme::Role::Body),
            ))
            .child(
                mono(format!("{leaves}"), theme::text_faint())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_git_folder(section, path.clone(), cx);
            }))
            .into_any_element()
        }
        RefTreeKind::Leaf {
            index,
            has_children,
            open,
        } => {
            let Some(row) = refs.get(*index) else {
                return div().into_any_element();
            };
            ref_row(
                *index,
                row,
                item.depth,
                &item.label,
                (*has_children).then_some(*open),
                selected_ref == Some(*index),
                cx,
            )
        }
    }
}

/// One ref. The dot says whether this is what HEAD points at; the counts at the end are the
/// commits either side of its upstream, and are absent rather than zero when it has none.
fn ref_row(
    index: usize,
    row: &RefRow,
    depth: usize,
    label: &str,
    twisty_open: Option<bool>,
    selected: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let colour = if row.current {
        theme::accent()
    } else {
        theme::text_faint()
    };
    let path = row.name.clone();
    let section = row.section;

    let mut line = file_row(
        eid("git-ref", index),
        depth,
        selected,
        false,
        false,
        row_font(),
    );
    if let Some(open) = twisty_open {
        line = line.child(twisty(
            eid("git-ref-twisty", index),
            open,
            cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_git_folder(section, path.clone(), cx);
            }),
        ));
    }
    line.child(status_dot(colour, theme::pane_bg()))
        .child(elided_with(
            eid("git-ref-name", index),
            label.to_string(),
            row.name.clone(),
            if row.current {
                theme::text()
            } else {
                theme::text_muted()
            },
            theme::font(theme::Family::Chrome, theme::Role::Body),
        ))
        .children(row.ahead.map(|ahead| {
            mono(format!("\u{2191}{ahead}"), theme::success())
                .text_size(theme::font(Family::Chrome, Role::Meta))
        }))
        .children(row.behind.map(|behind| {
            mono(format!("\u{2193}{behind}"), theme::warning())
                .text_size(theme::font(Family::Chrome, Role::Meta))
        }))
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if event.click_count() >= 2 {
                this.jump_to_git_ref(index, cx);
            } else {
                this.select_git_ref(index, cx);
            }
        }))
        .into_any_element()
}

/// A stable fragment for the section's element id. The enum has no ULID behind it and no index
/// that survives a reorder, so the name is what it is keyed by.
fn section_id(section: RefSection) -> &'static str {
    match section {
        RefSection::Local => "local",
        RefSection::Remotes => "remotes",
        RefSection::Tags => "tags",
        RefSection::Stashes => "stashes",
        RefSection::Submodules => "submodules",
    }
}
