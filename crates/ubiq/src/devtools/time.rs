//! Reading a moment out of text, and writing it back out in every common shape.
//!
//! The local zone is a parameter rather than a call to `chrono::Local`, so a test pins it: the
//! dialog passes `&chrono::Local`, a test a `FixedOffset`.

use chrono::{DateTime, Datelike as _, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};

/// How the input was read, said back to the user so a guess is never silent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadAs {
    UnixSeconds,
    UnixMillis,
    UnixMicros,
    UnixNanos,
    Rfc3339,
    Rfc2822,
    /// A date and time with no zone, read in the local one.
    LocalDateTime,
    /// A date alone, read as local midnight.
    LocalDate,
}

impl ReadAs {
    pub fn label(self) -> &'static str {
        match self {
            ReadAs::UnixSeconds => "Unix seconds",
            ReadAs::UnixMillis => "Unix milliseconds",
            ReadAs::UnixMicros => "Unix microseconds",
            ReadAs::UnixNanos => "Unix nanoseconds",
            ReadAs::Rfc3339 => "ISO 8601 / RFC 3339",
            ReadAs::Rfc2822 => "RFC 2822",
            ReadAs::LocalDateTime => "date and time, local zone",
            ReadAs::LocalDate => "date, local midnight",
        }
    }
}

/// Zone-less formats tried in order, after the zoned ones.
const DATETIME_FORMATS: &[&str] = &[
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M",
    "%Y-%m-%dT%H:%M",
    "%Y/%m/%d %H:%M:%S",
    "%Y/%m/%d %H:%M",
    "%d/%m/%Y %H:%M:%S",
    "%d/%m/%Y %H:%M",
    "%d.%m.%Y %H:%M:%S",
    "%d.%m.%Y %H:%M",
];
const DATE_FORMATS: &[&str] = &["%Y-%m-%d", "%Y/%m/%d", "%d/%m/%Y", "%d.%m.%Y"];

/// Read a number as an epoch, picking the unit by magnitude: anything past year ~5138 in seconds
/// is taken as milliseconds, and so on up. A fraction is only read on seconds.
pub fn parse_epoch(text: &str) -> Option<(DateTime<Utc>, ReadAs)> {
    let text = text.trim();
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() {
        return None;
    }
    if let Some((whole, frac)) = digits.split_once('.') {
        if whole.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !frac.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let secs: f64 = text.parse().ok()?;
        let nanos = (secs * 1e9).round();
        if !nanos.is_finite() || nanos.abs() > i64::MAX as f64 {
            return None;
        }
        return Some((
            DateTime::from_timestamp_nanos(nanos as i64),
            ReadAs::UnixSeconds,
        ));
    }
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = text.parse().ok()?;
    let size = n.unsigned_abs();
    let (at, read) = if size >= 100_000_000_000_000_000 {
        (Some(DateTime::from_timestamp_nanos(n)), ReadAs::UnixNanos)
    } else if size >= 100_000_000_000_000 {
        (DateTime::from_timestamp_micros(n), ReadAs::UnixMicros)
    } else if size >= 100_000_000_000 {
        (DateTime::from_timestamp_millis(n), ReadAs::UnixMillis)
    } else {
        (DateTime::from_timestamp(n, 0), ReadAs::UnixSeconds)
    };
    at.map(|at| (at, read))
}

