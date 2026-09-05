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
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};

use crate::app::AppState;
use crate::state::git::{RefRow, RefSection};
use crate::theme;
use crate::ui::eid;
use crate::ui::kit::{ROW_FONT, disclosure, elided, file_row, mono, panel, status_dot};

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

    for section in RefSection::all() {
        let open = git.is_open(section);
        let count = git.count(section);
        body = body.child(disclosure(
            eid("git-section", section_id(section)),
            section.label(),
            mono(format!("{count}"), theme::text_faint()).text_size(px(11.)),
            open,
            cx.listener(move |this, _, _, cx| this.toggle_git_section(section, cx)),
        ));

        if !open {
            continue;
        }
        for (index, row) in git.rows(section) {
            body = body.child(ref_row(index, row, git.selected_ref == Some(index), cx));
        }
    }

    panel().child(body).into_any_element()
}

/// One ref. The dot says whether this is what HEAD points at; the counts at the end are the
/// commits either side of its upstream, and are absent rather than zero when it has none.
fn ref_row(index: usize, row: &RefRow, selected: bool, cx: &mut Context<AppState>) -> AnyElement {
    let colour = if row.current {
        theme::accent()
    } else {
        theme::text_faint()
    };

    file_row(eid("git-ref", index), 0, selected, false, false, ROW_FONT)
        .child(status_dot(colour, theme::pane_bg()))
        .child(elided(
            eid("git-ref-name", index),
            row.name.clone(),
            if row.current {
                theme::text()
            } else {
                theme::text_muted()
            },
            12.5,
        ))
        .children(
            row.ahead
                .map(|ahead| mono(format!("\u{2191}{ahead}"), theme::success()).text_size(px(11.))),
        )
        .children(
            row.behind.map(|behind| {
                mono(format!("\u{2193}{behind}"), theme::warning()).text_size(px(11.))
            }),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.select_git_ref(index, cx)))
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
