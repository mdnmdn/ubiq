//! The explorer's structure tree: nodes, what each has loaded, the flattened rows, the filter and
//! the right-click menu's pure half.
//!
//! Levels follow the driver contract: connection → database → [schema] → object group → object →
//! column. A node's children are fetched the first time it is expanded, by one `DbTree` request
//! whose `DbNode` is derived from the node alone ([`Node::request`]); the groups under a schema (or
//! a database without schemas) arrive together with their objects, because one `Objects` listing
//! answers all of them. **No GPUI value but two handles** — the tree's scroll and the keyboard's
//! focus — and nothing here sends: every mutator answers with the request the caller should make.

use std::collections::HashMap;

use gpui::{FocusHandle, ScrollHandle};
use ubiq_proto::db::{
    DbConnState, DbConnection, DbKind, DbListing, DbNode, DbObject, ObjectKind, TableRef,
};
use ubiq_proto::ids::{DbConnId, DbProbeId};

/// What a node is.
#[derive(Clone, Debug)]
pub enum NodeKind {
    Connection,
    Database(String),
    Schema {
        database: String,
        schema: String,
    },
    Group(ObjectKind),
    Object(DbObject),
    Column {
        pk: bool,
    },
    /// A line of text under a node: "(no objects)".
    Note,
}

/// Where a node's children stand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    /// Not fetched yet.
    Idle,
    Loading,
    Done,
    /// Shown on the node, in the danger colour.
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Node {
    /// Unique in the whole tree: `<conn>/d:main/g:Tables/o:users`.
    pub id: String,
    pub conn: DbConnId,
    pub kind: NodeKind,
    pub label: String,
    /// Dim text after the label: a column type, a member count, a synonym's target.
    pub detail: String,
    pub expanded: bool,
    pub load: Load,
    pub children: Vec<Node>,
}

impl Node {
    fn new(parent: &str, tag: &str, conn: DbConnId, kind: NodeKind, label: &str) -> Node {
        let load = match kind {
            NodeKind::Group(_) | NodeKind::Column { .. } | NodeKind::Note => Load::Done,
            _ => Load::Idle,
        };
        Node {
            id: format!("{parent}/{tag}:{label}"),
            conn,
            kind,
            label: label.to_string(),
            detail: String::new(),
            expanded: false,
            load,
            children: Vec::new(),
        }
    }

    /// The root node of a saved connection.
    pub fn connection(conn: DbConnId, label: &str) -> Node {
        Node {
            id: conn.to_string(),
            conn,
            kind: NodeKind::Connection,
            label: label.to_string(),
            detail: String::new(),
            expanded: false,
            load: Load::Idle,
            children: Vec::new(),
        }
    }

    /// Whether the row has a disclosure arrow.
    pub fn expandable(&self) -> bool {
        !matches!(self.kind, NodeKind::Column { .. } | NodeKind::Note)
    }

    /// The table a double-click opens: relations only.
    pub fn table_ref(&self) -> Option<TableRef> {
        match &self.kind {
            NodeKind::Object(obj) if obj.kind.is_relation() => Some(obj.table_ref()),
            _ => None,
        }
    }

    /// The database this node sits in, for a SQL editor opened from it.
    pub fn database(&self) -> Option<String> {
        match &self.kind {
            NodeKind::Database(name) => Some(name.clone()),
            NodeKind::Schema { database, .. } => Some(database.clone()),
            NodeKind::Object(obj) => obj.database.clone(),
            _ => None,
        }
    }

    /// What this node needs fetched, derived from the node alone.
    pub fn request(&self, kind: DbKind) -> Option<DbNode> {
        match &self.kind {
            NodeKind::Connection => Some(DbNode::Databases),
            NodeKind::Database(database) if kind.has_schemas() => Some(DbNode::Schemas {
                database: database.clone(),
            }),
            NodeKind::Database(database) => Some(DbNode::Objects {
                database: database.clone(),
                schema: None,
            }),
            NodeKind::Schema { database, schema } => Some(DbNode::Objects {
                database: database.clone(),
                schema: Some(schema.clone()),
            }),
            NodeKind::Object(obj) => Some(DbNode::Columns {
                table: obj.table_ref(),
            }),
            NodeKind::Group(_) | NodeKind::Column { .. } | NodeKind::Note => None,
        }
    }

