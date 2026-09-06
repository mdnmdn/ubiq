//! The usage meter: the two rules that make a bucket a running total, and the one that makes
//! "this run" mean this run.
//!
//! There is very little logic here to test — the schema does the work. What is worth asserting is
//! exactly the part the schema could get silently wrong: an upsert whose conflict target does not
//! match the key stops accumulating without ever failing, and a `NULL` dimension would do the same
//! thing while looking correct in every other respect.

use std::time::{Duration, SystemTime};

use rusqlite::Connection;
use tempfile::TempDir;
use ubiq_host::store::usage::Usage;
use ubiq_proto::stats::UsageRow;

/// A row for the one dimension tuple these tests use, spending `tokens_in` and nothing else.
fn spent(tokens_in: u64, model: &str) -> UsageRow {
    UsageRow {
        harness: "claude-code".to_string(),
        project: "01J0PROJECT".to_string(),
        model: model.to_string(),
        tokens_in,
        ..UsageRow::default()
    }
}

/// A fixed instant, so two records land in the same hour and the same minute however slow the
/// test machine is.
fn at() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_770_000_000)
}

fn user_version(dir: &TempDir) -> i64 {
    let db = Connection::open(dir.path().join("usage.db")).unwrap();
    db.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn count(dir: &TempDir, table: &str) -> i64 {
    let db = Connection::open(dir.path().join("usage.db")).unwrap();
    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// Opening a database that is already at the head of the migration list must do nothing at all —
/// a second `CREATE TABLE` would error, so a passing reopen is the assertion.
#[test]
fn migrating_twice_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let first = {
        let usage = Usage::open(dir.path()).unwrap();
        drop(usage);
        user_version(&dir)
    };

    Usage::open(dir.path()).unwrap();

    assert_eq!(user_version(&dir), first);
}

/// The one piece of real logic: a second report for the same five dimensions adds into the row
/// already there. If the primary key or the `ON CONFLICT` target ever disagree, this is where it
/// shows — as two rows of 100 and 50 rather than one of 150.
#[test]
fn the_same_bucket_sums_rather_than_duplicating() {
    let dir = TempDir::new().unwrap();
    let usage = Usage::open(dir.path()).unwrap();

    usage.record(at(), &spent(100, "opus")).unwrap();
    usage.record(at(), &spent(50, "opus")).unwrap();

    let history = usage.history(0).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].tokens_in, 150);
    // The minute table is the same write, so it holds the same total.
    let this_run = usage.this_run().unwrap();
    assert_eq!(this_run.len(), 1);
    assert_eq!(this_run[0].tokens_in, 150);
}

/// The `''`-not-`NULL` rule. SQLite treats two NULLs as distinct in a unique index, so a harness
/// that never says which model answered would insert a fresh row on every report — the meter
/// would keep working and keep being wrong.
#[test]
fn an_unsaid_dimension_still_sums() {
    let dir = TempDir::new().unwrap();
    let usage = Usage::open(dir.path()).unwrap();

    usage.record(at(), &spent(100, "")).unwrap();
    usage.record(at(), &spent(50, "")).unwrap();

    let history = usage.history(0).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].tokens_in, 150);
}

/// "This run" is what the minute table means, and it has no run-id column to enforce that with —
/// the emptying on open is the enforcement. The hour table is untouched by it, which is the half
/// that makes losing the minutes free.
#[test]
fn a_previous_run_leaves_no_minutes_behind() {
    let dir = TempDir::new().unwrap();
    {
        let usage = Usage::open(dir.path()).unwrap();
        usage.record(at(), &spent(100, "opus")).unwrap();
    }

    let usage = Usage::open(dir.path()).unwrap();

    assert!(usage.this_run().unwrap().is_empty());
    assert_eq!(count(&dir, "usage_minute"), 0);
    assert_eq!(count(&dir, "usage_hour"), 1);
    assert_eq!(usage.history(0).unwrap()[0].tokens_in, 100);
}

/// A meter that cannot be opened is a stats screen with empty rows, not a session that ends. The
/// caller gets an `Err` to log and carries on without one.
#[test]
fn an_unwritable_database_does_not_stop_the_session() {
    let dir = TempDir::new().unwrap();
    // A file where the config root should be: the directory cannot be created, and `usage.db`
    // cannot be a child of it.
    let blocked = dir.path().join("not-a-directory");
    std::fs::write(&blocked, "").unwrap();

    assert!(Usage::open(&blocked).is_err());
}

/// The meter creates its file where the config root says, and both tables come out of the
/// migration. Not a rule so much as the end-to-end shape the screen depends on.
#[test]
fn the_database_lands_in_the_config_root_with_both_tables() {
    let dir = TempDir::new().unwrap();
    let _usage = Usage::open(dir.path()).unwrap();

    assert!(dir.path().join("usage.db").is_file(), "no usage.db");

    let db = Connection::open(dir.path().join("usage.db")).unwrap();
    let mut tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    tables.retain(|name| !name.starts_with("sqlite_"));
    assert_eq!(tables, vec!["usage_hour", "usage_minute"]);
}
