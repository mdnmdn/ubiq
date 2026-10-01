//! The database explorer on screen: the explorer panel, a table tab, a SQL tab, the Databases
//! settings section and the page DB mode draws when no table is open.
//!
//! One file per panel, so the packages that build them own disjoint files: [`explorer`] the left
//! panel, [`table`] a centre tab, [`sql`] a bottom tab, [`settings`] the section's body. Each
//! package declares the modules under its own file (`table.rs` its grid and cell editors, `sql.rs`
//! its plan view, `explorer.rs` the connection form), so this file does not grow with them.
//! [`keys`] holds the actions and bindings of both tab kinds.

use gpui::{AnyElement, Context, IntoElement, Styled, Window, div, px};

use crate::app::AppState;
use crate::theme;
use crate::ui::kit::UbiqIcon;
use crate::ui::{empty, mark};

pub mod explorer;
pub mod keys;
pub mod settings;
pub mod sql;
pub mod table;

/// The centre with no table open: the explorer is what opens one, so that is what it points at.
/// As soon as a table is open the table panels *are* the centre and this steps aside, as
/// [`crate::ui::kb::centre`] does for documents.
pub fn centre(app: &AppState, _window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let open = app.db(cx).is_some_and(|db| db.any_table_open());
    if open {
        // Reached for the frame between a tab opening and the dock settling its panel.
        return div()
            .flex()
            .flex_1()
            .min_h(px(0.))
            .bg(theme::app_bg())
            .into_any_element();
    }
    mark::backdrop(
        app,
        empty::empty_page(
            "No table open",
            "Pick one from the databases explorer on the left.",
            UbiqIcon::ModeDb,
            None,
        )
        .into_any_element(),
        cx,
    )
}
