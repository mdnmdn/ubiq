//! DBML — a database's structure as the model the drivers' introspection fills
//! (`Connection::structure`), and [`to_dbml`], which writes it as [DBML](https://dbml.dbdiagram.io/docs/).
//!
//! The model is what a reverse-engineered schema needs and no more: schemas, tables and their
//! comments, columns (type, nullable, default, key, unique, increment, comment), indexes,
//! foreign keys (composite, with their actions) and enums.
//!
//! # How the DBML is written
//!
//! - **The default schema is not written.** A table in `public` (Postgres) or `dbo` (SQL Server)
//!   is `users`, one anywhere else `audit.users`. MySQL and SQLite have no schema level: the
//!   database *is* the namespace, and it is never written either.
//! - A single-column primary key is the column's `pk`; a composite one is `(a, b) [pk]` under
//!   `Indexes`. A single-column unique index is the column's `unique`, and its name is not kept.
//! - An index method is written as `type: hash`; `btree` is DBML's default and left out; any other
//!   (`gin`, `gist`, `fulltext`, …) becomes the index's `note`, since DBML has no word for it.
//! - A foreign key whose action is `no action` leaves that action out. A foreign key is written
//!   only when both of its tables are in the export.
//! - A MySQL `enum(...)` column gets an `Enum` of its own, named `<table>_<column>_enum`.
//! - Identifiers that are not plain words, or that are DBML keywords, are double-quoted; notes are
//!   single-quoted (triple-quoted when they span lines) with their quotes escaped.

use serde::{Deserialize, Serialize};

use crate::conn::DbKind;

/// What part of a database to describe.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructureScope {
    /// One schema; `None` is every schema bar the system ones. Ignored on an engine without a
    /// schema level (MySQL, SQLite).
    pub schema: Option<String>,
    /// These tables only; empty is every table. An entry is `name` or `schema.name`.
    pub tables: Vec<String>,
}

impl StructureScope {
    /// Whether the table `schema.name` is in scope.
    pub fn keeps(&self, schema: Option<&str>, name: &str) -> bool {
        self.tables.is_empty()
            || self
                .tables
                .iter()
                .any(|t| t == name || schema.is_some_and(|s| t.split_once('.') == Some((s, name))))
    }
}

/// A database's structure.
#[derive(Clone, Debug, PartialEq)]
pub struct Structure {
    pub kind: DbKind,
    pub database: String,
    /// The schemas the tables and enums are in, sorted; empty without a schema level.
    pub schemas: Vec<String>,
    pub tables: Vec<Table>,
    pub refs: Vec<ForeignKey>,
    pub enums: Vec<EnumType>,
}

/// A schema-qualified name; `schema` is `None` without a schema level.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QualifiedName {
    pub schema: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub schema: Option<String>,
    pub name: String,
    pub comment: Option<String>,
    /// In ordinal order.
    pub columns: Vec<Column>,
    /// Every index, the primary key's included (`pk`).
    pub indexes: Vec<Index>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    /// As the engine declares it (`varchar(255)`, `int unsigned`); empty when SQLite has none.
    pub native_type: String,
    /// The enum this column's type is, when it is one; it replaces `native_type` in the DBML.
    pub enum_type: Option<QualifiedName>,
    pub nullable: bool,
    pub pk: bool,
    /// A single-column unique index (not the primary key) covers it.
    pub unique: bool,
    pub auto_increment: bool,
    pub default: Option<DefaultValue>,
    pub comment: Option<String>,
}