    pub fn find(&self, id: &str) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Node> {
        if self.id == id {
            return Some(self);
        }
        self.children
            .iter_mut()
            .find_map(|child| child.find_mut(id))
    }

    /// The node whose `request` is `want` — where a listing is filed.
    fn answering(&mut self, kind: DbKind, want: &DbNode) -> Option<&mut Node> {
        if self.request(kind).as_ref() == Some(want) {
            return Some(self);
        }
        self.children
            .iter_mut()
            .find_map(|child| child.answering(kind, want))
    }

    /// Back to "never loaded": a disconnect, or a refresh.
    pub fn reset(&mut self) {
        self.expanded = false;
        self.load = Load::Idle;
        self.children.clear();
    }

    /// Whether this node or anything loaded under it matches the lowercase `needle`.
    fn matches_deep(&self, needle: &str) -> bool {
        self.matches(needle) || self.children.iter().any(|child| child.matches_deep(needle))
    }

    fn matches(&self, needle: &str) -> bool {
        !matches!(self.kind, NodeKind::Note) && self.label.to_lowercase().contains(needle)
    }
}

/// Shape a listing as the children of `parent`.
fn fill(parent: &Node, kind: DbKind, listing: DbListing) -> Vec<Node> {
    let (pid, conn) = (parent.id.as_str(), parent.conn);
    match (&parent.kind, listing) {
        (NodeKind::Connection, DbListing::Names(names)) => names
            .iter()
            .map(|name| Node::new(pid, "d", conn, NodeKind::Database(name.clone()), name))
            .collect(),
        (NodeKind::Database(database), DbListing::Names(names)) => names
            .iter()
            .map(|schema| {
                let kind = NodeKind::Schema {
                    database: database.clone(),
                    schema: schema.clone(),
                };
                Node::new(pid, "s", conn, kind, schema)
            })
            .collect(),
        (_, DbListing::Objects(objects)) => groups(pid, conn, kind, objects),
        (_, DbListing::Columns(columns)) => columns
            .iter()
            .map(|col| {
                let mut node = Node::new(
                    pid,
                    "c",
                    conn,
                    NodeKind::Column { pk: col.is_pk },
                    &col.name,
                );
                node.detail = col.native_type.clone();
                node
            })
            .collect(),
        // A listing that does not fit its node: nothing to show rather than a wrong tree.
        _ => Vec::new(),
    }
}

/// One group node per object kind the engine has, empty ones left out.
fn groups(pid: &str, conn: DbConnId, kind: DbKind, objects: Vec<DbObject>) -> Vec<Node> {
    let mut out = Vec::new();
    for &object_kind in kind.object_kinds() {
        let members: Vec<&DbObject> = objects.iter().filter(|o| o.kind == object_kind).collect();
        if members.is_empty() {
            continue;
        }
        let mut group = Node::new(
            pid,
            "g",
            conn,
            NodeKind::Group(object_kind),
            object_kind.group_label(),
        );
        group.detail = members.len().to_string();
        group.children = members
            .into_iter()
            .map(|obj| {
                let mut node = Node::new(
                    &group.id,
                    "o",
                    conn,
                    NodeKind::Object(obj.clone()),
                    &obj.name,
                );
                if let Some(target) = &obj.target {
                    node.detail = format!("-> {target}");
                }
                node
            })
            .collect();
        out.push(group);
    }
    if out.is_empty() {
        out.push(Node::new(pid, "n", conn, NodeKind::Note, "(no objects)"));
    }
    out
}

