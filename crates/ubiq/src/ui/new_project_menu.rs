//! The titlebar's new-project chevron menu: the same three ways into a project the picker's foot
//! offers — add, clone, remote — reached without opening the picker first.
//!
//! Painted over the window for the reason [`super::overflow_menu`] is: the titlebar does not name
//! `AppState` beyond what it already reads, and this is a third menu the window draws the same
//! way. The rows are `WorkbenchState::new_project_rows`, read again here exactly as
//! `AppState::pick_new_project_menu` reads them — the rule every position-matched menu in this
//! window follows.
//!
//! **Reused, not restated.** The actions behind each row are the same ones
//! `ui::project_menu`'s `add_row`, `clone_row` and `remote_row` already run — see
//! `AppState::pick_new_project_menu`.

use gpui::{Context, IntoElement, Window, div, point, px};

use crate::app::AppState;
use crate::state::NewProjectRow;
use crate::ui::{self, kit};

/// Draw the open new-project menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(at) = app.workbench.new_project_menu else {
        return div().into_any_element();
    };

    let label = match app.preferred_remote_host() {
        Some((_, label)) => format!("Open a project on {label}\u{2026}"),
        None => "Open remote project\u{2026}".to_string(),
    };
    let items: Vec<_> = app
        .workbench
        .new_project_rows()
        .into_iter()
        .map(|row| kit::ContextItem::new(label_for(row, &label)).icon(icon(row)))
        .collect();

    kit::context_menu(
        "new-project-menu",
        point(px(at.0), px(at.1)),
        items,
        ui::indexed(&cx.entity(), |this, index, window, cx| {
            this.pick_new_project_menu(index, window, cx);
        }),
        ui::handler(&cx.entity(), |this, _, cx| {
            this.dismiss_new_project_menu(cx)
        }),
    )
    .into_any_element()
}

/// What one row reads as. `remote_label` carries the picker's own wording for the third row — the
/// active remote's name when one is attached — so the two never say something different for the
/// same action.
fn label_for(row: NewProjectRow, remote_label: &str) -> String {
    match row {
        NewProjectRow::AddProject => "Add a project\u{2026}".to_string(),
        NewProjectRow::CloneProject => "Clone a project\u{2026}".to_string(),
        NewProjectRow::RemoteProject => remote_label.to_string(),
    }
}

/// The glyph one row draws before its label — the same icon its row in the picker's foot uses.
fn icon(row: NewProjectRow) -> gpui_component::IconName {
    match row {
        NewProjectRow::AddProject => gpui_component::IconName::Plus,
        NewProjectRow::CloneProject => gpui_component::IconName::Globe,
        NewProjectRow::RemoteProject => gpui_component::IconName::Network,
    }
}
