//! `value` — the portable column type model, cell values, and local validation of typed input.
//!
//! [`map_native`] reduces each engine's type name to a [`DataType`]; [`parse_input`] checks what a
//! user entered into a grid cell (text, or `None` for an explicit NULL) against a column and
//! returns the [`Value`] to write — on a text column `''` and NULL are different inputs. Dates and
//! times are carried as validated ISO text, not a date library type: every engine accepts that
//! text as a literal, and the grid shows it unchanged.

use std::fmt;

use crate::conn::DbKind;
use crate::model::ColumnMeta;
use serde::{Deserialize, Serialize};

/// A column type, reduced to what validation and the SQL builder need.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataType {
    Bool,
    /// Integer of `bits` (8, 16, 24, 32, 64). `unsigned`: MySQL `UNSIGNED`, SQL Server `tinyint`.
    Int {
        bits: u8,
        unsigned: bool,
    },
    /// Exact numeric. `None` means unconstrained (Postgres bare `numeric`).
    Decimal {
        precision: Option<u16>,
        scale: Option<u16>,
    },
    /// `real`, `double`, `float` — any width.
    Float,
    /// Variable-length text; `max_len` in characters, `None` = unbounded / unknown.
    Text {
        max_len: Option<u32>,
    },
    /// Fixed-length text (`char(n)`).
    Char {
        len: Option<u32>,
    },
    Uuid,
    Date,
    /// Time of day (with or without time zone).
    Time,
    /// Timestamp without offset.
    DateTime,
    /// Timestamp with offset (`timestamptz`, `datetimeoffset`).
    DateTimeTz,
    /// `json`, `jsonb`.
    Json,
    Bytes,
    /// The allowed labels, case preserved.
    Enum(Vec<String>),
    /// Anything else, with the native name as given: accepted as text, unvalidated.
    Other(String),
}

impl DataType {
    /// The length limit of `Text` / `Char`, if any.
    pub fn max_len(&self) -> Option<u32> {
        match self {
            DataType::Text { max_len } => *max_len,
            DataType::Char { len } => *len,
            _ => None,
        }
    }

    /// Numeric types — the grid right-aligns them.
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            DataType::Int { .. } | DataType::Decimal { .. } | DataType::Float
        )
    }

    /// Text-like types: whitespace in the input is data and an empty input is the empty string
    /// `''`, not NULL (see [`parse_input`]). Everything else trims, and empty means NULL.
    pub fn is_text_like(&self) -> bool {
        matches!(
            self,
            DataType::Text { .. } | DataType::Char { .. } | DataType::Other(_)
        )
    }
}