/// A row estimate for display: `~12`, `~12.3k`, `~123k`, `~4.1M`. One decimal below 100 of a
/// unit, none above; a value that rounds up to 1000 of a unit moves to the next one.
pub fn humanise_rows(n: u64) -> String {
    const UNITS: [(f64, &str); 4] = [(1e3, "k"), (1e6, "M"), (1e9, "B"), (1e12, "T")];
    if n < 1000 {
        return format!("~{n}");
    }
    let shown = |value: f64| {
        if value < 100.0 {
            (value * 10.0).round() / 10.0
        } else {
            value.round()
        }
    };
    let mut ix = UNITS
        .iter()
        .rposition(|(size, _)| n as f64 >= *size)
        .unwrap_or(0);
    let mut value = shown(n as f64 / UNITS[ix].0);
    // 999_950 would read 1000k: say 1M instead.
    if value >= 1000.0 && ix + 1 < UNITS.len() {
        ix += 1;
        value = shown(n as f64 / UNITS[ix].0);
    }
    let text = if value < 100.0 {
        format!("{value:.1}").trim_end_matches(".0").to_string()
    } else {
        format!("{value:.0}")
    };
    format!("~{text}{}", UNITS[ix].1)
}

/// What a row shows right-aligned: the estimate of a table or materialized view. `None` for
/// everything else and for a table the engine has no statistics for.
pub fn approx_label(obj: &DbObject) -> Option<String> {
    matches!(obj.kind, ObjectKind::Table | ObjectKind::MaterializedView)
        .then_some(obj.approx_rows)
        .flatten()
        .map(humanise_rows)
}

/// One visible line of the flattened tree.
#[derive(Clone, Copy)]
pub struct Row<'a> {
    pub depth: usize,
    pub node: &'a Node,
    /// Whether the twisty reads open: the node's own flag, or forced by a filter that kept it for
    /// the match under it.
    pub open: bool,
}

/// Flatten what is visible, depth first. With no filter that is every expanded node's children;
/// with one it is every **loaded** node whose name contains it, case-insensitively, plus the
/// ancestors that lead to one — the filter fetches nothing, so it answers over what is in memory.
pub fn visible<'a>(roots: &'a [Node], filter: &str) -> Vec<Row<'a>> {
    fn walk<'a>(node: &'a Node, depth: usize, needle: &str, out: &mut Vec<Row<'a>>) {
        if needle.is_empty() {
            out.push(Row {
                depth,
                node,
                open: node.expanded,
            });
            if node.expanded {
                for child in &node.children {
                    walk(child, depth + 1, needle, out);
                }
            }
            return;
        }
        if !node.matches_deep(needle) {
            return;
        }
        let below = node.children.iter().any(|child| child.matches_deep(needle));
        out.push(Row {
            depth,
            node,
            open: below,
        });
        for child in &node.children {
            walk(child, depth + 1, needle, out);
        }
    }
    let needle = filter.trim().to_lowercase();
    let mut out = Vec::new();
    for root in roots {
        walk(root, 0, &needle, &mut out);
    }
    out
}

/// The keys the tree answers to once it holds the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Dismiss,
}

// ── The right-click menu ────────────────────────────────────────────────────

/// Which kind of row a menu was raised on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbMenuRow {
    Connection {
        /// Whether the host holds a live connection (`DbConnState::Connected`).
        connected: bool,
    },
    /// A database or a schema.
    Container,
    /// A table, view, materialized view or synonym.
    Relation,
    Column,
}

/// One command the explorer's right-click offers. Nothing is drawn disabled: an entry a row cannot
/// do is left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbAction {
    Connect,
    Disconnect,
    NewSql,
    Refresh,
    RefreshCounts,
    EditConnection,
    Remove,
    OpenData,
    OpenInSql,
    CopyName,
    /// The line between two groups. An action so it holds a slot in the list the pick indexes.
    Separator,
}