/// A column default, sorted into what DBML can say about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefaultValue {
    Number(String),
    Text(String),
    Bool(bool),
    Null,
    /// Anything else, as the catalog prints it (`now()`, `CURRENT_TIMESTAMP`).
    Expr(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexPart {
    Column(String),
    Expr(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    /// `None` when the engine named it itself (SQLite's autoindexes, MySQL's `PRIMARY`).
    pub name: Option<String>,
    pub parts: Vec<IndexPart>,
    pub unique: bool,
    pub pk: bool,
    /// The access method, lowercase (`btree`, `hash`, `gin`, `fulltext`); `None` where the
    /// engine has one kind (SQLite) or it says nothing useful (SQL Server).
    pub method: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignKey {
    pub name: Option<String>,
    pub from: QualifiedName,
    pub from_columns: Vec<String>,
    pub to: QualifiedName,
    pub to_columns: Vec<String>,
    /// Lowercase: `cascade`, `restrict`, `set null`, `set default`, `no action`.
    pub on_delete: Option<String>,
    pub on_update: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumType {
    pub schema: Option<String>,
    pub name: String,
    pub values: Vec<String>,
}

// ── reading the catalog's text ───────────────────────────────────────────────────────────────

/// A default as the catalog prints it, sorted. `generated` is MySQL's `DEFAULT_GENERATED`: the
/// text is an expression.
pub fn parse_default(kind: DbKind, raw: &str, generated: bool) -> Option<DefaultValue> {
    let mut s = raw.trim();
    if kind == DbKind::MySql {
        if raw.is_empty() {
            return Some(DefaultValue::Text(String::new())); // MySQL 8 prints `DEFAULT ''` so
        }
        if generated {
            return Some(DefaultValue::Expr(s.to_string()));
        }
        if s.eq_ignore_ascii_case("null") {
            return None; // MariaDB's "no default"
        }
    }
    if s.is_empty() {
        return None;
    }
    // SQL Server, and SQLite for an expression, wrap the default in parentheses
    while let Some(inner) = outer_parens(s) {
        s = inner.trim();
    }
    if let Some(text) = quoted(s) {
        return Some(DefaultValue::Text(text));
    }
    // Postgres: `NULL::text`, `0::numeric`
    let head = s.split_once("::").map_or(s, |(head, _)| head.trim());
    if is_number(head) {
        return Some(DefaultValue::Number(head.to_string()));
    }
    match head.to_ascii_lowercase().as_str() {
        "true" => return Some(DefaultValue::Bool(true)),
        "false" => return Some(DefaultValue::Bool(false)),
        "null" => return Some(DefaultValue::Null),
        _ => {}
    }
    // MySQL 8 prints a literal default unquoted
    if kind == DbKind::MySql && !mysql_expression(s) {
        return Some(DefaultValue::Text(s.to_string()));
    }
    Some(DefaultValue::Expr(s.to_string()))
}

/// The labels of a MySQL `enum('a','b')` type; `None` when it is not one.
pub fn enum_labels(native_type: &str) -> Option<Vec<String>> {
    let t = native_type.trim();
    let head = t.get(..5)?;
    if !head.eq_ignore_ascii_case("enum(") || !t.ends_with(')') {
        return None;
    }
    let body = &t[5..t.len() - 1];
    let mut labels = Vec::new();
    let mut chars = body.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        if chars.next()? != '\'' {
            return None;
        }
        let mut label = String::new();
        loop {
            match chars.next()? {
                '\'' if chars.peek() == Some(&'\'') => {
                    chars.next();
                    label.push('\'');
                }
                '\'' => break,
                '\\' => label.push(chars.next()?),
                c => label.push(c),
            }
        }
        labels.push(label);
        if chars.peek().is_none() {
            return Some(labels);
        }
    }
}

/// The inside of `( … )` when the first parenthesis closes at the very end.
fn outer_parens(s: &str) -> Option<&str> {
    if !s.starts_with('(') || !s.ends_with(')') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_quote = false;
    for (i, c) in s.char_indices() {
        match c {
            '\'' => in_quote = !in_quote,
            '(' if !in_quote => depth += 1,
            ')' if !in_quote => {
                depth -= 1;
                if depth == 0 {
                    return (i == s.len() - 1).then(|| &s[1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The text of `'…'` (or SQL Server's `N'…'`), optionally followed by a Postgres `::type` cast.
fn quoted(s: &str) -> Option<String> {
    let s = s
        .strip_prefix('N')
        .filter(|r| r.starts_with('\''))
        .unwrap_or(s);
    let mut chars = s.strip_prefix('\'')?.char_indices().peekable();
    let mut text = String::new();
    while let Some((i, c)) = chars.next() {
        if c == '\'' {
            if chars.peek().is_some_and(|(_, n)| *n == '\'') {
                chars.next();
                text.push('\'');
                continue;
            }
            let rest = s[1 + i + 1..].trim_start();
            return (rest.is_empty() || rest.starts_with("::")).then_some(text);
        }
        text.push(c);
    }
    None
}

fn is_number(s: &str) -> bool {
    let digits = s.trim_start_matches(['-', '+']);
    digits.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        && digits.chars().any(|c| c.is_ascii_digit())
        && s.parse::<f64>().is_ok()
}

fn mysql_expression(s: &str) -> bool {
    const WORDS: [&str; 6] = [
        "current_timestamp",
        "current_date",
        "current_time",
        "localtime",
        "localtimestamp",
        "now",
    ];
    s.ends_with(')') || WORDS.iter().any(|w| s.eq_ignore_ascii_case(w))
}

// ── writing ──────────────────────────────────────────────────────────────────────────────────

/// The structure as DBML: enums, then tables, then references.
pub fn to_dbml(s: &Structure) -> String {
    let default_schema = s.kind.default_schema();
    let qualify = |schema: Option<&str>, name: &str| match schema {
        Some(sc) if Some(sc) != default_schema => format!("{}.{}", ident(sc), ident(name)),
        _ => ident(name),
    };
    let mut blocks: Vec<String> = Vec::new();

    for e in &s.enums {
        let mut out = format!("Enum {} {{\n", qualify(e.schema.as_deref(), &e.name));
        for v in &e.values {
            out.push_str(&format!("  {}\n", ident(v)));
        }
        out.push('}');
        blocks.push(out);
    }

    for t in &s.tables {
        let mut out = format!("Table {} {{\n", qualify(t.schema.as_deref(), &t.name));
        let single_pk = t.columns.iter().filter(|c| c.pk).count() == 1;
        for c in &t.columns {
            let ty = match &c.enum_type {
                Some(e) => qualify(e.schema.as_deref(), &e.name),
                None => type_name(&c.native_type),
            };
            let mut settings = Vec::new();
            let pk = c.pk && single_pk;
            if pk {
                settings.push("pk".to_string());
            }
            if c.auto_increment {
                settings.push("increment".to_string());
            }
            if !c.nullable && !pk {
                settings.push("not null".to_string());
            }
            if c.unique && !pk {
                settings.push("unique".to_string());
            }
            if let Some(d) = &c.default {
                settings.push(format!("default: {}", default_value(d)));
            }
            if let Some(note) = &c.comment {
                settings.push(format!("note: {}", string(note)));
            }
            out.push_str(&format!("  {} {ty}{}\n", ident(&c.name), list(&settings)));
        }
        let indexes: Vec<String> = t
            .indexes
            .iter()
            .filter(|i| !written_on_column(t, i))
            .map(index_line)
            .collect();
        if !indexes.is_empty() {
            out.push_str("\n  Indexes {\n");
            for line in indexes {
                out.push_str(&format!("    {line}\n"));
            }
            out.push_str("  }\n");
        }
        if let Some(note) = &t.comment {
            out.push_str(&format!("\n  Note: {}\n", string(note)));
        }
        out.push('}');
        blocks.push(out);
    }

    let refs: Vec<String> = s
        .refs
        .iter()
        .map(|r| {
            let end = |n: &QualifiedName, cols: &[String]| {
                let table = qualify(n.schema.as_deref(), &n.name);
                match cols {
                    [one] => format!("{table}.{}", ident(one)),
                    many => format!(
                        "{table}.({})",
                        many.iter().map(|c| ident(c)).collect::<Vec<_>>().join(", ")
                    ),
                }
            };
            let mut settings = Vec::new();
            for (word, action) in [("delete", &r.on_delete), ("update", &r.on_update)] {
                if let Some(a) = action.as_deref().filter(|a| *a != "no action") {
                    settings.push(format!("{word}: {a}"));
                }
            }
            let name = r
                .name
                .as_deref()
                .map_or_else(String::new, |n| format!(" {}", ident(n)));
            format!(
                "Ref{name}: {} > {}{}",
                end(&r.from, &r.from_columns),
                end(&r.to, &r.to_columns),
                list(&settings)
            )
        })
        .collect();
    if !refs.is_empty() {
        blocks.push(refs.join("\n"));
    }

    let mut out = blocks.join("\n\n");
    out.push('\n');
    out
}

/// A single-column primary key or unique index is said on the column instead.
fn written_on_column(t: &Table, i: &Index) -> bool {
    let [IndexPart::Column(col)] = i.parts.as_slice() else {
        return false;
    };
    let column = t.columns.iter().find(|c| &c.name == col);
    (i.pk && t.columns.iter().filter(|c| c.pk).count() == 1)
        || (i.unique && !i.pk && column.is_some_and(|c| c.unique))
}

fn index_line(i: &Index) -> String {
    let part = |p: &IndexPart| match p {
        IndexPart::Column(c) => ident(c),
        IndexPart::Expr(e) => format!("`{}`", e.replace('`', "\\`")),
    };
    let target = match i.parts.as_slice() {
        [one] => part(one),
        many => format!("({})", many.iter().map(part).collect::<Vec<_>>().join(", ")),
    };
    let mut settings = Vec::new();
    if i.pk {
        settings.push("pk".to_string());
    } else {
        if i.unique {
            settings.push("unique".to_string());
        }
        if let Some(name) = &i.name {
            settings.push(format!("name: {}", string(name)));
        }
        match i.method.as_deref() {
            None | Some("btree") => {}
            Some("hash") => settings.push("type: hash".to_string()),
            Some(other) => settings.push(format!("note: {}", string(&format!("using {other}")))),
        }
    }
    format!("{target}{}", list(&settings))
}

fn list(settings: &[String]) -> String {
    if settings.is_empty() {
        String::new()
    } else {
        format!(" [{}]", settings.join(", "))
    }
}

fn default_value(d: &DefaultValue) -> String {
    match d {
        DefaultValue::Number(n) => n.clone(),
        DefaultValue::Text(t) => string(t),
        DefaultValue::Bool(b) => b.to_string(),
        DefaultValue::Null => "null".to_string(),
        DefaultValue::Expr(e) => format!("`{}`", e.replace('`', "\\`")),
    }
}

/// Words DBML reads as keywords where a name could stand.
const KEYWORDS: [&str; 8] = [
    "table",
    "ref",
    "enum",
    "note",
    "indexes",
    "project",
    "tablegroup",
    "as",
];

fn plain_word(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A name, double-quoted unless it is a plain word.
fn ident(s: &str) -> String {
    if plain_word(s) && !KEYWORDS.iter().any(|k| s.eq_ignore_ascii_case(k)) {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// A type: bare when it is `word`, `word(args)` or `word[]`, double-quoted otherwise
/// (`int unsigned`, `timestamp with time zone`).
fn type_name(native: &str) -> String {
    let t = native.trim();
    if t.is_empty() {
        return "any".to_string(); // SQLite lets a column have no declared type
    }
    let base = t.trim_end_matches("[]");
    let (word, args) = match base.split_once('(') {
        Some((word, rest)) => (word, Some(rest)),
        None => (base, None),
    };
    let args_ok = args
        .is_none_or(|a| a.ends_with(')') && !a[..a.len() - 1].contains(['(', ')', '"', '\'', '`']));
    if plain_word(word) && args_ok {
        t.to_string()
    } else {
        format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// A note or a string default, single-quoted; triple-quoted when it spans lines.
fn string(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\");
    if s.contains('\n') {
        format!("'''{}'''", escaped.replace("'''", "\\'''"))
    } else {
        format!("'{}'", escaped.replace('\'', "\\'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str) -> Column {
        Column {
            name: name.into(),
            native_type: ty.into(),
            enum_type: None,
            nullable: true,
            pk: false,
            unique: false,
            auto_increment: false,
            default: None,
            comment: None,
        }
    }

    fn name(schema: &str, n: &str) -> QualifiedName {
        QualifiedName {
            schema: Some(schema.into()),
            name: n.into(),
        }
    }

    fn sample() -> Structure {
        let mut id = col("id", "integer");
        id.pk = true;
        id.nullable = false;
        id.auto_increment = true;
        let mut email = col("email", "character varying(255)");
        email.nullable = false;
        email.unique = true;
        email.comment = Some("the user's login".into());
        let mut mood = col("mood", "mood");
        mood.enum_type = Some(name("public", "mood"));
        mood.default = Some(DefaultValue::Text("ok".into()));
        let mut created = col("created_at", "timestamp with time zone");
        created.default = Some(DefaultValue::Expr("now()".into()));
        let users = Table {
            schema: Some("public".into()),
            name: "users".into(),
            comment: Some("People\nwho log in".into()),
            columns: vec![id, email, mood, created],
            indexes: vec![
                Index {
                    name: Some("users_pkey".into()),
                    parts: vec![IndexPart::Column("id".into())],
                    unique: true,
                    pk: true,
                    method: Some("btree".into()),
                },
                Index {
                    name: Some("users_email_key".into()),
                    parts: vec![IndexPart::Column("email".into())],
                    unique: true,
                    pk: false,
                    method: Some("btree".into()),
                },
                Index {
                    name: Some("users_lower_email".into()),
                    parts: vec![IndexPart::Expr("lower((email)::text)".into())],
                    unique: false,
                    pk: false,
                    method: Some("hash".into()),
                },
            ],
        };
        let mut a = col("user_id", "integer");
        a.pk = true;
        a.nullable = false;
        let mut b = col("role", "text");
        b.pk = true;
        b.nullable = false;
        let mut weight = col("weight", "numeric(5,2)");
        weight.default = Some(DefaultValue::Number("1.0".into()));
        let mut order = col("order", "int");
        order.default = Some(DefaultValue::Null);
        let roles = Table {
            schema: Some("audit".into()),
            name: "user roles".into(),
            comment: None,
            columns: vec![a, b, weight, order],
            indexes: vec![
                Index {
                    name: Some("user_roles_pkey".into()),
                    parts: vec![
                        IndexPart::Column("user_id".into()),
                        IndexPart::Column("role".into()),
                    ],
                    unique: true,
                    pk: true,
                    method: Some("btree".into()),
                },
                Index {
                    name: Some("roles_by_weight".into()),
                    parts: vec![
                        IndexPart::Column("role".into()),
                        IndexPart::Column("weight".into()),
                    ],
                    unique: true,
                    pk: false,
                    method: Some("gin".into()),
                },
            ],
        };
        Structure {
            kind: DbKind::Postgres,
            database: "app".into(),
            schemas: vec!["audit".into(), "public".into()],
            tables: vec![users, roles],
            refs: vec![ForeignKey {
                name: Some("roles_user_fk".into()),
                from: name("audit", "user roles"),
                from_columns: vec!["user_id".into()],
                to: name("public", "users"),
                to_columns: vec!["id".into()],
                on_delete: Some("cascade".into()),
                on_update: Some("no action".into()),
            }],
            enums: vec![EnumType {
                schema: Some("public".into()),
                name: "mood".into(),
                values: vec!["ok".into(), "so so".into()],
            }],
        }
    }

    #[test]
    fn a_structure_as_dbml() {
        let expected = r#"Enum mood {
  ok
  "so so"
}

Table users {
  id integer [pk, increment]
  email "character varying(255)" [not null, unique, note: 'the user\'s login']
  mood mood [default: 'ok']
  created_at "timestamp with time zone" [default: `now()`]

  Indexes {
    `lower((email)::text)` [name: 'users_lower_email', type: hash]
  }

  Note: '''People
who log in'''
}

Table audit."user roles" {
  user_id integer [not null]
  role text [not null]
  weight numeric(5,2) [default: 1.0]
  order int [default: null]

  Indexes {
    (user_id, role) [pk]
    (role, weight) [unique, name: 'roles_by_weight', note: 'using gin']
  }
}

Ref roles_user_fk: audit."user roles".user_id > users.id [delete: cascade]
"#;
        assert_eq!(to_dbml(&sample()), expected);
    }

    #[test]
    fn composite_refs_and_no_schema_level() {
        let mut s = sample();
        s.kind = DbKind::MySql;
        s.tables.clear();
        s.enums.clear();
        s.refs = vec![ForeignKey {
            name: None,
            from: QualifiedName {
                schema: None,
                name: "line".into(),
            },
            from_columns: vec!["order_id".into(), "n".into()],
            to: QualifiedName {
                schema: None,
                name: "Table".into(),
            },
            to_columns: vec!["id".into(), "n".into()],
            on_delete: Some("set null".into()),
            on_update: Some("restrict".into()),
        }];
        assert_eq!(
            to_dbml(&s),
            "Ref: line.(order_id, n) > \"Table\".(id, n) [delete: set null, update: restrict]\n"
        );
    }

    #[test]
    fn defaults_are_sorted() {
        use DefaultValue::*;
        let pg = |raw| parse_default(DbKind::Postgres, raw, false);
        assert_eq!(pg("'ab''c'::character varying"), Some(Text("ab'c".into())));
        assert_eq!(pg("0"), Some(Number("0".into())));
        assert_eq!(pg("-1.5"), Some(Number("-1.5".into())));
        assert_eq!(pg("true"), Some(Bool(true)));
        assert_eq!(pg("NULL::text"), Some(Null));
        assert_eq!(pg("now()"), Some(Expr("now()".into())));
        let ms = |raw| parse_default(DbKind::MsSql, raw, false);
        assert_eq!(ms("((0))"), Some(Number("0".into())));
        assert_eq!(ms("(N'x')"), Some(Text("x".into())));
        assert_eq!(ms("(getdate())"), Some(Expr("getdate()".into())));
        assert_eq!(ms("(a)+(b)"), Some(Expr("(a)+(b)".into())));
        let my = |raw, generated| parse_default(DbKind::MySql, raw, generated);
        assert_eq!(my("abc", false), Some(Text("abc".into())));
        assert_eq!(my("'abc'", false), Some(Text("abc".into())));
        assert_eq!(my("", false), Some(Text(String::new())));
        assert_eq!(my("NULL", false), None);
        assert_eq!(
            my("current_timestamp()", false),
            Some(Expr("current_timestamp()".into()))
        );
        assert_eq!(
            my("CURRENT_TIMESTAMP", true),
            Some(Expr("CURRENT_TIMESTAMP".into()))
        );
        let lite = |raw| parse_default(DbKind::Sqlite, raw, false);
        assert_eq!(
            lite("(datetime('now'))"),
            Some(Expr("datetime('now')".into()))
        );
        assert_eq!(
            lite("CURRENT_TIMESTAMP"),
            Some(Expr("CURRENT_TIMESTAMP".into()))
        );
        assert_eq!(lite(""), None);
    }

    #[test]
    fn mysql_enum_labels() {
        assert_eq!(
            enum_labels("enum('a','b''c','d e')"),
            Some(vec!["a".into(), "b'c".into(), "d e".into()])
        );
        assert_eq!(enum_labels("varchar(3)"), None);
        assert_eq!(enum_labels("ENUM('x')"), Some(vec!["x".into()]));
    }

    #[test]
    fn scope_matches_bare_and_qualified_names() {
        let all = StructureScope::default();
        assert!(all.keeps(Some("s"), "t"));
        let some = StructureScope {
            schema: None,
            tables: vec!["t".into(), "audit.log".into()],
        };
        assert!(some.keeps(Some("public"), "t"));
        assert!(some.keeps(Some("audit"), "log"));
        assert!(!some.keeps(Some("public"), "log"));
        assert!(!some.keeps(None, "log"));
    }

    #[test]
    fn names_types_and_notes_are_quoted() {
        assert_eq!(ident("plain_1"), "plain_1");
        assert_eq!(ident("Note"), "\"Note\"");
        assert_eq!(ident("a\"b"), "\"a\\\"b\"");
        assert_eq!(type_name("varchar(255)"), "varchar(255)");
        assert_eq!(type_name("integer[]"), "integer[]");
        assert_eq!(type_name("int unsigned"), "\"int unsigned\"");
        assert_eq!(type_name(""), "any");
        assert_eq!(string("it's \\ ok"), "'it\\'s \\\\ ok'");
    }
}
