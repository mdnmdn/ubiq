//! The usage meter: what the agents spent, bucketed, kept across runs.
//!
//! Unlike everything else in this module it is not a file store behind a trait. There is one
//! implementation and nothing substitutes it, so it is a concrete type — the same shape
//! [`super::harness::FileHarnessCache`] takes, for the same reason.
//!
//! Two things make it a database rather than another TOML file. It accumulates: a bucket is a
//! running total that many small deltas add into, which is an upsert, not a rewrite of the whole
//! file. And it is unbounded: hours pile up forever, and the screen asks for a window of them.
//!
//! It lives at `<config root>/usage.db`, beside `projects.toml` and **not** under `cache/`.
//! Everything in `cache/` is defined as re-derivable by asking again; nobody can ask a harness
//! what it spent last Tuesday, so this is a record, not a cache.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use ubiq_proto::stats::UsageRow;

/// How wide a durable bucket is. An hour is what a history chart is drawn from, and it is coarse
/// enough that a year of continuous use is thousands of rows rather than millions.
const HOUR: i64 = 3600;
/// How wide a this-run bucket is. A minute so a session that has been going for ten of them has a
/// shape, not one bar.
const MINUTE: i64 = 60;

/// Every schema step, in order, forever. Append only — a step's index is its version.
const MIGRATIONS: &[&str] = &[SCHEMA_001, SCHEMA_002];

/// The two tables, which are the same table twice at two granularities.
///
/// Three things here are load-bearing rather than stylistic:
///
/// 1. Every dimension column is `TEXT NOT NULL` with `''` as the "not said" sentinel, never
///    `NULL`. SQLite treats two NULLs as distinct in a unique index, so a row with an unknown
///    model would insert afresh on every report instead of summing into the one already there —
///    the accumulating upsert below would silently stop accumulating.
/// 2. `bucket` is a floored unix epoch INTEGER (`now - now % 3600`), not a formatted string. It
///    range-scans as the leading column of the key, and it carries no timezone to be wrong about.
/// 3. The composite PRIMARY KEY *is* the upsert's conflict target, and `WITHOUT ROWID` means the
///    key is the storage rather than a second index over it.
const SCHEMA_001: &str = r#"
CREATE TABLE usage_hour (
  bucket       INTEGER NOT NULL,  -- unix seconds, floored to the hour, UTC
  project      TEXT    NOT NULL,  -- project ULID, '' when the work had no project
  harness      TEXT    NOT NULL,
  account      TEXT    NOT NULL,  -- '' when the harness default was used
  model        TEXT    NOT NULL,  -- '' when the harness did not say
  tokens_in    INTEGER NOT NULL DEFAULT 0,
  tokens_out   INTEGER NOT NULL DEFAULT 0,
  tokens_think INTEGER NOT NULL DEFAULT 0,
  tokens_other INTEGER NOT NULL DEFAULT 0,
  msgs_in      INTEGER NOT NULL DEFAULT 0,
  msgs_out     INTEGER NOT NULL DEFAULT 0,
  tool_calls   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (bucket, project, harness, account, model)
) WITHOUT ROWID;

CREATE TABLE usage_minute (
  bucket       INTEGER NOT NULL,  -- unix seconds, floored to the minute, UTC
  project      TEXT    NOT NULL,
  harness      TEXT    NOT NULL,
  account      TEXT    NOT NULL,
  model        TEXT    NOT NULL,
  tokens_in    INTEGER NOT NULL DEFAULT 0,
  tokens_out   INTEGER NOT NULL DEFAULT 0,
  tokens_think INTEGER NOT NULL DEFAULT 0,
  tokens_other INTEGER NOT NULL DEFAULT 0,
  msgs_in      INTEGER NOT NULL DEFAULT 0,
  msgs_out     INTEGER NOT NULL DEFAULT 0,
  tool_calls   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (bucket, project, harness, account, model)
) WITHOUT ROWID;
"#;

