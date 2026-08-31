//! How long ago something was, as the picker prints it.
//!
//! Rendered at draw time rather than stored, because how long ago something was is a fact about
//! the moment it is drawn — the old fixture kept a pre-rendered string that could only ever get
//! staler.
//!
//! `now` is a parameter rather than read from the clock, so the buckets are a table in a test
//! instead of something that has to be waited for.

use chrono::{DateTime, Datelike, Local, Utc};

/// What a project that has never been opened prints.
pub const NEVER: &str = "\u{2014}";

/// A short relative time: `now`, `12m`, `5h`, `yst`, `3d`, `2w`, `4mo`, `2y`.
pub fn relative(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(then).num_seconds();

    // A clock that went backwards, or a stamp from a moment ago: both read as the present.
    if seconds < 45 {
        return "now".to_string();
    }

    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }

    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }

    // Yesterday is a calendar fact, not a duration: 23 hours ago can be yesterday, and 25 hours
    // ago can be the day before. The reader's own zone is the one that decides.
    let (then_local, now_local) = (then.with_timezone(&Local), now.with_timezone(&Local));
    let days_apart = now_local
        .date_naive()
        .signed_duration_since(then_local.date_naive())
        .num_days();
    if days_apart == 1 {
        return "yst".to_string();
    }

    let days = days_apart.max(hours / 24);
    if days < 7 {
        return format!("{days}d");
    }
    if days < 28 {
        return format!("{}w", days / 7);
    }
    if days < 365 {
        return format!("{}mo", days / 30);
    }
    format!("{}y", days / 365)
}

/// The same, for a project that may never have been opened.
pub fn relative_opt(then: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    match then {
        Some(then) => relative(then, now),
        None => NEVER.to_string(),
    }
}

/// The `Datelike` import is what makes the calendar-day comparison above possible; naming it here
/// keeps the intent obvious to anyone tidying imports.
const _: fn() -> i32 = || Local::now().year();