/// Map an engine's native type name to a [`DataType`]. Case-insensitive; takes the forms both
/// the catalogs and DDL print: `varchar(255)`, `character varying`, `numeric(10, 2)`,
/// `int(11) unsigned`, `timestamp(3) with time zone`, `nvarchar(max)`, `enum('a','b')`.
///
/// Engine specifics: MySQL `tinyint(1)` and `bit(1)` are `Bool`; SQL Server `timestamp` is
/// `rowversion` (`Bytes`) and `tinyint` is unsigned; SQLite follows its affinity rules after the
/// common names (`boolean`, `date`, `datetime`, `uuid`, `json`) — and since SQLite enforces
/// nothing, local validation is stricter than the engine. Postgres arrays and every unknown
/// name are `Other`.
pub fn map_native(kind: DbKind, native_type: &str) -> DataType {
    let raw = native_type.trim();
    let (base, args, suffix) = split_type(raw);
    let mut words: Vec<&str> = base
        .split_whitespace()
        .chain(suffix.split_whitespace())
        .collect();
    let unsigned = words.contains(&"unsigned");
    words.retain(|w| !matches!(*w, "unsigned" | "signed" | "zerofill"));
    let name = words.join(" ");
    let n0 = args.first().and_then(|a| a.trim().parse::<u32>().ok());
    let n1 = args.get(1).and_then(|a| a.trim().parse::<u32>().ok());
    let int = |bits| DataType::Int { bits, unsigned };
    let dec = |p: Option<u32>, s: Option<u32>| DataType::Decimal {
        precision: p.map(|v| v as u16),
        scale: s.map(|v| v as u16),
    };
    let other = || DataType::Other(raw.to_string());
    if raw.ends_with("[]") || (kind == DbKind::Postgres && name.starts_with('_')) {
        return other();
    }
    match kind {
        DbKind::Postgres => match name.as_str() {
            "smallint" | "int2" | "smallserial" | "serial2" => int(16),
            "integer" | "int" | "int4" | "serial" | "serial4" => int(32),
            "bigint" | "int8" | "bigserial" | "serial8" => int(64),
            "boolean" | "bool" => DataType::Bool,
            "numeric" | "decimal" => dec(n0, if n0.is_some() { n1.or(Some(0)) } else { None }),
            "real" | "float4" | "double precision" | "float8" | "float" => DataType::Float,
            "text" | "citext" | "name" => DataType::Text { max_len: None },
            "character varying" | "varchar" => DataType::Text { max_len: n0 },
            "character" | "char" => DataType::Char {
                len: n0.or(Some(1)),
            },
            "bpchar" => DataType::Char { len: n0 },
            "uuid" => DataType::Uuid,
            "date" => DataType::Date,
            "time" | "time without time zone" | "timetz" | "time with time zone" => DataType::Time,
            "timestamp" | "timestamp without time zone" => DataType::DateTime,
            "timestamptz" | "timestamp with time zone" => DataType::DateTimeTz,
            "json" | "jsonb" => DataType::Json,
            "bytea" => DataType::Bytes,
            _ => other(),
        },
        DbKind::MySql => match name.as_str() {
            "tinyint" if n0 == Some(1) && !unsigned => DataType::Bool,
            "tinyint" => int(8),
            "bool" | "boolean" => DataType::Bool,
            "smallint" => int(16),
            "mediumint" => int(24),
            "int" | "integer" => int(32),
            "bigint" => int(64),
            "serial" => DataType::Int {
                bits: 64,
                unsigned: true,
            },
            "bit" if n0.unwrap_or(1) == 1 => DataType::Bool,
            "decimal" | "numeric" | "dec" | "fixed" => dec(n0.or(Some(10)), n1.or(Some(0))),
            "float" | "double" | "double precision" | "real" => DataType::Float,
            "char" | "nchar" | "national char" => DataType::Char {
                len: n0.or(Some(1)),
            },
            "varchar" | "nvarchar" | "national varchar" | "character varying" => {
                DataType::Text { max_len: n0 }
            }
            "tinytext" => DataType::Text { max_len: Some(255) },
            "text" => DataType::Text {
                max_len: Some(65_535),
            },
            "mediumtext" => DataType::Text {
                max_len: Some(16_777_215),
            },
            "longtext" => DataType::Text { max_len: None },
            "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" => {
                DataType::Bytes
            }
            "date" => DataType::Date,
            "time" => DataType::Time,
            "datetime" | "timestamp" => DataType::DateTime,
            "json" => DataType::Json,
            "enum" => DataType::Enum(args.iter().map(|a| unquote(a)).collect()),
            _ => other(),
        },
        DbKind::MsSql => match name.as_str() {
            "bit" => DataType::Bool,
            "tinyint" => DataType::Int {
                bits: 8,
                unsigned: true,
            },
            "smallint" => int(16),
            "int" => int(32),
            "bigint" => int(64),
            "decimal" | "numeric" => dec(n0.or(Some(18)), n1.or(Some(0))),
            "money" => dec(Some(19), Some(4)),
            "smallmoney" => dec(Some(10), Some(4)),
            "float" | "real" => DataType::Float,
            "char" | "nchar" => DataType::Char {
                len: n0.or(Some(1)),
            },
            "varchar" | "nvarchar" => DataType::Text { max_len: n0 },
            "text" | "ntext" => DataType::Text { max_len: None },
            "uniqueidentifier" => DataType::Uuid,
            "date" => DataType::Date,
            "time" => DataType::Time,
            "datetime" | "datetime2" | "smalldatetime" => DataType::DateTime,
            "datetimeoffset" => DataType::DateTimeTz,
            "binary" | "varbinary" | "image" | "timestamp" | "rowversion" => DataType::Bytes,
            _ => other(),
        },
        DbKind::Sqlite => match name.as_str() {
            "" => other(),
            "boolean" | "bool" => DataType::Bool,
            "date" => DataType::Date,
            "datetime" | "timestamp" => DataType::DateTime,
            "time" => DataType::Time,
            "uuid" | "guid" => DataType::Uuid,
            "json" => DataType::Json,
            "decimal" | "numeric" => dec(n0, if n0.is_some() { n1.or(Some(0)) } else { None }),
            n if n.contains("int") => int(64),
            n if n.contains("char") || n.contains("clob") || n.contains("text") => {
                DataType::Text { max_len: n0 }
            }
            n if n.contains("blob") => DataType::Bytes,
            n if n.contains("real") || n.contains("floa") || n.contains("doub") => DataType::Float,
            _ => other(),
        },
    }
}