/// The sixth dimension: which subagent spent it, `''` for the conversation's own spend.
///
/// A turn's spend splits between the conversation and the agents it spawned, and a subagent's
/// report repeats the parent's occupancy unchanged — so the two must land in *different* rows or
/// a subagent's tokens would be summed into the parent's bucket and lost as a breakdown.
///
/// SQLite cannot widen a `WITHOUT ROWID` table's primary key in place, so this is the standard
/// create-new / copy / drop / rename. Rows already in the field carry over with `subagent = ''`,
/// which is exactly what they are: spend nobody attributed to a subagent.
const SCHEMA_002: &str = r#"
CREATE TABLE usage_hour_next (
  bucket       INTEGER NOT NULL,
  project      TEXT    NOT NULL,
  harness      TEXT    NOT NULL,
  account      TEXT    NOT NULL,
  model        TEXT    NOT NULL,
  subagent     TEXT    NOT NULL DEFAULT '',
  tokens_in    INTEGER NOT NULL DEFAULT 0,
  tokens_out   INTEGER NOT NULL DEFAULT 0,
  tokens_think INTEGER NOT NULL DEFAULT 0,
  tokens_other INTEGER NOT NULL DEFAULT 0,
  msgs_in      INTEGER NOT NULL DEFAULT 0,
  msgs_out     INTEGER NOT NULL DEFAULT 0,
  tool_calls   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (bucket, project, harness, account, model, subagent)
) WITHOUT ROWID;

INSERT INTO usage_hour_next
SELECT bucket, project, harness, account, model, '', tokens_in, tokens_out, tokens_think,
       tokens_other, msgs_in, msgs_out, tool_calls
FROM usage_hour;

DROP TABLE usage_hour;
ALTER TABLE usage_hour_next RENAME TO usage_hour;

CREATE TABLE usage_minute_next (
  bucket       INTEGER NOT NULL,
  project      TEXT    NOT NULL,
  harness      TEXT    NOT NULL,
  account      TEXT    NOT NULL,
  model        TEXT    NOT NULL,
  subagent     TEXT    NOT NULL DEFAULT '',
  tokens_in    INTEGER NOT NULL DEFAULT 0,
  tokens_out   INTEGER NOT NULL DEFAULT 0,
  tokens_think INTEGER NOT NULL DEFAULT 0,
  tokens_other INTEGER NOT NULL DEFAULT 0,
  msgs_in      INTEGER NOT NULL DEFAULT 0,
  msgs_out     INTEGER NOT NULL DEFAULT 0,
  tool_calls   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (bucket, project, harness, account, model, subagent)
) WITHOUT ROWID;

INSERT INTO usage_minute_next
SELECT bucket, project, harness, account, model, '', tokens_in, tokens_out, tokens_think,
       tokens_other, msgs_in, msgs_out, tool_calls
FROM usage_minute;

DROP TABLE usage_minute;
ALTER TABLE usage_minute_next RENAME TO usage_minute;
"#;

/// The columns, in the one order every statement here uses.
const COLUMNS: &str = "bucket, project, harness, account, model, subagent, tokens_in, tokens_out, \
                       tokens_think, tokens_other, msgs_in, msgs_out, tool_calls";

/// What can go wrong reaching the meter. One variant: every failure here is the database saying
/// no, and there is nothing a caller does differently for one kind over another — it logs and
/// carries on without a meter.
#[derive(Debug, thiserror::Error)]
#[error("{path}: {source}")]
pub struct UsageError {
    pub path: PathBuf,
    #[source]
    pub source: rusqlite::Error,
}

/// The usage meter, as one SQLite file.
pub struct Usage {
    // ponytail: one lock over one connection; a pool if the meter ever writes from more than the
    // coordinator thread.
    db: Mutex<Connection>,
    path: PathBuf,
}

impl Usage {
    /// Open `<root>/usage.db`, migrate it, and drop the previous run's minutes.
    ///
    /// An `Err` here is survivable and is meant to be: a read-only config root costs the user
    /// their token history, which is not worth costing them their session. The coordinator logs
    /// it and carries an absent meter.
    pub fn open(root: &Path) -> Result<Self, UsageError> {
        let path = root.join("usage.db");
        let wrap = |source| UsageError {
            path: path.clone(),
            source,
        };

        // A missing config root is the ordinary first run, not a failure. The `open` below reports
        // whatever is genuinely wrong with the path.
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let db = Connection::open(&path).map_err(wrap)?;
        // WAL so a reader (the stats sampler) and the writer (the meter) never wait on each other.
        db.pragma_update(None, "journal_mode", "WAL")
            .map_err(wrap)?;
        migrate(&db).map_err(wrap)?;
        // "The current run" is what `usage_minute` means, so a previous run's minutes are not it —
        // and they are not lost either, having been summed into `usage_hour` as they were written.
        // This is why the table needs no run-id column.
        db.execute_batch("DELETE FROM usage_minute").map_err(wrap)?;

        Ok(Self {
            db: Mutex::new(db),
            path,
        })
    }

