//! A SQL result as TOON, the shape an agent reads.
//!
//! TOON writes a table's column names once and then one line per row, which is what makes a result
//! cheap in tokens. The value is built as a [`serde_json::Value`] and handed to
//! [`toon_format::encode`]; nothing here writes TOON by hand.
//!
//! **Cells.** `NULL`, integers, finite floats and booleans are native TOON. Everything else is a
//! string — a decimal in particular, because a float would lose the digits it exists to keep. A
//! string longer than `max_field_size` characters is cut and suffixed `[blob:<id>:<total>]`, the
//! full text kept in [`Blobs`]. Bytes are never inlined: always `[blob:<id>:<bytes>]`.
//!
//! **Columns** are listed as `columns[N]{name,type,nullable}` and a row is an object keyed by the
//! *deduplicated* column name (`id`, `id_2`), because a TOON table is keyed and a join routinely
//! yields two columns of the same name.
//!
//! **The output cap** is [`OUTPUT_CAP`] characters for the whole answer. Past it, rows are dropped
//! from the end of the largest result until it fits, and that statement says `cut_by_output_cap`.

use serde_json::{Map, Value as Json, json};
use ubiq_db::model::ColumnMeta;
use ubiq_db::value::Value;

use super::blobs::{Blob, Blobs};

/// The most characters an answer may have.
pub const OUTPUT_CAP: usize = 200_000;

/// Who the answer is about, and how the connection was opened.
pub struct Header {
    pub connection: String,
    pub engine: String,
    pub read_only: bool,
}

/// What one statement produced.
pub enum Outcome {
    Rows {
        columns: Vec<ColumnMeta>,
        rows: Vec<Vec<Value>>,
        /// The server had more rows than were read.
        more_rows: bool,
    },
    Affected(u64),
    Failed(String),
}

pub struct Statement {
    pub sql: String,
    pub elapsed_ms: u64,
    pub outcome: Outcome,
}

/// One statement's answer, built but not yet size-checked.
struct Built {
    head: Map<String, Json>,
    columns: Vec<Json>,
    rows: Vec<Json>,
    /// Present for a result set: whether the cap has dropped rows.
    has_rows: bool,
    kept: usize,
    cut: bool,
}

impl Built {
    fn value(&self) -> Json {
        let mut object = self.head.clone();
        if self.has_rows {
            if self.cut {
                object.insert("cut_by_output_cap".into(), json!(true));
            }
            object.insert("columns".into(), Json::Array(self.columns.clone()));
            object.insert("rows".into(), Json::Array(self.rows[..self.kept].to_vec()));
        }
        Json::Object(object)
    }
}

/// Encode a run: one statement flat, several as `statements[N]`.
pub fn encode_run(
    header: &Header,
    statements: &[Statement],
    max_field_size: usize,
    blobs: &Blobs,
    agent: &str,
) -> String {
    let multi = statements.len() > 1;
    let mut built: Vec<Built> = statements
        .iter()
        .enumerate()
        .map(|(i, stmt)| build(stmt, i, multi, max_field_size, blobs, agent))
        .collect();

    let assemble = |built: &[Built]| -> Json {
        let mut top = Map::new();
        top.insert("connection".into(), json!(header.connection));
        top.insert("engine".into(), json!(header.engine));
        top.insert("read_only".into(), json!(header.read_only));
        if multi {
            top.insert(
                "statements".into(),
                Json::Array(built.iter().map(Built::value).collect()),
            );
        } else if let Some(only) = built.first()
            && let Json::Object(fields) = only.value()
        {
            top.extend(fields);
        }
        Json::Object(top)
    };

    let mut text = toon(&assemble(&built));
    while text.chars().count() > OUTPUT_CAP {
        // Halve the statement holding the most rows; a result of one row drops it.
        let Some(largest) = (0..built.len())
            .filter(|&i| built[i].kept > 0)
            .max_by_key(|&i| built[i].kept)
        else {
            // Nothing left to drop: the headers alone are over the cap.
            return text.chars().take(OUTPUT_CAP).collect();
        };
        let b = &mut built[largest];
        b.kept /= 2;
        b.cut = true;
        text = toon(&assemble(&built));
    }
    text
}

fn toon(value: &Json) -> String {
    toon_format::encode(value, &toon_format::EncodeOptions::default())
        .unwrap_or_else(|error| format!("error: the result could not be encoded: {error}"))
}

