//! `Connection::structure` — the catalog queries that fill a [`Structure`], one set per engine, and
//! the assembly they share. Every engine answers five row sets of text cells in the same column
//! layout (below); the driver only runs them, through its own unlogged catalog path.
//!
//! | Engine | Comments | Enums | Index method | Not covered |
//! |---|---|---|---|---|
//! | PostgreSQL | `obj_description` / `col_description` | `pg_enum` | `pg_am.amname` | partitions (the parent is written), partial-index predicates, `INCLUDE` columns |
//! | MySQL / MariaDB | `TABLE_COMMENT` / `COLUMN_COMMENT` | `enum(...)` inline, one `Enum` per column | `INDEX_TYPE` | functional key parts (the index is left out), foreign keys to another database |
//! | SQLite | none — the engine has none | none | none | expression indexes (left out) |
//! | SQL Server | `MS_Description` extended properties | none | none | included columns, filtered-index predicates |
//!
//! Views, sequences, triggers and check constraints are never written.

use std::collections::{HashMap, HashSet};

use super::Result;
use crate::conn::DbKind;
use crate::dbml::{
    Column, EnumType, ForeignKey, Index, IndexPart, QualifiedName, Structure, StructureScope,
    Table, enum_labels, parse_default,
};

/// Rows of a catalog query, every cell as text.
pub(super) type Rows = Vec<Vec<Option<String>>>;

// Row layouts:
// - tables:  schema, table, comment
// - columns: schema, table, column, type, nullable 1/0, default, auto-increment 1/0, comment,
//            enum schema, enum name, default-is-an-expression 1/0, primary-key ordinal (0 = not)
// - indexes: schema, table, index, column, expression, unique 1/0, primary 1/0, method
//            — ordered by table, index, key position
// - refs:    group key, name, schema, table, column, target schema, target table, target column,
//            on delete, on update — ordered by table, key, position
// - enums:   schema, name, label — ordered by name, sort order

/// The five queries of one engine.
pub(super) struct Queries {
    tables: String,
    columns: String,
    indexes: String,
    refs: String,
    enums: Option<String>,
}

/// The structure of `database`, from the engine's queries run through `run`.
pub(super) fn introspect(
    kind: DbKind,
    database: &str,
    scope: &StructureScope,
    mut run: impl FnMut(&str) -> Result<Rows>,
) -> Result<Structure> {
    let q = queries(kind, database, scope);
    let tables = run(&q.tables)?;
    let columns = run(&q.columns)?;
    let indexes = run(&q.indexes)?;
    let refs = run(&q.refs)?;
    let enums = match &q.enums {
        Some(sql) => run(sql)?,
        None => Vec::new(),
    };
    Ok(assemble(
        kind, database, scope, &tables, &columns, &indexes, &refs, &enums,
    ))
}

fn queries(kind: DbKind, database: &str, scope: &StructureScope) -> Queries {
    match kind {
        DbKind::Postgres => postgres(scope),
        DbKind::MySql => mysql(database),
        DbKind::Sqlite => sqlite(database),
        DbKind::MsSql => mssql(database, scope),
    }
}

// ── the engines ──────────────────────────────────────────────────────────────────────────────