impl DbAction {
    pub fn label(self) -> &'static str {
        match self {
            DbAction::Connect => "Connect",
            DbAction::Disconnect => "Disconnect",
            DbAction::NewSql => "New SQL editor",
            DbAction::Refresh => "Refresh",
            DbAction::RefreshCounts => "Refresh counts",
            DbAction::EditConnection => "Edit connection\u{2026}",
            DbAction::Remove => "Remove",
            DbAction::OpenData => "Open data",
            DbAction::OpenInSql => "Open in SQL editor",
            DbAction::CopyName => "Copy name",
            DbAction::Separator => "",
        }
    }

    pub fn is_separator(self) -> bool {
        self == DbAction::Separator
    }
}

/// What a right-click on this kind of row offers, in the order the menu draws it. The order is
/// what the pick reads, so it is decided here rather than by whoever draws the list.
pub fn db_menu_entries(row: DbMenuRow) -> Vec<DbAction> {
    let groups: Vec<Vec<DbAction>> = match row {
        DbMenuRow::Connection { connected: true } => vec![
            vec![DbAction::NewSql, DbAction::Refresh, DbAction::Disconnect],
            vec![DbAction::EditConnection, DbAction::Remove],
        ],
        DbMenuRow::Connection { connected: false } => vec![
            vec![DbAction::Connect],
            vec![DbAction::EditConnection, DbAction::Remove],
        ],
        DbMenuRow::Container => vec![vec![DbAction::NewSql, DbAction::RefreshCounts]],
        DbMenuRow::Relation => vec![
            vec![DbAction::OpenData, DbAction::OpenInSql],
            vec![DbAction::CopyName, DbAction::Refresh],
        ],
        DbMenuRow::Column => vec![vec![DbAction::CopyName]],
    };
    let mut items = Vec::new();
    for group in groups {
        if !items.is_empty() {
            items.push(DbAction::Separator);
        }
        items.extend(group);
    }
    items
}

/// The explorer's right-click menu: where it opened, which row it is about, and which tick of
/// `menu_epoch` it is. Everything its entry list depends on is copied in as it opens, so
/// [`DbMenu::entries`] stays a pure function of what was remembered and the pick, an index into
/// that list, points at the row that was actually drawn.
#[derive(Clone, Debug)]
pub struct DbMenu {
    pub epoch: u64,
    pub x: f32,
    pub y: f32,
    pub conn: DbConnId,
    /// The node's id in [`DbTreeState::roots`].
    pub node: String,
    pub row: DbMenuRow,
}

impl DbMenu {
    pub fn entries(&self) -> Vec<DbAction> {
        db_menu_entries(self.row)
    }
}

// ── The state ───────────────────────────────────────────────────────────────

/// What the gesture that expanded a node asks the caller to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Toggled {
    /// Nothing to ask: collapsed, already loaded, or not expandable.
    Done,
    /// Send this `DbTree` for the node.
    Ask(DbNode),
}

/// The tree's state, and the small amount of the explorer's and the settings list's UI that has no
/// better owner.
#[derive(Default)]
pub struct DbTreeState {
    /// What the filter field holds. It filters the *loaded* nodes by name, case-insensitively,
    /// and never fetches.
    pub filter: String,
    /// One root per saved connection, in the list's order.
    pub roots: Vec<Node>,
    /// The row the keyboard is on, by node id.
    pub selected: Option<String>,
    /// The tree's scroll, so a key that moves the cursor can bring it into view.
    pub scroll: ScrollHandle,
    /// The tree as a focus of its own, separate from the filter above it. Made in `render`, where
    /// there is a `Context`.
    pub focus: Option<FocusHandle>,
    /// The connection the Databases settings section is asking to remove, until it is answered.
    pub removing: Option<DbConnId>,
    /// The Test each settings row last sent, looked up in `DbState::probes`.
    pub tests: HashMap<DbConnId, DbProbeId>,
    /// The password prompt's field, made when the prompt first needs one.
    pub prompt_input: Option<gpui::Entity<gpui_component::input::InputState>>,
}