/// `name(args) suffix` → (lowercased name, raw args split on top-level commas, lowercased suffix).
fn split_type(raw: &str) -> (String, Vec<String>, String) {
    let Some(open) = raw.find('(') else {
        return (raw.to_ascii_lowercase(), Vec::new(), String::new());
    };
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quote = false;
    let mut close = raw.len();
    let mut chars = raw[open + 1..].char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\'' if quote && chars.peek().is_some_and(|(_, n)| *n == '\'') => {
                cur.push_str("''");
                chars.next();
            }
            '\'' => {
                quote = !quote;
                cur.push(c);
            }
            ',' if !quote => args.push(std::mem::take(&mut cur)),
            ')' if !quote => {
                close = open + 1 + i;
                break;
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !args.is_empty() {
        args.push(cur);
    }
    let suffix = raw.get(close + 1..).unwrap_or("").to_ascii_lowercase();
    (raw[..open].to_ascii_lowercase(), args, suffix)
}

/// `'a''b'` → `a'b`; anything unquoted is trimmed.
fn unquote(a: &str) -> String {
    let a = a.trim();
    match a.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => a.to_string(),
    }
}

/// A cell. Temporal and uuid values are validated text in canonical form (see [`parse_input`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Exact numeric as normalised text (`-12.50`). Also a `BIGINT UNSIGNED` above `i64::MAX`.
    Decimal(String),
    /// Text, char, enum labels, and every `DataType::Other`.
    Text(String),
    Bytes(Vec<u8>),
    /// Lowercase, hyphenated.
    Uuid(String),
    /// `YYYY-MM-DD`.
    Date(String),
    /// `HH:MM[:SS[.fraction]]`.
    Time(String),
    /// `YYYY-MM-DD HH:MM[:SS[.fraction]]`.
    DateTime(String),
    /// As `DateTime`, plus an optional offset `Z` / `±HH[:MM]`.
    DateTimeTz(String),
    /// Syntactically valid JSON, as typed.
    Json(String),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The text an editor starts from: like `Display`, but `Null` is empty. Loses the difference
    /// between NULL and `''`; an editor that keeps it uses [`Value::input_text`].
    pub fn edit_text(&self) -> String {
        match self {
            Value::Null => String::new(),
            v => v.to_string(),
        }
    }

    /// The editor's input for this value: `None` for NULL, else the text. Feeding it back to
    /// [`parse_input`] yields the same value.
    pub fn input_text(&self) -> Option<String> {
        (!self.is_null()).then(|| self.to_string())
    }
}

impl fmt::Display for Value {
    /// Grid text: `NULL`, `true`/`false`, numbers as Rust prints them, bytes as `0x…` lowercase
    /// hex, everything else verbatim.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("NULL"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Bytes(b) => {
                f.write_str("0x")?;
                b.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
            }
            Value::Decimal(s)
            | Value::Text(s)
            | Value::Uuid(s)
            | Value::Date(s)
            | Value::Time(s)
            | Value::DateTime(s)
            | Value::DateTimeTz(s)
            | Value::Json(s) => f.write_str(s),
        }
    }
}

/// A cell's text failed its column's type or constraints.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{column}: {message}")]
pub struct ValidationError {
    pub column: String,
    /// User-facing, starts lowercase (`not an integer`).
    pub message: String,
}

/// Validate `text` (what the user typed) against `col` and return the value to write, treating
/// empty text on a nullable column as NULL — the legacy rule, kept for callers that have only a
/// string. An editor that can tell `''` from NULL calls [`parse_input`] instead.
pub fn parse_cell(text: &str, col: &ColumnMeta) -> Result<Value, ValidationError> {
    let blank = if col.data_type.is_text_like() {
        text.is_empty()
    } else {
        text.trim().is_empty()
    };
    parse_input((!(blank && col.nullable)).then_some(text), col)
}

