//! The databases a project keeps, as the window holds them: the saved connections, where each
//! stands, the structure tree, and the table and SQL tabs open over them.
//!
//! **State only.** Nothing here renders, opens a connection or names a driver — `ubiq-db` is
//! linked here with its default features, the pure half, and a connection is the host's. A
//! connection arrives as a [`DbConnection`] whose password is only ever a [`PasswordState`]; the
//! interface never receives a decrypted one.
//!
//! The pieces are split by who owns them: [`tree`] the explorer's rows and filter, [`table`] one
//! table tab, [`pending`] the pure edit buffer a table tab keeps, [`sql`] one SQL tab, [`form`] the
//! connection form. This file holds what they share — the list, the keystore, the connection
//! states, the open tabs and the password prompt — and the two tab-key spellings.
//!
//! **A tab is a session.** Each table or SQL tab mints a [`DbSessionId`] when it opens and sends it
//! on every request, so a reply for a tab the user has since closed is discarded by id rather than
//! drawn (`receive_db`).

use std::collections::HashMap;

use ubiq_proto::db::{DbConnState, DbConnection, DbFailure, DbKeystore, PasswordState, TableRef};
use ubiq_proto::ids::{DbConnId, DbProbeId, DbSessionId};

pub mod form;
pub mod pending;
pub mod sql;
pub mod table;
pub mod tree;

pub use form::DbConnForm;
pub use sql::DbSqlTab;
pub use table::DbTableTab;
pub use tree::{DbMenu, DbTreeState};

/// What every table tab key starts with.
const TABLE_PREFIX: &str = "db:";
/// What every SQL tab key starts with.
const SQL_PREFIX: &str = "dbsql:";

/// The tab key for one table: `db:<conn>:<database>:<schema>:<name>`, an absent level written as
/// empty. **The one convention** — the panel's payload and the dock's saved layout read it here.
pub fn db_table_key(conn: DbConnId, table: &TableRef) -> String {
    format!(
        "{TABLE_PREFIX}{conn}:{}:{}:{}",
        table.database.as_deref().unwrap_or(""),
        table.schema.as_deref().unwrap_or(""),
        table.name
    )
}

/// The same key read back, or nothing for a key that names no table. The name may itself hold a
/// `:`, which is why it is the last field and the split stops there.
pub fn db_table_from_key(key: &str) -> Option<(DbConnId, TableRef)> {
    let mut parts = key.strip_prefix(TABLE_PREFIX)?.splitn(4, ':');
    let conn = parts.next()?.parse::<DbConnId>().ok()?;
    let database = parts.next()?;
    let schema = parts.next()?;
    let name = parts.next()?;
    fn level(part: &str) -> Option<&str> {
        (!part.is_empty()).then_some(part)
    }
    Some((conn, TableRef::new(level(database), level(schema), name)))
}

/// The tab key for one SQL tab: `dbsql:<session id>`.
pub fn db_sql_key(session: DbSessionId) -> String {
    format!("{SQL_PREFIX}{session}")
}

/// The session a SQL tab key names.
pub fn db_sql_from_key(key: &str) -> Option<DbSessionId> {
    key.strip_prefix(SQL_PREFIX)?.parse::<DbSessionId>().ok()
}

/// What a tab is called when its state has gone: the table's name, or `SQL`.
pub fn db_tab_label(key: &str) -> String {
    match db_table_from_key(key) {
        Some((_, table)) => table.name,
        None => "SQL".to_string(),
    }
}

/// The databases of the project on screen.
#[derive(Default)]
pub struct DbState {
    /// The saved connections, as the host last listed them.
    pub connections: Vec<DbConnection>,
    /// Whether the host has answered the list at all — `loaded` is not `!connections.is_empty()`,
    /// for [`crate::state::KbState`]'s reason: a list that has not arrived draws as waiting, not
    /// as "no connections".
    pub loaded: bool,
    /// Whether this install can seal a password. `None` until the list arrives.
    pub keystore: Option<DbKeystore>,
    /// Where each connection stands, as the host last said. Absent is `Idle`.
    pub states: HashMap<DbConnId, DbConnState>,
    /// The structure tree and its filter.
    pub tree: DbTreeState,
    /// The explorer's right-click menu, while one is up.
    pub menu: Option<DbMenu>,
    /// The table tabs open, in the order they were opened.
    pub tables: Vec<DbTableTab>,
    /// The SQL tabs open, in the order they were opened.
    pub sqls: Vec<DbSqlTab>,
    /// The prompt a connection raised by answering `NeedsPassword`, while it is up.
    pub password_prompt: Option<DbPasswordPrompt>,
    /// The connection form, while the modal is up.
    pub form: Option<DbConnForm>,
    /// The answers to Test, by probe. Absent means the probe is still running or was never sent.
    pub probes: HashMap<DbProbeId, Result<String, DbFailure>>,
    /// Stamped onto each menu as it opens, so the outside click that dismisses the old one cannot
    /// shut the new one raised by the same event — [`crate::state::KbState`]'s reasoning.
    pub menu_epoch: u64,
}

/// The password a connection asked for, and what the person has chosen about keeping it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbPasswordPrompt {
    pub conn: DbConnId,
    /// Whether to seal it under this install's key. Off when the keystore is unavailable.
    pub remember: bool,
}