impl DbTreeState {
    /// Follow the connection list: a root stays with its tree, a new connection gets a fresh
    /// root, a removed one takes its tree with it, and a rename relabels.
    pub fn sync(&mut self, connections: &[DbConnection]) {
        let mut held = std::mem::take(&mut self.roots);
        self.roots = connections
            .iter()
            .map(|conn| match held.iter().position(|root| root.conn == conn.id) {
                Some(at) => {
                    let mut root = held.remove(at);
                    root.label = conn.config.name.clone();
                    root
                }
                None => Node::connection(conn.id, &conn.config.name),
            })
            .collect();
        if self
            .selected
            .as_ref()
            .is_some_and(|id| self.roots.iter().all(|root| root.find(id).is_none()))
        {
            self.selected = None;
        }
    }

    pub fn root(&self, conn: DbConnId) -> Option<&Node> {
        self.roots.iter().find(|root| root.conn == conn)
    }

    pub fn root_mut(&mut self, conn: DbConnId) -> Option<&mut Node> {
        self.roots.iter_mut().find(|root| root.conn == conn)
    }

    pub fn node(&self, conn: DbConnId, id: &str) -> Option<&Node> {
        self.root(conn)?.find(id)
    }

    fn node_mut(&mut self, conn: DbConnId, id: &str) -> Option<&mut Node> {
        self.root_mut(conn)?.find_mut(id)
    }

    /// Open a node, or close it. Opening one that has not been fetched (or failed last time)
    /// marks it loading and answers with the request to send.
    pub fn toggle(&mut self, conn: DbConnId, id: &str, kind: DbKind) -> Toggled {
        let Some(node) = self.node_mut(conn, id) else {
            return Toggled::Done;
        };
        if !node.expandable() {
            return Toggled::Done;
        }
        if node.expanded {
            node.expanded = false;
            return Toggled::Done;
        }
        node.expanded = true;
        Self::load_if_needed(node, kind)
    }

    fn load_if_needed(node: &mut Node, kind: DbKind) -> Toggled {
        if !matches!(node.load, Load::Idle | Load::Failed(_)) {
            return Toggled::Done;
        }
        match node.request(kind) {
            Some(request) => {
                node.load = Load::Loading;
                Toggled::Ask(request)
            }
            None => Toggled::Done,
        }
    }

    /// Open a connection's root and fetch its databases, unless that is already done or under way.
    pub fn connect(&mut self, conn: DbConnId, kind: DbKind) -> Toggled {
        let Some(root) = self.root_mut(conn) else {
            return Toggled::Done;
        };
        root.expanded = true;
        Self::load_if_needed(root, kind)
    }

    /// Forget what a node has loaded and fetch it again, keeping it open.
    pub fn refresh(&mut self, conn: DbConnId, id: &str, kind: DbKind) -> Toggled {
        let Some(node) = self.node_mut(conn, id) else {
            return Toggled::Done;
        };
        let Some(request) = node.request(kind) else {
            return Toggled::Done;
        };
        node.children.clear();
        node.expanded = true;
        node.load = Load::Loading;
        Toggled::Ask(request)
    }

    /// A connection went away (disconnect, edit): its tree is closed and forgotten.
    pub fn reset(&mut self, conn: DbConnId) {
        if let Some(root) = self.root_mut(conn) {
            root.reset();
        }
    }

    /// The connection came up: a root that was open and had failed (a password had to be typed)
    /// asks again. Answers the request to send.
    pub fn retry_failed(&mut self, conn: DbConnId, kind: DbKind) -> Option<DbNode> {
        let root = self.root_mut(conn)?;
        if !root.expanded || !matches!(root.load, Load::Failed(_)) {
            return None;
        }
        match Self::load_if_needed(root, kind) {
            Toggled::Ask(request) => Some(request),
            Toggled::Done => None,
        }
    }