fn build(
    stmt: &Statement,
    index: usize,
    multi: bool,
    max_field_size: usize,
    blobs: &Blobs,
    agent: &str,
) -> Built {
    let mut head = Map::new();
    if multi {
        head.insert("index".into(), json!(index + 1));
        head.insert("sql".into(), json!(preview(&stmt.sql)));
    }
    head.insert("duration_ms".into(), json!(stmt.elapsed_ms));
    let mut built = Built {
        head,
        columns: Vec::new(),
        rows: Vec::new(),
        has_rows: false,
        kept: 0,
        cut: false,
    };
    match &stmt.outcome {
        Outcome::Affected(n) => {
            built.head.insert("affected".into(), json!(n));
        }
        Outcome::Failed(message) => {
            built.head.insert("error".into(), json!(message));
        }
        Outcome::Rows {
            columns,
            rows,
            more_rows,
        } => {
            let names = dedupe(columns.iter().map(|c| c.name.as_str()));
            let mut truncated = 0usize;
            let mut binary = 0usize;
            let rows_json: Vec<Json> = rows
                .iter()
                .map(|row| {
                    let mut object = Map::new();
                    for (name, cell) in names.iter().zip(row) {
                        object.insert(
                            name.clone(),
                            cell_json(
                                cell,
                                max_field_size,
                                blobs,
                                agent,
                                &mut truncated,
                                &mut binary,
                            ),
                        );
                    }
                    Json::Object(object)
                })
                .collect();
            built
                .head
                .insert("row_count".into(), json!(rows_json.len()));
            built.head.insert("more_rows".into(), json!(more_rows));
            built
                .head
                .insert("truncated_fields".into(), json!(truncated));
            built.head.insert("binary_blobs".into(), json!(binary));
            built.columns = columns
                .iter()
                .zip(&names)
                .map(|(c, name)| {
                    json!({"name": name, "type": c.native_type, "nullable": c.nullable})
                })
                .collect();
            built.kept = rows_json.len();
            built.rows = rows_json;
            built.has_rows = true;
        }
    }
    built
}

/// One cell as a TOON-native value.
fn cell_json(
    cell: &Value,
    max_field_size: usize,
    blobs: &Blobs,
    agent: &str,
    truncated: &mut usize,
    binary: &mut usize,
) -> Json {
    match cell {
        Value::Null => Json::Null,
        Value::Bool(b) => json!(b),
        Value::Int(n) => json!(n),
        Value::Float(x) if x.is_finite() => json!(x),
        Value::Float(x) => json!(x.to_string()),
        Value::Bytes(bytes) => {
            *binary += 1;
            let stored = blobs.put(agent, Blob::Bytes(bytes.clone()));
            json!(marker(&stored.id, stored.len))
        }
        other => {
            let text = other.to_string();
            let total = text.chars().count();
            if total <= max_field_size {
                return json!(text);
            }
            *truncated += 1;
            let stored = blobs.put(agent, Blob::Text(text.clone()));
            let mut cut: String = text.chars().take(max_field_size).collect();
            cut.push_str(&marker(&stored.id, stored.len));
            json!(cut)
        }
    }
}

fn marker(id: &str, len: usize) -> String {
    format!("[blob:{id}:{len}]")
}

fn preview(sql: &str) -> String {
    let one_line = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 80 {
        let mut cut: String = one_line.chars().take(80).collect();
        cut.push('…');
        cut
    } else {
        one_line
    }
}

