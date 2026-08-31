//! The relative times the picker prints.
//!
//! `now` is a parameter, so this is a table rather than something that has to be waited for.

use chrono::{Duration, TimeZone, Utc};
use ubiq::state::when::{NEVER, relative, relative_opt};

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, 0).unwrap()
}

#[test]
fn each_bucket_prints_the_way_the_picker_expects() {
    let now = now();
    let cases = [
        (Duration::seconds(0), "now"),
        (Duration::seconds(44), "now"),
        (Duration::seconds(45), "0m"),
        (Duration::minutes(12), "12m"),
        (Duration::minutes(59), "59m"),
        (Duration::hours(1), "1h"),
        (Duration::hours(5), "5h"),
        (Duration::days(3), "3d"),
        (Duration::days(14), "2w"),
        (Duration::days(120), "4mo"),
        (Duration::days(800), "2y"),
    ];

    for (ago, want) in cases {
        let got = relative(now - ago, now);
        assert_eq!(got, want, "{ago} ago should read {want}, got {got}");
    }
}

#[test]
fn yesterday_is_a_calendar_day_rather_than_a_duration() {
    // 13:00 today against 11:00 the previous day is 22 hours, and is still yesterday.
    let now = Utc.with_ymd_and_hms(2026, 8, 31, 13, 0, 0).unwrap();
    let then = Utc.with_ymd_and_hms(2026, 8, 30, 11, 0, 0).unwrap();

    // Whether this crosses local midnight depends on the reader's zone, which is the point: it is
    // either yesterday or a count of hours, and never something in between.
    let got = relative(then, now);
    assert!(
        got == "yst" || got.ends_with('h'),
        "22 hours back should be yesterday or an hour count, got {got}"
    );
}

#[test]
fn two_days_back_is_never_yesterday() {
    let now = Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, 0).unwrap();
    let then = Utc.with_ymd_and_hms(2026, 8, 29, 12, 0, 0).unwrap();

    assert_eq!(relative(then, now), "2d");
}

#[test]
fn a_clock_that_went_backwards_still_reads_as_the_present() {
    let now = now();
    assert_eq!(relative(now + Duration::hours(3), now), "now");
}

#[test]
fn a_project_never_opened_says_so() {
    assert_eq!(relative_opt(None, now()), NEVER);
    assert_eq!(
        relative_opt(Some(now() - Duration::minutes(5)), now()),
        "5m"
    );
}