/// Validate a cell input against `col` and return the value to write. `None` is NULL — an explicit
/// state of the editor, never inferred from empty text.
///
/// - `None`: `Null` if the column is nullable, else an error (`required`).
/// - Empty text on a text / char / other column: the empty string `''`, nullable or not.
/// - Empty text (after trimming) on any other column: `Null` if nullable, else an error.
///   Leaving an identity or defaulted column to the server is the editor's choice made before
///   calling this.
/// - Bool: `true/false`, `t/f`, `1/0`, `yes/no`, `y/n`, `on/off`, any case.
/// - Int: decimal integer within the range of `bits` / `unsigned`.
/// - Decimal: plain `[-+]digits[.digits]` (no exponent) within precision and scale.
/// - Float: anything finite `f64` parses.
/// - Text / Char: at most `max_len` characters. Not trimmed.
/// - Uuid: 32 hex digits, hyphens and braces optional → lowercase hyphenated.
/// - Date `YYYY-MM-DD` (real calendar day); Time `HH:MM[:SS[.f{1,9}]]`; DateTime a date, `T` or
///   space, a time — a bare date gets ` 00:00:00`; DateTimeTz as DateTime plus optional
///   `Z`/`±HH[:MM]`. The separator is normalised to a space.
/// - Json: any valid JSON document. Bytes: `0x…` or `\x…` hex. Enum: one of the labels (case
///   folded to the label's spelling). Other: accepted verbatim.
pub fn parse_input(input: Option<&str>, col: &ColumnMeta) -> Result<Value, ValidationError> {
    let fail = |message: String| ValidationError {
        column: col.name.clone(),
        message,
    };
    let required = || fail("required — the column is NOT NULL".into());
    let ty = &col.data_type;
    let Some(text) = input else {
        return if col.nullable {
            Ok(Value::Null)
        } else {
            Err(required())
        };
    };
    let t = if ty.is_text_like() { text } else { text.trim() };
    if t.is_empty() {
        if ty.is_text_like() {
            return Ok(Value::Text(String::new()));
        }
        return if col.nullable {
            Ok(Value::Null)
        } else {
            Err(required())
        };
    }
    match ty {
        DataType::Bool => match t.to_ascii_lowercase().as_str() {
            "true" | "t" | "1" | "yes" | "y" | "on" => Ok(Value::Bool(true)),
            "false" | "f" | "0" | "no" | "n" | "off" => Ok(Value::Bool(false)),
            _ => Err(fail(format!(
                "`{t}` is not a boolean (true/false, 1/0, yes/no)"
            ))),
        },
        DataType::Int { bits, unsigned } => {
            let n: i128 = t
                .parse()
                .map_err(|_| fail(format!("`{t}` is not an integer")))?;
            let bits = u32::from((*bits).clamp(1, 64));
            let (min, max) = if *unsigned {
                (0, (1i128 << bits) - 1)
            } else {
                (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
            };
            if n < min || n > max {
                return Err(fail(format!("{n} is out of range {min}..={max}")));
            }
            Ok(i64::try_from(n).map_or_else(|_| Value::Decimal(n.to_string()), Value::Int))
        }
        DataType::Decimal { precision, scale } => check_decimal(t, *precision, *scale)
            .map(Value::Decimal)
            .map_err(fail),
        DataType::Float => match t.parse::<f64>() {
            Ok(x) if x.is_finite() => Ok(Value::Float(x)),
            _ => Err(fail(format!("`{t}` is not a number"))),
        },
        DataType::Text { max_len } | DataType::Char { len: max_len } => {
            let n = t.chars().count();
            match max_len {
                Some(max) if n > *max as usize => {
                    Err(fail(format!("at most {max} characters (got {n})")))
                }
                _ => Ok(Value::Text(t.to_string())),
            }
        }
        DataType::Uuid => {
            let inner = t
                .strip_prefix('{')
                .and_then(|s| s.strip_suffix('}'))
                .unwrap_or(t);
            let hex: String = inner.chars().filter(|c| *c != '-').collect();
            let dashes_ok =
                !inner.contains('-') || inner.split('-').map(str::len).eq([8usize, 4, 4, 4, 12]);
            if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) || !dashes_ok {
                return Err(fail(format!("`{t}` is not a UUID")));
            }
            let h = hex.to_ascii_lowercase();
            Ok(Value::Uuid(format!(
                "{}-{}-{}-{}-{}",
                &h[..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..]
            )))
        }
        DataType::Date => check_date(t)
            .then(|| Value::Date(t.to_string()))
            .ok_or_else(|| fail(format!("`{t}` is not a date (YYYY-MM-DD)"))),
        DataType::Time => check_time(t)
            .then(|| Value::Time(t.to_string()))
            .ok_or_else(|| fail(format!("`{t}` is not a time (HH:MM[:SS])"))),
        DataType::DateTime => parse_datetime(t, false)
            .map(Value::DateTime)
            .ok_or_else(|| fail(format!("`{t}` is not a date-time (YYYY-MM-DD HH:MM[:SS])"))),
        DataType::DateTimeTz => parse_datetime(t, true)
            .map(Value::DateTimeTz)
            .ok_or_else(|| {
                fail(format!(
                    "`{t}` is not a date-time (YYYY-MM-DD HH:MM[:SS][±HH:MM])"
                ))
            }),
        DataType::Json => serde_json::from_str::<serde::de::IgnoredAny>(t)
            .map(|_| Value::Json(t.to_string()))
            .map_err(|e| fail(format!("invalid JSON: {e}"))),
        DataType::Bytes => {
            let hex = ["0x", "0X", "\\x"].iter().find_map(|p| t.strip_prefix(p));
            hex.and_then(decode_hex)
                .map(Value::Bytes)
                .ok_or_else(|| fail("binary must be hex, like 0x0a1b".into()))
        }
        DataType::Enum(labels) => labels
            .iter()
            .find(|l| l.as_str() == t)
            .or_else(|| labels.iter().find(|l| l.eq_ignore_ascii_case(t)))
            .map(|l| Value::Text(l.clone()))
            .ok_or_else(|| fail(format!("must be one of: {}", labels.join(", ")))),
        DataType::Other(_) => Ok(Value::Text(t.to_string())),
    }
}