fn postgres(scope: &StructureScope) -> Queries {
    let lit = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let sp = |n: &str| match &scope.schema {
        Some(s) => format!("{n}.nspname = {}", lit(s)),
        None => format!("{n}.nspname <> 'information_schema' AND {n}.nspname !~ '^pg_'"),
    };
    let rel = "c.relkind IN ('r', 'p', 'f') AND NOT c.relispartition";
    let action = |col: &str| {
        format!(
            "CASE {col} WHEN 'r' THEN 'restrict' WHEN 'c' THEN 'cascade' WHEN 'n' THEN 'set null' \
             WHEN 'd' THEN 'set default' ELSE 'no action' END"
        )
    };
    Queries {
        tables: format!(
            "SELECT n.nspname::text, c.relname::text, obj_description(c.oid, 'pg_class') \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE {rel} AND {} ORDER BY 1, 2",
            sp("n")
        ),
        columns: format!(
            "SELECT n.nspname::text, c.relname::text, a.attname::text, \
               format_type(a.atttypid, a.atttypmod), \
               CASE WHEN a.attnotnull THEN '0' ELSE '1' END, \
               CASE WHEN a.attgenerated = '' THEN pg_get_expr(d.adbin, d.adrelid) END, \
               CASE WHEN a.attidentity IN ('a', 'd') \
                 OR pg_get_expr(d.adbin, d.adrelid) LIKE 'nextval(%' THEN '1' ELSE '0' END, \
               col_description(c.oid, a.attnum), \
               CASE WHEN t.typtype = 'e' THEN tn.nspname::text END, \
               CASE WHEN t.typtype = 'e' THEN t.typname::text END, '0', '0' \
             FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_type t ON t.oid = a.atttypid JOIN pg_namespace tn ON tn.oid = t.typnamespace \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE {rel} AND a.attnum > 0 AND NOT a.attisdropped AND {} \
             ORDER BY 1, 2, a.attnum",
            sp("n")
        ),
        indexes: format!(
            "SELECT n.nspname::text, c.relname::text, ic.relname::text, a.attname::text, \
               CASE WHEN k.attnum = 0 THEN pg_get_indexdef(i.indexrelid, k.ord::int, true) END, \
               CASE WHEN i.indisunique THEN '1' ELSE '0' END, \
               CASE WHEN i.indisprimary THEN '1' ELSE '0' END, am.amname::text \
             FROM pg_index i JOIN pg_class c ON c.oid = i.indrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_class ic ON ic.oid = i.indexrelid JOIN pg_am am ON am.oid = ic.relam \
             CROSS JOIN LATERAL unnest(i.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord) \
             LEFT JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum AND k.attnum > 0 \
             WHERE {rel} AND k.ord <= i.indnkeyatts AND {} \
             ORDER BY 1, 2, 3, k.ord",
            sp("n")
        ),
        refs: format!(
            "SELECT con.oid::text, con.conname::text, n.nspname::text, c.relname::text, \
               a.attname::text, fn.nspname::text, fc.relname::text, fa.attname::text, {}, {} \
             FROM pg_constraint con JOIN pg_class c ON c.oid = con.conrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_class fc ON fc.oid = con.confrelid JOIN pg_namespace fn ON fn.oid = fc.relnamespace \
             CROSS JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY AS k(col, fcol, ord) \
             JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.col \
             JOIN pg_attribute fa ON fa.attrelid = con.confrelid AND fa.attnum = k.fcol \
             WHERE con.contype = 'f' AND {rel} AND {} \
             ORDER BY 3, 4, 2, 1, k.ord",
            action("con.confdeltype"),
            action("con.confupdtype"),
            sp("n")
        ),
        enums: Some(format!(
            "SELECT n.nspname::text, t.typname::text, e.enumlabel::text \
             FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid \
             JOIN pg_namespace n ON n.oid = t.typnamespace \
             WHERE {} ORDER BY 1, 2, e.enumsortorder",
            sp("n")
        )),
    }
}