/// `id, id` becomes `id, id_2`, and never collides with a name already taken.
pub fn dedupe<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    names
        .map(|name| {
            let mut candidate = name.to_string();
            let mut n = 1;
            while !taken.insert(candidate.clone()) {
                n += 1;
                candidate = format!("{name}_{n}");
            }
            candidate
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str) -> ColumnMeta {
        ColumnMeta::new(ubiq_db::conn::DbKind::Postgres, name, ty)
    }

    fn header() -> Header {
        Header {
            connection: "main".into(),
            engine: "postgres".into(),
            read_only: true,
        }
    }

    fn rows_stmt(columns: Vec<ColumnMeta>, rows: Vec<Vec<Value>>) -> Statement {
        Statement {
            sql: "select 1".into(),
            elapsed_ms: 12,
            outcome: Outcome::Rows {
                columns,
                rows,
                more_rows: false,
            },
        }
    }

    #[test]
    fn a_result_is_toon_with_native_cells_and_a_sample_shape() {
        let blobs = Blobs::default();
        let stmt = rows_stmt(
            vec![
                col("id", "int4"),
                col("name", "text"),
                col("price", "numeric(10,2)"),
            ],
            vec![
                vec![
                    Value::Int(1),
                    Value::Text("ann".into()),
                    Value::Decimal("12.50".into()),
                ],
                vec![Value::Int(2), Value::Null, Value::Decimal("0.10".into())],
            ],
        );
        let out = encode_run(&header(), &[stmt], 100, &blobs, "a");
        println!("{out}");
        assert!(out.contains("connection: main"), "{out}");
        assert!(out.contains("read_only: true"), "{out}");
        assert!(out.contains("columns[3]{name,type,nullable}:"), "{out}");
        assert!(out.contains("rows[2]{id,name,price}:"), "{out}");
        assert!(
            out.contains("1,ann,\"12.50\"") || out.contains("1,ann,12.50"),
            "{out}"
        );
        assert!(out.contains("2,null,"), "{out}");
    }

    #[test]
    fn a_decimal_stays_a_string_and_a_float_stays_a_number() {
        let blobs = Blobs::default();
        let cell = |v| cell_json(&v, 100, &blobs, "a", &mut 0, &mut 0);
        assert_eq!(cell(Value::Decimal("1.10".into())), json!("1.10"));
        assert_eq!(cell(Value::Float(1.5)), json!(1.5));
        assert_eq!(cell(Value::Float(f64::NAN)), json!("NaN"));
        assert_eq!(cell(Value::Bool(true)), json!(true));
        assert_eq!(cell(Value::Date("2026-01-02".into())), json!("2026-01-02"));
        assert_eq!(cell(Value::Null), Json::Null);
    }

    #[test]
    fn long_text_is_cut_and_the_whole_is_in_the_blob_cache() {
        let blobs = Blobs::default();
        let mut truncated = 0;
        let cell = cell_json(
            &Value::Text("abcdefghij".into()),
            4,
            &blobs,
            "a",
            &mut truncated,
            &mut 0,
        );
        let text = cell.as_str().unwrap();
        assert!(text.starts_with("abcd[blob:b"), "{text}");
        assert!(text.ends_with(":10]"), "{text}");
        assert_eq!(truncated, 1);
        let id = text
            .trim_start_matches("abcd[blob:")
            .split(':')
            .next()
            .unwrap();
        let full = blobs.get("a", id).unwrap();
        assert_eq!(*full, Blob::Text("abcdefghij".into()));
        // Exactly at the limit is not cut.
        assert_eq!(
            cell_json(&Value::Text("abcd".into()), 4, &blobs, "a", &mut 0, &mut 0),
            json!("abcd")
        );
    }

    #[test]
    fn the_cut_counts_characters_not_bytes() {
        let blobs = Blobs::default();
        let cell = cell_json(&Value::Text("ééééé".into()), 2, &blobs, "a", &mut 0, &mut 0);
        assert!(cell.as_str().unwrap().starts_with("éé[blob:"));
        assert!(cell.as_str().unwrap().ends_with(":5]"));
    }

    #[test]
    fn binary_is_always_a_blob_even_when_small() {
        let blobs = Blobs::default();
        let mut binary = 0;
        let cell = cell_json(
            &Value::Bytes(vec![1, 2, 3]),
            100_000,
            &blobs,
            "a",
            &mut 0,
            &mut binary,
        );
        let text = cell.as_str().unwrap();
        assert!(
            text.starts_with("[blob:b") && text.ends_with(":3]"),
            "{text}"
        );
        assert_eq!(binary, 1);
    }

    #[test]
    fn duplicate_column_names_are_renamed_and_keep_their_order() {
        assert_eq!(
            dedupe(["id", "id", "name", "id"].into_iter()),
            ["id", "id_2", "name", "id_3"]
        );
        // A real `id_2` already there is not clobbered.
        assert_eq!(
            dedupe(["id", "id_2", "id"].into_iter()),
            ["id", "id_2", "id_3"]
        );
        let blobs = Blobs::default();
        let stmt = rows_stmt(
            vec![col("id", "int"), col("id", "int")],
            vec![vec![Value::Int(1), Value::Int(2)]],
        );
        let out = encode_run(&header(), &[stmt], 10, &blobs, "a");
        assert!(out.contains("rows[1]{id,id_2}:"), "{out}");
    }

    #[test]
    fn several_statements_are_a_list_and_a_write_says_affected() {
        let blobs = Blobs::default();
        let stmts = vec![
            Statement {
                sql: "update t\n set a = 1".into(),
                elapsed_ms: 3,
                outcome: Outcome::Affected(4),
            },
            Statement {
                sql: "bogus".into(),
                elapsed_ms: 1,
                outcome: Outcome::Failed("syntax error".into()),
            },
        ];
        let out = encode_run(&header(), &stmts, 10, &blobs, "a");
        println!("{out}");
        assert!(out.contains("statements[2]"), "{out}");
        assert!(out.contains("affected: 4"), "{out}");
        assert!(out.contains("update t set a = 1"), "{out}");
        assert!(out.contains("error: syntax error"), "{out}");
    }

    #[test]
    fn the_output_cap_drops_trailing_rows_and_says_so() {
        let blobs = Blobs::default();
        let rows: Vec<Vec<Value>> = (0..20_000)
            .map(|i| vec![Value::Int(i), Value::Text("x".repeat(40))])
            .collect();
        let stmt = rows_stmt(vec![col("n", "int"), col("s", "text")], rows);
        let out = encode_run(&header(), &[stmt], 100, &blobs, "a");
        assert!(out.chars().count() <= OUTPUT_CAP);
        assert!(out.contains("cut_by_output_cap: true"), "{}", &out[..300]);
        assert!(
            out.contains("row_count: 20000"),
            "row_count is what was read"
        );
        // The kept rows are a prefix.
        assert!(out.contains("\n  0,"), "{}", &out[..400]);
    }

    #[test]
    fn a_small_result_is_not_marked_cut() {
        let blobs = Blobs::default();
        let stmt = rows_stmt(vec![col("n", "int")], vec![vec![Value::Int(1)]]);
        let out = encode_run(&header(), &[stmt], 10, &blobs, "a");
        assert!(!out.contains("cut_by_output_cap"));
    }
}
