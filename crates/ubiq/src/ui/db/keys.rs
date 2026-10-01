//! The database tabs' actions and key bindings, declared once.
//!
//! A table tab's root element carries the key context [`TABLE_CONTEXT`] and a SQL tab's
//! [`SQL_CONTEXT`]; each package registers `on_action` handlers for the actions below on its own
//! tab. Every binding is made twice — for the tab and for the field inside it — for the reason
//! [`crate::ui::explorer::key_bindings`] gives: the component library binds at the deepest node,
//! so a binding on the tab alone is never reached while the caret is in the editor.

use gpui::KeyBinding;

/// The key context of a table tab's root.
pub const TABLE_CONTEXT: &str = "DbTable";
/// The key context of a SQL tab's root.
pub const SQL_CONTEXT: &str = "DbSql";

gpui::actions!(
    ubiq_db,
    [
        /// Run the statement under the cursor, or the selection.
        DbRunStatement,
        /// Run every statement, one result tab each.
        DbRunAll,
        /// Stop the statement that is running.
        DbStop,
        /// Edit the selected cell in place.
        DbEditCell,
    ]
);

/// The keys the tabs answer to.
pub fn key_bindings() -> Vec<KeyBinding> {
    fn sql<A: gpui::Action + Clone>(key: &str, action: A) -> [KeyBinding; 2] {
        [
            KeyBinding::new(key, action.clone(), Some(SQL_CONTEXT)),
            KeyBinding::new(key, action, Some("DbSql > Input")),
        ]
    }
    let mut binds = Vec::new();
    binds.extend(sql("cmd-enter", DbRunStatement));
    binds.extend(sql("ctrl-enter", DbRunStatement));
    binds.extend(sql("cmd-shift-enter", DbRunAll));
    binds.extend(sql("ctrl-shift-enter", DbRunAll));
    binds.extend(sql("cmd-.", DbStop));
    binds.extend(sql("ctrl-.", DbStop));
    binds.push(KeyBinding::new("f2", DbEditCell, Some(TABLE_CONTEXT)));
    binds
}