impl DbState {
    /// Take the host's list. A connection that is gone takes its state, its tree and its tabs'
    /// right to be drawn with it; the tabs themselves close on the window's side.
    pub fn accept(&mut self, connections: Vec<DbConnection>, keystore: DbKeystore) {
        self.states
            .retain(|conn, _| connections.iter().any(|known| known.id == *conn));
        if self
            .password_prompt
            .as_ref()
            .is_some_and(|prompt| !connections.iter().any(|known| known.id == prompt.conn))
        {
            self.password_prompt = None;
        }
        self.connections = connections;
        self.tree.sync(&self.connections);
        self.keystore = Some(keystore);
        self.loaded = true;
    }

    pub fn connection(&self, id: DbConnId) -> Option<&DbConnection> {
        self.connections.iter().find(|conn| conn.id == id)
    }

    /// Where a connection stands; `Idle` for one nobody has touched.
    pub fn state_of(&self, id: DbConnId) -> &DbConnState {
        self.states.get(&id).unwrap_or(&DbConnState::Idle)
    }

    /// Whether a connection cannot write, whatever a tab says: the connection's own setting.
    pub fn is_read_only(&self, id: DbConnId) -> bool {
        self.connection(id)
            .is_some_and(|conn| conn.config.read_only)
    }

    /// Record a connection's new state. `NeedsPassword` raises the prompt; reaching `Connected`
    /// takes it down. The prompt offers *Remember* only while the keystore can seal.
    pub fn set_state(&mut self, conn: DbConnId, state: DbConnState) {
        match &state {
            DbConnState::NeedsPassword => {
                let remember = !matches!(self.keystore, Some(DbKeystore::Unavailable(_)));
                self.password_prompt = Some(DbPasswordPrompt { conn, remember });
            }
            DbConnState::Connected { .. }
                if self
                    .password_prompt
                    .as_ref()
                    .is_some_and(|prompt| prompt.conn == conn) =>
            {
                self.password_prompt = None;
            }
            _ => {}
        }
        self.states.insert(conn, state);
    }

    /// How a connection's password stands, for the settings rows and the form.
    pub fn password_of(&self, id: DbConnId) -> PasswordState {
        self.connection(id)
            .map(|conn| conn.password)
            .unwrap_or_default()
    }

    // ── The open tabs ───────────────────────────────────────────────

    pub fn table_tab(&self, key: &str) -> Option<&DbTableTab> {
        self.tables.iter().find(|tab| tab.key == key)
    }

    pub fn table_tab_mut(&mut self, key: &str) -> Option<&mut DbTableTab> {
        self.tables.iter_mut().find(|tab| tab.key == key)
    }

    pub fn table_by_session(&mut self, session: DbSessionId) -> Option<&mut DbTableTab> {
        self.tables.iter_mut().find(|tab| tab.session == session)
    }

    pub fn sql_tab(&self, key: &str) -> Option<&DbSqlTab> {
        self.sqls.iter().find(|tab| tab.key() == key)
    }

    pub fn sql_tab_mut(&mut self, key: &str) -> Option<&mut DbSqlTab> {
        self.sqls.iter_mut().find(|tab| tab.key() == key)
    }

    pub fn sql_by_session(&mut self, session: DbSessionId) -> Option<&mut DbSqlTab> {
        self.sqls.iter_mut().find(|tab| tab.session == session)
    }

    /// Whether a panel key names a tab this project has open — the dock's `file_open` answer for
    /// the two database kinds.
    pub fn holds_tab(&self, key: &str) -> bool {
        self.table_tab(key).is_some() || self.sql_tab(key).is_some()
    }

    /// Every open tab's key, tables first.
    pub fn tab_keys(&self) -> Vec<String> {
        self.tables
            .iter()
            .map(|tab| tab.key.clone())
            .chain(self.sqls.iter().map(|tab| tab.key()))
            .collect()
    }

    /// Whether a table tab is open — what the centre page steps aside for.
    pub fn any_table_open(&self) -> bool {
        !self.tables.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_key_reads_back_to_the_table_it_named() {
        let conn = DbConnId::generate();
        let table = TableRef::new(Some("orders"), None, "line:items");
        let key = db_table_key(conn, &table);
        assert_eq!(db_table_from_key(&key), Some((conn, table)));
        assert_eq!(db_tab_label(&key), "line:items");
    }

    #[test]
    fn a_sql_key_reads_back_to_its_session() {
        let session = DbSessionId::generate();
        assert_eq!(db_sql_from_key(&db_sql_key(session)), Some(session));
        assert_eq!(db_table_from_key(&db_sql_key(session)), None);
    }

    #[test]
    fn needing_a_password_raises_the_prompt_and_connecting_takes_it_down() {
        let mut db = DbState::default();
        let conn = DbConnId::generate();
        db.set_state(conn, DbConnState::NeedsPassword);
        assert_eq!(db.password_prompt.as_ref().map(|p| p.conn), Some(conn));
        db.set_state(
            conn,
            DbConnState::Connected {
                server: "3.45".into(),
            },
        );
        assert!(db.password_prompt.is_none());
    }
}