/// Read a date written out as text: RFC 3339 (with `T` or a space), RFC 2822, then the zone-less
/// shapes in [`DATETIME_FORMATS`] and [`DATE_FORMATS`], read in `local`.
pub fn parse_text<Tz: TimeZone>(text: &str, local: &Tz) -> Option<(DateTime<Utc>, ReadAs)> {
    let text = text.trim();
    if let Ok(at) = DateTime::parse_from_rfc3339(text) {
        return Some((at.with_timezone(&Utc), ReadAs::Rfc3339));
    }
    if text.len() > 10
        && text.as_bytes().get(10) == Some(&b' ')
        && let Ok(at) = DateTime::parse_from_rfc3339(&text.replacen(' ', "T", 1))
    {
        return Some((at.with_timezone(&Utc), ReadAs::Rfc3339));
    }
    if let Ok(at) = DateTime::parse_from_rfc2822(text) {
        return Some((at.with_timezone(&Utc), ReadAs::Rfc2822));
    }
    for format in DATETIME_FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            let at = local.from_local_datetime(&naive).earliest()?;
            return Some((at.with_timezone(&Utc), ReadAs::LocalDateTime));
        }
    }
    for format in DATE_FORMATS {
        if let Ok(date) = NaiveDate::parse_from_str(text, format) {
            let at = local
                .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
                .earliest()?;
            return Some((at.with_timezone(&Utc), ReadAs::LocalDate));
        }
    }
    None
}

/// Either of the two readings above.
pub fn parse<Tz: TimeZone>(text: &str, local: &Tz) -> Result<(DateTime<Utc>, ReadAs), String> {
    if text.trim().is_empty() {
        return Err("Type or paste a timestamp or a date".to_string());
    }
    parse_epoch(text)
        .or_else(|| parse_text(text, local))
        .ok_or_else(|| "Not a timestamp or a date this reads".to_string())
}