fn mysql(database: &str) -> Queries {
    let d = format!("'{}'", database.replace('\\', "\\\\").replace('\'', "''"));
    Queries {
        tables: format!(
            "SELECT NULL, TABLE_NAME, TABLE_COMMENT FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = {d} AND TABLE_TYPE IN ('BASE TABLE', 'SYSTEM VERSIONED') \
             ORDER BY TABLE_NAME"
        ),
        columns: format!(
            "SELECT NULL, TABLE_NAME, COLUMN_NAME, COLUMN_TYPE, IF(IS_NULLABLE = 'YES', '1', '0'), \
               COLUMN_DEFAULT, IF(EXTRA LIKE '%auto_increment%', '1', '0'), COLUMN_COMMENT, \
               NULL, NULL, IF(EXTRA LIKE '%DEFAULT_GENERATED%', '1', '0'), '0' \
             FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = {d} \
             ORDER BY TABLE_NAME, ORDINAL_POSITION"
        ),
        indexes: format!(
            "SELECT NULL, TABLE_NAME, INDEX_NAME, COLUMN_NAME, NULL, IF(NON_UNIQUE = 0, '1', '0'), \
               IF(INDEX_NAME = 'PRIMARY', '1', '0'), INDEX_TYPE \
             FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = {d} \
             ORDER BY TABLE_NAME, INDEX_NAME, SEQ_IN_INDEX"
        ),
        refs: format!(
            "SELECT k.CONSTRAINT_NAME, k.CONSTRAINT_NAME, NULL, k.TABLE_NAME, k.COLUMN_NAME, \
               IF(k.REFERENCED_TABLE_SCHEMA = k.TABLE_SCHEMA, NULL, k.REFERENCED_TABLE_SCHEMA), \
               k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, r.DELETE_RULE, r.UPDATE_RULE \
             FROM information_schema.KEY_COLUMN_USAGE k \
             JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
               ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA \
              AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME \
             WHERE k.TABLE_SCHEMA = {d} AND k.REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY k.TABLE_NAME, k.CONSTRAINT_NAME, k.ORDINAL_POSITION"
        ),
        enums: None,
    }
}

fn sqlite(database: &str) -> Queries {
    let db = format!("\"{}\"", database.replace('"', "\"\""));
    let dl = format!("'{}'", database.replace('\'', "''"));
    let user = "m.type = 'table' AND m.name NOT LIKE 'sqlite\\_%' ESCAPE '\\'";
    Queries {
        tables: format!(
            "SELECT NULL, m.name, NULL FROM {db}.sqlite_master m WHERE {user} ORDER BY m.name"
        ),
        columns: format!(
            "SELECT NULL, m.name, p.name, p.type, \
               CASE WHEN p.\"notnull\" = 0 AND p.pk = 0 THEN '1' ELSE '0' END, p.dflt_value, \
               CASE WHEN p.pk = 1 AND lower(p.type) = 'integer' AND (SELECT count(*) \
                 FROM pragma_table_info(m.name, {dl}) q WHERE q.pk > 0) = 1 THEN '1' ELSE '0' END, \
               NULL, NULL, NULL, '0', p.pk \
             FROM {db}.sqlite_master m JOIN pragma_table_xinfo(m.name, {dl}) p \
             WHERE {user} AND p.hidden <> 1 ORDER BY m.name, p.cid"
        ),
        indexes: format!(
            "SELECT NULL, m.name, il.name, ii.name, NULL, \
               CASE WHEN il.\"unique\" THEN '1' ELSE '0' END, \
               CASE WHEN il.origin = 'pk' THEN '1' ELSE '0' END, NULL \
             FROM {db}.sqlite_master m JOIN pragma_index_list(m.name, {dl}) il \
             JOIN pragma_index_info(il.name, {dl}) ii \
             WHERE {user} ORDER BY m.name, il.name, ii.seqno"
        ),
        refs: format!(
            "SELECT f.id, NULL, NULL, m.name, f.\"from\", NULL, f.\"table\", f.\"to\", \
               f.on_delete, f.on_update \
             FROM {db}.sqlite_master m JOIN pragma_foreign_key_list(m.name, {dl}) f \
             WHERE {user} ORDER BY m.name, f.id, f.seq"
        ),
        enums: None,
    }
}