fn check_decimal(t: &str, precision: Option<u16>, scale: Option<u16>) -> Result<String, String> {
    let bad = || format!("`{t}` is not a decimal number");
    let (neg, body) = match t.as_bytes()[0] {
        b'-' => (true, &t[1..]),
        b'+' => (false, &t[1..]),
        _ => (false, t),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if (int.is_empty() && frac.is_empty()) || !digits(int) || !digits(frac) {
        return Err(bad());
    }
    let int = int.trim_start_matches('0');
    let frac_sig = frac.trim_end_matches('0').len();
    if let Some(s) = scale.filter(|s| frac_sig > *s as usize) {
        return Err(format!("at most {s} decimal places"));
    }
    if let Some(p) = precision {
        let room = p.saturating_sub(scale.unwrap_or(0)) as usize;
        if int.len() > room {
            return Err(format!("at most {room} digits before the decimal point"));
        }
    }
    let mut out = String::new();
    if neg && (!int.is_empty() || frac_sig > 0) {
        out.push('-');
    }
    out.push_str(if int.is_empty() { "0" } else { int });
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    Ok(out)
}

fn num(s: &str, len: usize) -> Option<u32> {
    (s.len() == len && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok())?
}

fn check_date(t: &str) -> bool {
    let mut it = t.split('-');
    let (Some(y), Some(m), Some(d), None) = (it.next(), it.next(), it.next(), it.next()) else {
        return false;
    };
    let (Some(y), Some(m), Some(d)) = (num(y, 4), num(m, 2), num(d, 2)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    y > 0 && (1..=days).contains(&d)
}

fn check_time(t: &str) -> bool {
    let (hms, frac) = t.split_once('.').unwrap_or((t, ""));
    if t.contains('.')
        && (frac.is_empty() || frac.len() > 9 || !frac.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    let parts: Vec<&str> = hms.split(':').collect();
    if !(2..=3).contains(&parts.len()) || (parts.len() == 2 && !frac.is_empty()) {
        return false;
    }
    let limits = [23, 59, 59];
    parts
        .iter()
        .zip(limits)
        .all(|(p, max)| num(p, 2).is_some_and(|n| n <= max))
}

/// Date `T`/space time, with an optional offset when `tz`. Returns the canonical text.
fn parse_datetime(t: &str, tz: bool) -> Option<String> {
    if check_date(t) {
        return Some(format!("{t} 00:00:00"));
    }
    let (date, rest) = (t.get(..10)?, t.get(10..)?);
    let rest = rest.strip_prefix(['T', 't', ' '])?;
    if !check_date(date) {
        return None;
    }
    let (time, offset) = if tz { split_offset(rest)? } else { (rest, "") };
    check_time(time).then(|| format!("{date} {time}{offset}"))
}

/// `HH:MM:SS[.f][ ](Z|±HH[:MM]|±HHMM)` → (time, offset as typed, without the space).
fn split_offset(rest: &str) -> Option<(&str, &str)> {
    if let Some(time) = rest.strip_suffix(['Z', 'z']) {
        return Some((time.trim_end(), &rest[rest.len() - 1..]));
    }
    let Some(i) = rest.find(['+', '-']) else {
        return Some((rest, ""));
    };
    let (time, off) = (rest[..i].trim_end(), &rest[i..]);
    let digits = off[1..].replace(':', "");
    let ok = matches!(digits.len(), 2 | 4)
        && digits.bytes().all(|b| b.is_ascii_digit())
        && digits[..2].parse::<u32>().is_ok_and(|h| h <= 15)
        && digits
            .get(2..)
            .is_none_or(|m| m.is_empty() || m.parse::<u32>().is_ok_and(|m| m < 60));
    ok.then_some((time, off))
}

fn decode_hex(h: &str) -> Option<Vec<u8>> {
    if !h.len().is_multiple_of(2) || !h.is_ascii() {
        return None;
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(kind: DbKind, native: &str, nullable: bool) -> ColumnMeta {
        let mut c = ColumnMeta::new(kind, "c", native);
        c.nullable = nullable;
        c
    }

    fn ok(kind: DbKind, native: &str, text: &str) -> Value {
        parse_cell(text, &col(kind, native, true))
            .unwrap_or_else(|e| panic!("{native} {text}: {e}"))
    }

    fn err(kind: DbKind, native: &str, text: &str) -> String {
        match parse_cell(text, &col(kind, native, false)) {
            Ok(v) => panic!("{native} `{text}` should fail, got {v:?}"),
            Err(e) => e.message,
        }
    }

    use DbKind::*;

    #[test]
    fn map_native_per_engine() {
        let i = |bits, unsigned| DataType::Int { bits, unsigned };
        assert_eq!(
            map_native(Postgres, "character varying(20)"),
            DataType::Text { max_len: Some(20) }
        );
        assert_eq!(
            map_native(Postgres, "NUMERIC(10, 2)"),
            DataType::Decimal {
                precision: Some(10),
                scale: Some(2)
            }
        );
        assert_eq!(
            map_native(Postgres, "numeric"),
            DataType::Decimal {
                precision: None,
                scale: None
            }
        );
        assert_eq!(
            map_native(Postgres, "timestamp(3) with time zone"),
            DataType::DateTimeTz
        );
        assert_eq!(map_native(Postgres, "int4"), i(32, false));
        assert_eq!(
            map_native(Postgres, "integer[]"),
            DataType::Other("integer[]".into())
        );
        assert_eq!(map_native(Postgres, "jsonb"), DataType::Json);
        assert_eq!(map_native(MySql, "tinyint(1)"), DataType::Bool);
        assert_eq!(map_native(MySql, "int(11) unsigned"), i(32, true));
        assert_eq!(
            map_native(MySql, "enum('a','it''s')"),
            DataType::Enum(vec!["a".into(), "it's".into()])
        );
        assert_eq!(
            map_native(MySql, "decimal"),
            DataType::Decimal {
                precision: Some(10),
                scale: Some(0)
            }
        );
        assert_eq!(map_native(MySql, "longblob"), DataType::Bytes);
        assert_eq!(
            map_native(MsSql, "nvarchar(max)"),
            DataType::Text { max_len: None }
        );
        assert_eq!(map_native(MsSql, "tinyint"), i(8, true));
        assert_eq!(map_native(MsSql, "timestamp"), DataType::Bytes);
        assert_eq!(map_native(MsSql, "uniqueidentifier"), DataType::Uuid);
        assert_eq!(map_native(MsSql, "datetimeoffset(7)"), DataType::DateTimeTz);
        assert_eq!(map_native(Sqlite, "INTEGER"), i(64, false));
        assert_eq!(
            map_native(Sqlite, "VARCHAR(30)"),
            DataType::Text { max_len: Some(30) }
        );
        assert_eq!(map_native(Sqlite, "DOUBLE"), DataType::Float);
        assert_eq!(map_native(Sqlite, ""), DataType::Other(String::new()));
        assert_eq!(map_native(Sqlite, "BOOLEAN"), DataType::Bool);
    }

    #[test]
    fn empty_and_null() {
        // The legacy string entry point: empty on a nullable column is NULL.
        assert_eq!(ok(Postgres, "integer", "  "), Value::Null);
        assert_eq!(ok(Postgres, "text", ""), Value::Null);
        assert!(err(Postgres, "integer", "").contains("NOT NULL"));
        assert_eq!(
            parse_cell("", &col(Postgres, "text", false)),
            Ok(Value::Text(String::new()))
        );
        assert_eq!(
            ok(Postgres, "text", "  padded "),
            Value::Text("  padded ".into())
        );
    }

    #[test]
    fn input_tells_empty_string_from_null() {
        let inp = |native, nullable, input: Option<&str>| {
            parse_input(input, &col(Postgres, native, nullable))
        };
        // Text-like: empty is '' (nullable or not); NULL only when asked for, and only if allowed.
        for native in ["text", "varchar(5)", "char(3)", "inet"] {
            assert_eq!(inp(native, true, Some("")), Ok(Value::Text(String::new())));
            assert_eq!(inp(native, false, Some("")), Ok(Value::Text(String::new())));
            assert_eq!(inp(native, true, None), Ok(Value::Null));
            assert!(
                inp(native, false, None)
                    .unwrap_err()
                    .message
                    .contains("NOT NULL")
            );
        }
        // Whitespace on a text column is data.
        assert_eq!(inp("text", true, Some(" ")), Ok(Value::Text(" ".into())));
        // Everything else: empty / blank is NULL if nullable, else required.
        for native in [
            "integer",
            "date",
            "boolean",
            "uuid",
            "jsonb",
            "numeric(5,2)",
        ] {
            assert_eq!(inp(native, true, Some("")), Ok(Value::Null), "{native}");
            assert_eq!(inp(native, true, Some("  ")), Ok(Value::Null), "{native}");
            assert_eq!(inp(native, true, None), Ok(Value::Null), "{native}");
            assert!(inp(native, false, Some("")).is_err(), "{native}");
            assert!(inp(native, false, None).is_err(), "{native}");
        }
        assert_eq!(inp("integer", true, Some(" 7 ")), Ok(Value::Int(7)));
    }

    #[test]
    fn bools() {
        for t in ["true", "T", "1", "Yes", "on", "y"] {
            assert_eq!(ok(Postgres, "boolean", t), Value::Bool(true), "{t}");
        }
        for t in ["false", "F", "0", "NO", "off", "n"] {
            assert_eq!(ok(MsSql, "bit", t), Value::Bool(false), "{t}");
        }
        err(Postgres, "boolean", "maybe");
    }

    #[test]
    fn ints() {
        assert_eq!(ok(Postgres, "smallint", "-32768"), Value::Int(-32768));
        assert!(err(Postgres, "smallint", "32768").contains("out of range"));
        assert_eq!(ok(MsSql, "tinyint", "255"), Value::Int(255));
        err(MsSql, "tinyint", "-1");
        err(MySql, "int unsigned", "-5");
        assert_eq!(
            ok(MySql, "bigint unsigned", "18446744073709551615"),
            Value::Decimal("18446744073709551615".into())
        );
        err(MySql, "bigint unsigned", "18446744073709551616");
        assert_eq!(ok(Postgres, "bigint", " +42 "), Value::Int(42));
        err(Postgres, "integer", "4.2");
        err(Postgres, "integer", "abc");
    }

    #[test]
    fn decimals() {
        let d = |s: &str| Value::Decimal(s.into());
        assert_eq!(ok(Postgres, "numeric(5,2)", "123.45"), d("123.45"));
        assert_eq!(ok(Postgres, "numeric(5,2)", "-007.5"), d("-7.5"));
        assert_eq!(ok(Postgres, "numeric(5,2)", ".5"), d("0.5"));
        assert_eq!(ok(Postgres, "numeric(5,2)", "1.500"), d("1.500"));
        assert!(err(Postgres, "numeric(5,2)", "1234.5").contains("3 digits"));
        assert!(err(Postgres, "numeric(5,2)", "1.234").contains("2 decimal"));
        assert_eq!(
            ok(Postgres, "numeric", "123456789012345678901234567890.123"),
            d("123456789012345678901234567890.123")
        );
        err(Postgres, "numeric", "1e5");
        err(Postgres, "numeric", "-");
        err(MsSql, "money", "1.23456");
    }

    #[test]
    fn floats() {
        assert_eq!(
            ok(Postgres, "double precision", "1e-3"),
            Value::Float(0.001)
        );
        err(Postgres, "real", "inf");
        err(Postgres, "real", "x");
    }

    #[test]
    fn texts() {
        assert_eq!(ok(Postgres, "varchar(3)", "àbc"), Value::Text("àbc".into()));
        assert!(err(Postgres, "varchar(3)", "abcd").contains("at most 3"));
        err(MsSql, "nchar(2)", "abc");
        assert_eq!(
            ok(MsSql, "nvarchar(max)", &"x".repeat(10_000)),
            Value::Text("x".repeat(10_000))
        );
    }

    #[test]
    fn uuids() {
        let u = Value::Uuid("123e4567-e89b-12d3-a456-426614174000".into());
        assert_eq!(
            ok(Postgres, "uuid", "123E4567-E89B-12D3-A456-426614174000"),
            u
        );
        assert_eq!(
            ok(
                MsSql,
                "uniqueidentifier",
                "{123e4567-e89b-12d3-a456-426614174000}"
            ),
            u
        );
        assert_eq!(ok(Postgres, "uuid", "123e4567e89b12d3a456426614174000"), u);
        err(Postgres, "uuid", "123e4567-e89b-12d3-a456-42661417400");
        err(Postgres, "uuid", "123e4567e-89b-12d3-a456-426614174000");
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(
            ok(Postgres, "date", "2024-02-29"),
            Value::Date("2024-02-29".into())
        );
        err(Postgres, "date", "2023-02-29");
        err(Postgres, "date", "2024-13-01");
        err(Postgres, "date", "24-01-01");
        assert_eq!(
            ok(Postgres, "time", "23:59:59.123456"),
            Value::Time("23:59:59.123456".into())
        );
        assert_eq!(ok(Postgres, "time", "08:30"), Value::Time("08:30".into()));
        err(Postgres, "time", "24:00:00");
        err(Postgres, "time", "12:60");
        err(Postgres, "time", "12:00:00.");
        let dt = |s: &str| Value::DateTime(s.into());
        assert_eq!(
            ok(MySql, "datetime", "2024-01-02T03:04:05"),
            dt("2024-01-02 03:04:05")
        );
        assert_eq!(
            ok(MySql, "datetime", "2024-01-02"),
            dt("2024-01-02 00:00:00")
        );
        err(MySql, "datetime", "2024-01-02 03:04:05+02:00");
        let tz = |s: &str| Value::DateTimeTz(s.into());
        assert_eq!(
            ok(Postgres, "timestamptz", "2024-01-02T03:04:05Z"),
            tz("2024-01-02 03:04:05Z")
        );
        assert_eq!(
            ok(
                MsSql,
                "datetimeoffset",
                "2024-01-02 03:04:05.1234567 -05:30"
            ),
            tz("2024-01-02 03:04:05.1234567-05:30")
        );
        assert_eq!(
            ok(Postgres, "timestamptz", "2024-01-02 03:04:05+0200"),
            tz("2024-01-02 03:04:05+0200")
        );
        assert_eq!(
            ok(Postgres, "timestamptz", "2024-01-02 03:04"),
            tz("2024-01-02 03:04")
        );
        err(Postgres, "timestamptz", "2024-01-02 03:04:05+25:00");
    }

    #[test]
    fn json_bytes_enum_other() {
        assert_eq!(
            ok(Postgres, "jsonb", r#" {"a": [1, 2]} "#),
            Value::Json(r#"{"a": [1, 2]}"#.into())
        );
        assert!(err(Postgres, "json", "{a:1}").starts_with("invalid JSON"));
        err(Postgres, "json", "[1] x");
        assert_eq!(
            ok(Postgres, "bytea", "\\xDEADbeef"),
            Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef])
        );
        assert_eq!(
            ok(MsSql, "varbinary(10)", "0x00ff"),
            Value::Bytes(vec![0, 255])
        );
        err(MsSql, "varbinary(10)", "0x0");
        err(MsSql, "varbinary(10)", "hello");
        assert_eq!(
            ok(MySql, "enum('Small','Large')", "large"),
            Value::Text("Large".into())
        );
        assert!(err(MySql, "enum('Small','Large')", "medium").contains("Small, Large"));
        assert_eq!(
            ok(Postgres, "interval", " 1 day "),
            Value::Text(" 1 day ".into())
        );
    }

    #[test]
    fn edit_text_round_trips() {
        let cases = [
            (Postgres, "integer", Value::Int(-3)),
            (Postgres, "boolean", Value::Bool(true)),
            (Postgres, "bytea", Value::Bytes(vec![1, 171])),
            (Postgres, "numeric(6,2)", Value::Decimal("12.30".into())),
            (Postgres, "date", Value::Date("2020-01-01".into())),
            (Postgres, "integer", Value::Null),
            (Postgres, "text", Value::Text(String::new())),
            (Postgres, "text", Value::Null),
        ];
        for (k, native, v) in cases {
            assert_eq!(
                parse_input(v.input_text().as_deref(), &col(k, native, true)),
                Ok(v.clone()),
                "{native}"
            );
        }
        assert_eq!(Value::Text(String::new()).input_text(), Some(String::new()));
        assert_eq!(Value::Null.input_text(), None);
        assert_eq!(Value::Null.to_string(), "NULL");
    }
}