/// Every shape the section writes, labelled, in the order it draws them.
pub fn rows<Tz: TimeZone>(
    at: DateTime<Utc>,
    local: &Tz,
    now: DateTime<Utc>,
) -> Vec<(&'static str, String)>
where
    Tz::Offset: std::fmt::Display,
{
    let here = at.with_timezone(local);
    vec![
        ("Unix seconds", at.timestamp().to_string()),
        ("Unix milliseconds", at.timestamp_millis().to_string()),
        (
            "ISO 8601 (UTC)",
            at.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        ),
        (
            "ISO 8601 (local)",
            here.to_rfc3339_opts(SecondsFormat::AutoSi, false),
        ),
        // `to_rfc2822` panics outside years 0..=9999 — RFC 2822 cannot write them. RFC 3339 and
        // `format` write any year chrono holds, with a sign, so only this one is guarded.
        (
            "RFC 2822",
            if (0..=9999).contains(&at.year()) {
                at.to_rfc2822()
            } else {
                "—".to_string()
            },
        ),
        ("UTC", at.format("%Y-%m-%d %H:%M:%S").to_string()),
        ("Local", here.format("%Y-%m-%d %H:%M:%S %:z").to_string()),
        (
            "Readable",
            here.format("%A, %-d %B %Y, %H:%M:%S").to_string(),
        ),
        (
            "Week / day",
            format!(
                "ISO week {} of {}, day {} of the year",
                at.iso_week().week(),
                at.iso_week().year(),
                at.ordinal()
            ),
        ),
        ("Relative", relative(at, now)),
    ]
}

/// "3 days ago", "in 2 hours", "now" — the largest whole unit, rounded down.
pub fn relative(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let delta = at.signed_duration_since(now).num_seconds();
    let size = delta.unsigned_abs();
    if size < 5 {
        return "now".to_string();
    }
    const UNITS: [(u64, &str); 6] = [
        (365 * 86_400, "year"),
        (30 * 86_400, "month"),
        (86_400, "day"),
        (3_600, "hour"),
        (60, "minute"),
        (1, "second"),
    ];
    let (unit, name) = UNITS
        .iter()
        .find(|(unit, _)| size >= *unit)
        .copied()
        .unwrap_or((1, "second"));
    let count = size / unit;
    let plural = if count == 1 { "" } else { "s" };
    if delta < 0 {
        format!("{count} {name}{plural} ago")
    } else {
        format!("in {count} {name}{plural}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn cet() -> FixedOffset {
        FixedOffset::east_opt(3600).unwrap()
    }

    #[test]
    fn epochs_by_magnitude() {
        let (at, read) = parse_epoch("1700000000").unwrap();
        assert_eq!(read, ReadAs::UnixSeconds);
        assert_eq!(at.to_rfc3339(), "2023-11-14T22:13:20+00:00");
        let (ms, read) = parse_epoch("1700000000123").unwrap();
        assert_eq!(read, ReadAs::UnixMillis);
        assert_eq!(ms.timestamp_millis(), 1_700_000_000_123);
        assert_eq!(
            parse_epoch("1700000000123456").unwrap().1,
            ReadAs::UnixMicros
        );
        assert_eq!(
            parse_epoch("1700000000123456789").unwrap().1,
            ReadAs::UnixNanos
        );
        let (frac, _) = parse_epoch("1700000000.5").unwrap();
        assert_eq!(frac.timestamp_millis(), 1_700_000_000_500);
        assert!(parse_epoch("12ab").is_none());
        assert!(parse_epoch("").is_none());
    }

    #[test]
    fn text_dates() {
        let (at, read) = parse_text("2023-11-14T22:13:20Z", &cet()).unwrap();
        assert_eq!(read, ReadAs::Rfc3339);
        assert_eq!(at.timestamp(), 1_700_000_000);
        let (at, _) = parse_text("2023-11-14 23:13:20+01:00", &cet()).unwrap();
        assert_eq!(at.timestamp(), 1_700_000_000);
        let (at, read) = parse_text("Tue, 14 Nov 2023 22:13:20 +0000", &cet()).unwrap();
        assert_eq!(read, ReadAs::Rfc2822);
        assert_eq!(at.timestamp(), 1_700_000_000);
        // Zone-less is read in the local zone given.
        let (at, read) = parse_text("2023-11-14 23:13:20", &cet()).unwrap();
        assert_eq!(read, ReadAs::LocalDateTime);
        assert_eq!(at.timestamp(), 1_700_000_000);
        let (at, read) = parse_text("2023-11-15", &cet()).unwrap();
        assert_eq!(read, ReadAs::LocalDate);
        assert_eq!(at.to_rfc3339(), "2023-11-14T23:00:00+00:00");
        assert!(parse_text("tomorrow", &cet()).is_none());
    }

    #[test]
    fn rows_cover_every_shape() {
        let at = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let rows = rows(at, &cet(), at);
        let get = |name: &str| rows.iter().find(|(n, _)| *n == name).unwrap().1.clone();
        assert_eq!(get("Unix seconds"), "1700000000");
        assert_eq!(get("Unix milliseconds"), "1700000000000");
        assert_eq!(get("ISO 8601 (UTC)"), "2023-11-14T22:13:20Z");
        assert_eq!(get("ISO 8601 (local)"), "2023-11-14T23:13:20+01:00");
        assert_eq!(get("RFC 2822"), "Tue, 14 Nov 2023 22:13:20 +0000");
        assert_eq!(get("Relative"), "now");
    }

    /// Years RFC 2822 cannot write must not panic (chrono's `to_rfc2822` does).
    #[test]
    fn out_of_range_years_do_not_panic() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        for input in ["-70000000000", "-0001-01-01"] {
            let (at, _) = parse(input, &cet()).unwrap_or_else(|e| panic!("{input}: {e}"));
            let rows = rows(at, &cet(), now);
            let rfc2822 = &rows.iter().find(|(n, _)| *n == "RFC 2822").unwrap().1;
            assert_eq!(rfc2822, "—", "{input}");
        }
    }

    #[test]
    fn relative_words() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let ago = DateTime::from_timestamp(1_700_000_000 - 3 * 86_400, 0).unwrap();
        let ahead = DateTime::from_timestamp(1_700_000_000 + 3_600, 0).unwrap();
        assert_eq!(relative(ago, now), "3 days ago");
        assert_eq!(relative(ahead, now), "in 1 hour");
    }
}