fn mssql(database: &str, scope: &StructureScope) -> Queries {
    let db = format!("[{}]", database.replace(']', "]]"));
    let name = |col: &str| format!("CAST({col} AS nvarchar(128))");
    let bit =
        |col: &str| format!("CAST(CASE WHEN {col} = 1 THEN N'1' ELSE N'0' END AS nvarchar(1))");
    let null = "CAST(NULL AS nvarchar(1))";
    let sp = match &scope.schema {
        Some(s) => format!("s.name = N'{}'", s.replace('\'', "''")),
        None => "s.name NOT IN (N'sys', N'INFORMATION_SCHEMA')".to_string(),
    };
    let user = format!("t.is_ms_shipped = 0 AND {sp}");
    let tables = format!("{db}.sys.tables t JOIN {db}.sys.schemas s ON s.schema_id = t.schema_id");
    let note = |major: &str, minor: &str| {
        format!(
            "LEFT JOIN {db}.sys.extended_properties ep ON ep.class = 1 \
             AND ep.major_id = {major} AND ep.minor_id = {minor} AND ep.name = N'MS_Description'"
        )
    };
    let action = |col: &str| format!("CAST(REPLACE(LOWER({col}), N'_', N' ') AS nvarchar(60))");
    Queries {
        tables: format!(
            "SELECT {}, {}, CAST(ep.value AS nvarchar(4000)) FROM {tables} {} \
             WHERE {user} ORDER BY 1, 2",
            name("s.name"),
            name("t.name"),
            note("t.object_id", "0")
        ),
        columns: format!(
            "SELECT {}, {}, {}, {}, {}, CAST(dc.definition AS nvarchar(4000)), {}, \
               CAST(ep.value AS nvarchar(4000)), {null}, {null}, N'0', N'0' \
             FROM {db}.sys.columns c JOIN {tables} ON t.object_id = c.object_id \
             JOIN {db}.sys.types ty ON ty.user_type_id = c.user_type_id \
             LEFT JOIN {db}.sys.default_constraints dc ON dc.object_id = c.default_object_id {} \
             WHERE {user} ORDER BY 1, 2, c.column_id",
            name("s.name"),
            name("t.name"),
            name("c.name"),
            super::mssql::NATIVE_TYPE,
            bit("c.is_nullable"),
            bit("c.is_identity"),
            note("c.object_id", "c.column_id")
        ),
        indexes: format!(
            "SELECT {}, {}, {}, {}, {null}, {}, {}, {null} \
             FROM {db}.sys.indexes i JOIN {tables} ON t.object_id = i.object_id \
             JOIN {db}.sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
             JOIN {db}.sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
             WHERE i.type > 0 AND ic.is_included_column = 0 AND {user} \
             ORDER BY 1, 2, 3, ic.key_ordinal",
            name("s.name"),
            name("t.name"),
            name("i.name"),
            name("c.name"),
            bit("i.is_unique"),
            bit("i.is_primary_key")
        ),
        refs: format!(
            "SELECT {}, {}, {}, {}, {}, {}, {}, {}, {}, {} \
             FROM {db}.sys.foreign_keys fk \
             JOIN {db}.sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
             JOIN {tables} ON t.object_id = fk.parent_object_id \
             JOIN {db}.sys.columns c ON c.object_id = fkc.parent_object_id \
               AND c.column_id = fkc.parent_column_id \
             JOIN {db}.sys.tables rt ON rt.object_id = fk.referenced_object_id \
             JOIN {db}.sys.schemas rs ON rs.schema_id = rt.schema_id \
             JOIN {db}.sys.columns rc ON rc.object_id = fkc.referenced_object_id \
               AND rc.column_id = fkc.referenced_column_id \
             WHERE {user} ORDER BY 3, 4, 1, fkc.constraint_column_id",
            name("fk.name"),
            name("fk.name"),
            name("s.name"),
            name("t.name"),
            name("c.name"),
            name("rs.name"),
            name("rt.name"),
            name("rc.name"),
            action("fk.delete_referential_action_desc"),
            action("fk.update_referential_action_desc")
        ),
        enums: None,
    }
}

// ── the assembly ─────────────────────────────────────────────────────────────────────────────

fn at(row: &[Option<String>], i: usize) -> Option<&str> {
    row.get(i)
        .and_then(Option::as_deref)
        .filter(|s| !s.is_empty())
}

fn raw(row: &[Option<String>], i: usize) -> Option<&str> {
    row.get(i).and_then(Option::as_deref)
}

fn on(row: &[Option<String>], i: usize) -> bool {
    at(row, i) == Some("1")
}

type Key = (Option<String>, String);