    /// Record one delta, into both tables, in one transaction.
    ///
    /// The caller reports what was just spent once; the hour row and the minute row are this
    /// method's business, not theirs. `row.bucket` is ignored — `at` is the truth, floored twice.
    ///
    /// Called from the conversation pump, not from the coordinator — see
    /// [`crate::conversation::UsageMeter`] for why the meter has to travel to the thread that
    /// reads the harness.
    ///
    /// Only a report carrying spend reaches here: occupancy is a level, and a level is never
    /// accumulated.
    pub fn record(&self, at: SystemTime, row: &UsageRow) -> Result<(), UsageError> {
        let now = at
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let wrap = |source| UsageError {
            path: self.path.clone(),
            source,
        };

        let tx = db.unchecked_transaction().map_err(wrap)?;
        for (table, width) in [("usage_hour", HOUR), ("usage_minute", MINUTE)] {
            // The counters cross as `i64`: SQLite has one integer type, and no plausible token
            // count comes near its top.
            tx.execute(
                &upsert(table),
                rusqlite::params![
                    now - now % width,
                    row.project,
                    row.harness,
                    row.account,
                    row.model,
                    row.subagent,
                    row.tokens_in as i64,
                    row.tokens_out as i64,
                    row.tokens_think as i64,
                    row.tokens_other as i64,
                    row.msgs_in as i64,
                    row.msgs_out as i64,
                    row.tool_calls as i64,
                ],
            )
            .map_err(wrap)?;
        }
        tx.commit().map_err(wrap)
    }

    /// Every hour bucket at or after `since`, oldest first.
    pub fn history(&self, since: i64) -> Result<Vec<UsageRow>, UsageError> {
        self.rows("usage_hour", since)
    }

    /// Every minute bucket of this run — the table holds nothing else, so there is nothing to
    /// filter by.
    pub fn this_run(&self) -> Result<Vec<UsageRow>, UsageError> {
        self.rows("usage_minute", i64::MIN)
    }

    fn rows(&self, table: &str, since: i64) -> Result<Vec<UsageRow>, UsageError> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let wrap = |source| UsageError {
            path: self.path.clone(),
            source,
        };

        // `table` is one of two literals from this module, never anything a caller said.
        let sql = format!("SELECT {COLUMNS} FROM {table} WHERE bucket >= ?1 ORDER BY bucket");
        let mut statement = db.prepare(&sql).map_err(wrap)?;
        let rows = statement
            .query_map([since], |r| {
                Ok(UsageRow {
                    bucket: r.get(0)?,
                    project: r.get(1)?,
                    harness: r.get(2)?,
                    account: r.get(3)?,
                    model: r.get(4)?,
                    subagent: r.get(5)?,
                    tokens_in: r.get(6)?,
                    tokens_out: r.get(7)?,
                    tokens_think: r.get(8)?,
                    tokens_other: r.get(9)?,
                    msgs_in: r.get(10)?,
                    msgs_out: r.get(11)?,
                    tool_calls: r.get(12)?,
                })
            })
            .map_err(wrap)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(wrap)?;
        Ok(rows)
    }
}

/// The one write, for either table: insert the bucket, or add into the one already there. The
/// conflict target is the whole primary key, which is what makes a second report for the same
/// six dimensions a sum rather than a duplicate.
fn upsert(table: &str) -> String {
    format!(
        "INSERT INTO {table} ({COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) \
         ON CONFLICT(bucket,project,harness,account,model,subagent) DO UPDATE SET \
         tokens_in = tokens_in + excluded.tokens_in, \
         tokens_out = tokens_out + excluded.tokens_out, \
         tokens_think = tokens_think + excluded.tokens_think, \
         tokens_other = tokens_other + excluded.tokens_other, \
         msgs_in = msgs_in + excluded.msgs_in, \
         msgs_out = msgs_out + excluded.msgs_out, \
         tool_calls = tool_calls + excluded.tool_calls"
    )
}

/// Migrate to the head of [`MIGRATIONS`]. `PRAGMA user_version` is the whole engine: an integer
/// SQLite already keeps in the file header, so a schema table is not needed to hold one number.
fn migrate(db: &Connection) -> rusqlite::Result<()> {
    let at: usize = db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    if at >= MIGRATIONS.len() {
        return Ok(());
    }
    let tx = db.unchecked_transaction()?;
    for step in &MIGRATIONS[at..] {
        tx.execute_batch(step)?;
    }
    // Not a bind parameter: PRAGMA does not take one.
    tx.execute_batch(&format!("PRAGMA user_version = {}", MIGRATIONS.len()))?;
    tx.commit()
}