    /// File a listing under the node that asked for it. A listing for a node the tree no longer
    /// holds (refreshed or collapsed since) is dropped.
    pub fn apply(
        &mut self,
        conn: DbConnId,
        kind: DbKind,
        asked: &DbNode,
        result: Result<DbListing, ubiq_proto::db::DbFailure>,
    ) {
        let Some(root) = self.root_mut(conn) else {
            return;
        };
        let Some(node) = root.answering(kind, asked) else {
            return;
        };
        match result {
            Ok(listing) => {
                let children = fill(node, kind, listing);
                node.children = children;
                node.load = Load::Done;
            }
            Err(failure) => node.load = Load::Failed(failure.message),
        }
    }

    // ── The keyboard ────────────────────────────────────────────────────

    /// Move the cursor `delta` rows through what is visible.
    pub fn step(&mut self, delta: isize) {
        let rows = visible(&self.roots, &self.filter);
        if rows.is_empty() {
            return;
        }
        let at = self
            .selected
            .as_ref()
            .and_then(|id| rows.iter().position(|row| &row.node.id == id));
        let next = match at {
            Some(at) => (at as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
            None if delta < 0 => rows.len() - 1,
            None => 0,
        };
        self.selected = Some(rows[next].node.id.clone());
    }

    /// The index of the cursor in the visible rows.
    pub fn cursor_index(&self) -> Option<usize> {
        let id = self.selected.as_ref()?;
        visible(&self.roots, &self.filter)
            .iter()
            .position(|row| &row.node.id == id)
    }

    /// The id of the visible row above the cursor's with a smaller depth.
    pub fn parent_of_cursor(&self) -> Option<String> {
        let rows = visible(&self.roots, &self.filter);
        let at = self
            .selected
            .as_ref()
            .and_then(|id| rows.iter().position(|row| &row.node.id == id))?;
        let depth = rows[at].depth;
        rows[..at]
            .iter()
            .rev()
            .find(|row| row.depth < depth)
            .map(|row| row.node.id.clone())
    }

    /// The row the menu or a key is about, with the connection state its kind depends on.
    pub fn menu_row(&self, conn: DbConnId, id: &str, state: &DbConnState) -> Option<DbMenuRow> {
        let node = self.node(conn, id)?;
        Some(match &node.kind {
            NodeKind::Connection => DbMenuRow::Connection {
                connected: matches!(state, DbConnState::Connected { .. }),
            },
            NodeKind::Database(_) | NodeKind::Schema { .. } => DbMenuRow::Container,
            NodeKind::Object(obj) if obj.kind.is_relation() => DbMenuRow::Relation,
            NodeKind::Column { .. } => DbMenuRow::Column,
            NodeKind::Object(_) | NodeKind::Group(_) | NodeKind::Note => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::db::{ColumnMeta, ConnectionConfig, DbFailure, DbFailureKind};

    fn object(kind: ObjectKind, name: &str, rows: Option<u64>) -> DbObject {
        DbObject {
            kind,
            name: name.into(),
            schema: Some("public".into()),
            database: Some("shop".into()),
            target: None,
            comment: None,
            approx_rows: rows,
        }
    }

    fn connection(name: &str) -> DbConnection {
        let mut config = ConnectionConfig::new(DbKind::Postgres);
        config.name = name.into();
        DbConnection {
            id: DbConnId::generate(),
            config,
            password: Default::default(),
        }
    }

    /// A Postgres tree loaded down to the tables of `shop.public`, two tables and a view.
    fn loaded() -> (DbTreeState, DbConnId) {
        let conn = connection("staging");
        let id = conn.id;
        let mut tree = DbTreeState::default();
        tree.sync(&[conn]);
        let kind = DbKind::Postgres;
        assert_eq!(
            tree.connect(id, kind),
            Toggled::Ask(DbNode::Databases),
            "a first connect asks for the databases"
        );
        tree.apply(id, kind, &DbNode::Databases, Ok(DbListing::Names(vec!["shop".into()])));
        let db = tree.root(id).unwrap().children[0].id.clone();
        assert_eq!(
            tree.toggle(id, &db, kind),
            Toggled::Ask(DbNode::Schemas {
                database: "shop".into()
            })
        );
        tree.apply(
            id,
            kind,
            &DbNode::Schemas {
                database: "shop".into(),
            },
            Ok(DbListing::Names(vec!["public".into()])),
        );
        let schema = tree.node(id, &db).unwrap().children[0].id.clone();
        let want = DbNode::Objects {
            database: "shop".into(),
            schema: Some("public".into()),
        };
        assert_eq!(tree.toggle(id, &schema, kind), Toggled::Ask(want.clone()));
        tree.apply(
            id,
            kind,
            &want,
            Ok(DbListing::Objects(vec![
                object(ObjectKind::Table, "orders", Some(12_300)),
                object(ObjectKind::Table, "customers", None),
                object(ObjectKind::View, "order_totals", None),
            ])),
        );
        (tree, id)
    }

    fn names(tree: &DbTreeState) -> Vec<String> {
        visible(&tree.roots, &tree.filter)
            .iter()
            .map(|row| row.node.label.clone())
            .collect()
    }

    #[test]
    fn a_loaded_tree_flattens_only_what_is_open() {
        let (mut tree, id) = loaded();
        // Groups arrive closed, so the tables are not rows yet.
        assert_eq!(
            names(&tree),
            ["staging", "shop", "public", "Tables", "Views"]
        );
        let tables = tree.root(id).unwrap().children[0].children[0].children[0]
            .id
            .clone();
        tree.toggle(id, &tables, DbKind::Postgres);
        assert_eq!(
            names(&tree),
            ["staging", "shop", "public", "Tables", "orders", "customers", "Views"]
        );
    }

    #[test]
    fn the_filter_keeps_the_ancestors_of_a_match_and_nothing_else() {
        let (mut tree, _) = loaded();
        tree.filter = "ORDER".into();
        // `orders` and the view `order_totals` match, so their groups and every ancestor stay open
        // to show them; `customers` does not match and goes.
        assert_eq!(
            names(&tree),
            [
                "staging",
                "shop",
                "public",
                "Tables",
                "orders",
                "Views",
                "order_totals"
            ]
        );
        tree.filter = "nothing like it".into();
        assert!(names(&tree).is_empty());
        tree.filter = "  ".into();
        assert_eq!(names(&tree).len(), 5);
    }

    #[test]
    fn a_filter_finds_nothing_it_has_not_loaded() {
        let (mut tree, id) = loaded();
        // The columns of `orders` were never fetched, so `total` is not in memory to find.
        tree.filter = "total".into();
        assert_eq!(
            names(&tree),
            ["staging", "shop", "public", "Views", "order_totals"]
        );
        let orders = tree.root(id).unwrap().children[0].children[0].children[0].children[0]
            .id
            .clone();
        let kind = DbKind::Postgres;
        let want = DbNode::Columns {
            table: TableRef::new(Some("shop"), Some("public"), "orders"),
        };
        assert_eq!(tree.toggle(id, &orders, kind), Toggled::Ask(want.clone()));
        let mut col = ColumnMeta::new(kind, "total", "numeric");
        col.is_pk = false;
        tree.apply(id, kind, &want, Ok(DbListing::Columns(vec![col])));
        assert!(names(&tree).contains(&"total".to_string()));
    }

    #[test]
    fn a_failed_listing_is_retried_by_opening_it_again() {
        let conn = connection("down");
        let id = conn.id;
        let mut tree = DbTreeState::default();
        tree.sync(&[conn]);
        let kind = DbKind::Postgres;
        tree.connect(id, kind);
        let failure = DbFailure::new(DbFailureKind::NeedsPassword, "password needed");
        tree.apply(id, kind, &DbNode::Databases, Err(failure));
        assert_eq!(
            tree.root(id).unwrap().load,
            Load::Failed("password needed".into())
        );
        // The password was typed and the host connected: the open root asks again by itself.
        assert_eq!(tree.retry_failed(id, kind), Some(DbNode::Databases));
        assert_eq!(tree.root(id).unwrap().load, Load::Loading);
        assert_eq!(tree.retry_failed(id, kind), None);
    }

    #[test]
    fn syncing_keeps_a_tree_for_a_connection_that_stays_and_drops_one_that_goes() {
        let (mut tree, id) = loaded();
        let mut renamed = connection("renamed");
        renamed.id = id;
        let other = connection("other");
        tree.sync(&[other.clone(), renamed]);
        assert_eq!(tree.roots[0].label, "other");
        assert_eq!(tree.roots[1].label, "renamed");
        assert!(tree.roots[1].expanded, "its tree survives a rename");
        tree.sync(&[other]);
        assert_eq!(tree.roots.len(), 1);
    }

    #[test]
    fn row_counts_are_humanised() {
        for (n, want) in [
            (0, "~0"),
            (999, "~999"),
            (1_000, "~1k"),
            (12_300, "~12.3k"),
            (123_456, "~123k"),
            (999_500, "~1M"),
            (4_100_000, "~4.1M"),
            (2_500_000_000, "~2.5B"),
        ] {
            assert_eq!(humanise_rows(n), want, "{n}");
        }
        let mut obj = object(ObjectKind::Table, "t", Some(1500));
        assert_eq!(approx_label(&obj).as_deref(), Some("~1.5k"));
        obj.kind = ObjectKind::View;
        assert_eq!(approx_label(&obj), None);
    }

    #[test]
    fn the_menu_offers_each_kind_of_row_its_own_entries() {
        use DbAction::*;
        let connected = DbMenuRow::Connection { connected: true };
        let idle = DbMenuRow::Connection { connected: false };
        assert_eq!(
            db_menu_entries(connected),
            [NewSql, Refresh, Disconnect, Separator, EditConnection, Remove]
        );
        assert_eq!(
            db_menu_entries(idle),
            [Connect, Separator, EditConnection, Remove]
        );
        assert_eq!(db_menu_entries(DbMenuRow::Container), [NewSql, RefreshCounts]);
        assert_eq!(
            db_menu_entries(DbMenuRow::Relation),
            [OpenData, OpenInSql, Separator, CopyName, Refresh]
        );
        assert_eq!(db_menu_entries(DbMenuRow::Column), [CopyName]);
    }

    #[test]
    fn the_menu_row_follows_the_node_and_the_connection_state() {
        let (tree, id) = loaded();
        let up = DbConnState::Connected { server: "16".into() };
        assert_eq!(
            tree.menu_row(id, &id.to_string(), &up),
            Some(DbMenuRow::Connection { connected: true })
        );
        assert_eq!(
            tree.menu_row(id, &id.to_string(), &DbConnState::Idle),
            Some(DbMenuRow::Connection { connected: false })
        );
        let group = &tree.root(id).unwrap().children[0].children[0].children[0];
        assert_eq!(tree.menu_row(id, &group.id, &up), None, "a group has no menu");
        let table = &group.children[0];
        assert_eq!(tree.menu_row(id, &table.id, &up), Some(DbMenuRow::Relation));
    }

    #[test]
    fn the_keyboard_walks_the_visible_rows() {
        let (mut tree, _) = loaded();
        tree.step(1);
        assert_eq!(names(&tree)[tree.cursor_index().unwrap()], "staging");
        tree.step(2);
        assert_eq!(names(&tree)[tree.cursor_index().unwrap()], "public");
        assert_eq!(tree.parent_of_cursor().map(|id| id.len() > 10), Some(true));
        tree.step(100);
        assert_eq!(names(&tree)[tree.cursor_index().unwrap()], "Views");
        tree.step(-100);
        assert_eq!(tree.cursor_index(), Some(0));
    }
}