fn key(row: &[Option<String>], schema: usize, table: usize) -> Option<Key> {
    Some((
        at(row, schema).map(str::to_string),
        at(row, table)?.to_string(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    kind: DbKind,
    database: &str,
    scope: &StructureScope,
    tables: &Rows,
    columns: &Rows,
    indexes: &Rows,
    refs: &Rows,
    enums: &Rows,
) -> Structure {
    let mut out: Vec<Table> = tables
        .iter()
        .filter_map(|r| {
            let (schema, name) = key(r, 0, 1)?;
            scope.keeps(schema.as_deref(), &name).then(|| Table {
                schema,
                name,
                comment: at(r, 2).map(str::to_string),
                columns: Vec::new(),
                indexes: Vec::new(),
            })
        })
        .collect();
    let place: HashMap<Key, usize> = out
        .iter()
        .enumerate()
        .map(|(i, t)| ((t.schema.clone(), t.name.clone()), i))
        .collect();

    // columns, and SQLite's primary-key order (it has no index for a rowid key)
    let mut pk_order: HashMap<usize, Vec<(u32, String)>> = HashMap::new();
    for r in columns {
        let Some(&t) = key(r, 0, 1).and_then(|k| place.get(&k)) else {
            continue;
        };
        let Some(name) = at(r, 2) else { continue };
        let native_type = at(r, 3).unwrap_or_default().to_string();
        let enum_type = at(r, 9).map(|n| QualifiedName {
            schema: at(r, 8).map(str::to_string),
            name: n.to_string(),
        });
        let mut column = Column {
            name: name.to_string(),
            native_type,
            enum_type,
            nullable: on(r, 4),
            pk: false,
            unique: false,
            auto_increment: on(r, 6),
            default: raw(r, 5).and_then(|d| parse_default(kind, d, on(r, 10))),
            comment: at(r, 7).map(str::to_string),
        };
        if column.auto_increment
            && matches!(&column.default, Some(crate::dbml::DefaultValue::Expr(e)) if e.starts_with("nextval("))
        {
            column.default = None; // a serial's sequence is what `increment` says
        }
        if let Some(ord) = at(r, 11)
            .and_then(|o| o.parse::<u32>().ok())
            .filter(|o| *o > 0)
        {
            pk_order
                .entry(t)
                .or_default()
                .push((ord, column.name.clone()));
        }
        out[t].columns.push(column);
    }

    // indexes, grouped by consecutive (table, name); one with a part nobody can name is dropped
    let mut last: Option<(usize, String)> = None;
    let mut broken: HashSet<(usize, usize)> = HashSet::new();
    for r in indexes {
        let Some(&t) = key(r, 0, 1).and_then(|k| place.get(&k)) else {
            continue;
        };
        let Some(index_name) = at(r, 2) else { continue };
        let part = match (at(r, 3), at(r, 4)) {
            (Some(c), _) => Some(IndexPart::Column(c.to_string())),
            (None, Some(e)) => Some(IndexPart::Expr(e.to_string())),
            _ => None,
        };
        if last.as_ref() != Some(&(t, index_name.to_string())) {
            let pk = on(r, 6);
            let engine_named =
                index_name.starts_with("sqlite_autoindex_") || (kind == DbKind::MySql && pk);
            out[t].indexes.push(Index {
                name: (!engine_named).then(|| index_name.to_string()),
                parts: Vec::new(),
                unique: on(r, 5),
                pk,
                method: at(r, 7).map(str::to_ascii_lowercase),
            });
            last = Some((t, index_name.to_string()));
        }
        let i = out[t].indexes.len() - 1;
        match part {
            Some(p) => out[t].indexes[i].parts.push(p),
            None => {
                broken.insert((t, i));
            }
        }
    }
    for (t, table) in out.iter_mut().enumerate() {
        let mut i = 0;
        table.indexes.retain(|_| {
            i += 1;
            !broken.contains(&(t, i - 1))
        });
    }

    // the key and the unique flags
    for (t, table) in out.iter_mut().enumerate() {
        if !table.indexes.iter().any(|i| i.pk)
            && let Some(mut order) = pk_order.remove(&t)
        {
            order.sort();
            table.indexes.insert(
                0,
                Index {
                    name: None,
                    parts: order
                        .into_iter()
                        .map(|(_, c)| IndexPart::Column(c))
                        .collect(),
                    unique: true,
                    pk: true,
                    method: None,
                },
            );
        }
        let mut pk = HashSet::new();
        let mut unique = HashSet::new();
        for index in &table.indexes {
            let cols: Vec<&String> = index
                .parts
                .iter()
                .filter_map(|p| match p {
                    IndexPart::Column(c) => Some(c),
                    IndexPart::Expr(_) => None,
                })
                .collect();
            if index.pk {
                pk.extend(cols.iter().map(|c| c.to_string()));
            } else if index.unique && index.parts.len() == 1 && cols.len() == 1 {
                unique.insert(cols[0].to_string());
            }
        }
        for c in &mut table.columns {
            c.pk = pk.contains(&c.name);
            c.unique = !c.pk && unique.contains(&c.name);
        }
        table.indexes.sort_by_key(|i| !i.pk); // the key first; stable otherwise
    }

    // enums: the engine's, then MySQL's inline ones
    let mut enum_types: Vec<EnumType> = Vec::new();
    for r in enums {
        let (Some(name), Some(label)) = (at(r, 1), raw(r, 2)) else {
            continue;
        };
        let schema = at(r, 0).map(str::to_string);
        match enum_types.last_mut() {
            Some(e) if e.name == name && e.schema == schema => e.values.push(label.to_string()),
            _ => enum_types.push(EnumType {
                schema,
                name: name.to_string(),
                values: vec![label.to_string()],
            }),
        }
    }
    if kind == DbKind::MySql {
        for table in &mut out {
            for c in &mut table.columns {
                if let Some(values) = enum_labels(&c.native_type) {
                    let name = format!("{}_{}_enum", table.name, c.name);
                    c.enum_type = Some(QualifiedName {
                        schema: None,
                        name: name.clone(),
                    });
                    enum_types.push(EnumType {
                        schema: None,
                        name,
                        values,
                    });
                }
            }
        }
    }
    if !scope.tables.is_empty() {
        let used: HashSet<QualifiedName> = out
            .iter()
            .flat_map(|t| t.columns.iter().filter_map(|c| c.enum_type.clone()))
            .collect();
        enum_types.retain(|e| {
            used.contains(&QualifiedName {
                schema: e.schema.clone(),
                name: e.name.clone(),
            })
        });
    }

    // foreign keys, grouped by consecutive (table, key); both ends must be in the export
    let mut fks: Vec<(Key, String, ForeignKey)> = Vec::new();
    for r in refs {
        let (Some(from), Some(group), Some(column), Some(target)) =
            (key(r, 2, 3), at(r, 0), at(r, 4), at(r, 6))
        else {
            continue;
        };
        let to = (at(r, 5).map(str::to_string), target.to_string());
        let action = |i: usize| at(r, i).map(|a| a.to_ascii_lowercase().replace('_', " "));
        let same = fks.last().is_some_and(|(k, g, _)| *k == from && g == group);
        if !same {
            fks.push((
                from.clone(),
                group.to_string(),
                ForeignKey {
                    name: at(r, 1).map(str::to_string),
                    from: QualifiedName {
                        schema: from.0.clone(),
                        name: from.1.clone(),
                    },
                    from_columns: Vec::new(),
                    to: QualifiedName {
                        schema: to.0,
                        name: to.1,
                    },
                    to_columns: Vec::new(),
                    on_delete: action(8),
                    on_update: action(9),
                },
            ));
        }
        let fk = &mut fks.last_mut().expect("just pushed").2;
        fk.from_columns.push(column.to_string());
        if let Some(c) = at(r, 7) {
            fk.to_columns.push(c.to_string());
        }
    }
    let pk_of = |k: &Key| -> Vec<String> {
        place
            .get(k)
            .map(|&t| {
                out[t]
                    .indexes
                    .iter()
                    .find(|i| i.pk)
                    .map(|i| {
                        i.parts
                            .iter()
                            .filter_map(|p| match p {
                                IndexPart::Column(c) => Some(c.clone()),
                                IndexPart::Expr(_) => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    };
    let refs: Vec<ForeignKey> = fks
        .into_iter()
        .filter_map(|(from, _, mut fk)| {
            let to: Key = (fk.to.schema.clone(), fk.to.name.clone());
            if !place.contains_key(&from) || !place.contains_key(&to) {
                return None;
            }
            if fk.to_columns.is_empty() {
                fk.to_columns = pk_of(&to); // SQLite: `REFERENCES t` means t's key
            }
            (fk.to_columns.len() == fk.from_columns.len()).then_some(fk)
        })
        .collect();

    let mut schemas: Vec<String> = out
        .iter()
        .filter_map(|t| t.schema.clone())
        .chain(enum_types.iter().filter_map(|e| e.schema.clone()))
        .collect();
    schemas.sort();
    schemas.dedup();
    Structure {
        kind,
        database: database.to_string(),
        schemas,
        tables: out,
        refs,
        enums: enum_types,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[Option<&str>]) -> Vec<Option<String>> {
        cells.iter().map(|c| c.map(str::to_string)).collect()
    }

    /// The assembly on its own, as SQL Server would answer: a composite key, a filtered scope.
    #[test]
    fn rows_become_a_structure() {
        let s = Some;
        let tables = vec![
            row(&[s("dbo"), s("a"), s("first")]),
            row(&[s("dbo"), s("b"), None]),
            row(&[s("x"), s("c"), None]),
        ];
        let col = |t: &'static str, c: &'static str| {
            row(&[
                s("dbo"),
                s(t),
                s(c),
                s("int"),
                s("0"),
                s("((0))"),
                s("0"),
                None,
                None,
                None,
                s("0"),
                s("0"),
            ])
        };
        let columns = vec![
            col("a", "id"),
            col("a", "n"),
            col("b", "a_id"),
            col("b", "n"),
        ];
        let indexes = vec![
            row(&[
                s("dbo"),
                s("a"),
                s("pk_a"),
                s("id"),
                None,
                s("1"),
                s("1"),
                None,
            ]),
            row(&[
                s("dbo"),
                s("a"),
                s("pk_a"),
                s("n"),
                None,
                s("1"),
                s("1"),
                None,
            ]),
        ];
        let refs = vec![
            row(&[
                s("fk"),
                s("fk"),
                s("dbo"),
                s("b"),
                s("a_id"),
                s("dbo"),
                s("a"),
                s("id"),
                s("cascade"),
                s("no action"),
            ]),
            row(&[
                s("fk"),
                s("fk"),
                s("dbo"),
                s("b"),
                s("n"),
                s("dbo"),
                s("a"),
                s("n"),
                s("cascade"),
                s("no action"),
            ]),
        ];
        let scope = StructureScope {
            schema: None,
            tables: vec!["a".into(), "dbo.b".into()],
        };
        let st = assemble(
            DbKind::MsSql,
            "app",
            &scope,
            &tables,
            &columns,
            &indexes,
            &refs,
            &Vec::new(),
        );
        assert_eq!(st.tables.len(), 2);
        assert!(st.tables[0].columns.iter().all(|c| c.pk));
        assert_eq!(st.refs.len(), 1);
        assert_eq!(st.refs[0].from_columns, vec!["a_id", "n"]);
        assert_eq!(
            crate::dbml::to_dbml(&st),
            "Table a {\n  id int [not null, default: 0]\n  n int [not null, default: 0]\n\n  \
             Indexes {\n    (id, n) [pk]\n  }\n\n  Note: 'first'\n}\n\n\
             Table b {\n  a_id int [not null, default: 0]\n  n int [not null, default: 0]\n}\n\n\
             Ref fk: b.(a_id, n) > a.(id, n) [delete: cascade]\n"
        );
    }
}
