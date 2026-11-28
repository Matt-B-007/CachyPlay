// SPDX-License-Identifier: GPL-3.0-or-later
//! Every date and time conversion in CachyPlay goes through here.
//!
//! Times are kept as the PC's wall-clock time (`DateTime`, no time zone),
//! the same way they were before the switch from chrono to jiff. Each helper
//! below matches what the old chrono call did, so stored files written by
//! older versions read back the same.

use std::time::SystemTime;

pub use jiff::civil::DateTime;
use jiff::fmt::temporal::{Pieces, PiecesOffset};
use jiff::tz::TimeZone;
pub use jiff::SignedDuration;
use jiff::{Timestamp, ZonedDateTime};

/// The current moment in the PC's time zone.
pub fn now() -> ZonedDateTime {
    ZonedDateTime::now()
}

/// The PC's current wall-clock time.
pub fn now_local() -> DateTime {
    now().datetime()
}

/// The current moment, as an instant.
pub fn now_instant() -> Timestamp {
    Timestamp::now()
}

/// RFC 3339 with the fraction chrono's `to_rfc3339()` used (0, 3, 6 or 9 digits).
pub fn rfc3339(z: &ZonedDateTime) -> String {
    let ns = z.subsec_nanosecond();
    let frac = if ns == 0 {
        String::new()
    } else if ns % 1_000_000 == 0 {
        format!(".{:03}", ns / 1_000_000)
    } else if ns % 1_000 == 0 {
        format!(".{:06}", ns / 1_000)
    } else {
        format!(".{ns:09}")
    };
    format!(
        "{}{frac}{}",
        z.strftime("%Y-%m-%dT%H:%M:%S"),
        z.strftime("%:z")
    )
}

/// RFC 3339 to the second, like chrono's `to_rfc3339_opts(Secs, false)`.
pub fn rfc3339_secs(z: &ZonedDateTime) -> String {
    z.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

pub fn now_rfc3339() -> String {
    rfc3339(&now())
}

pub fn now_rfc3339_secs() -> String {
    rfc3339_secs(&now())
}

/// A file-name stamp like `2026-11-27_18-22-05`.
pub fn stamp() -> String {
    now().strftime("%Y-%m-%d_%H-%M-%S").to_string()
}

/// A file-name stamp with milliseconds, like `2026-11-27_18-22-05.123`.
pub fn stamp_ms() -> String {
    let z = now();
    format!(
        "{}.{:03}",
        z.strftime("%Y-%m-%d_%H-%M-%S"),
        z.subsec_nanosecond() / 1_000_000
    )
}

/// The wall-clock time written in an RFC 3339 string, in that string's own
/// offset (what chrono's `parse_from_rfc3339(s).naive_local()` returned).
/// The offset must be present, as before.
pub fn wall(s: &str) -> Option<DateTime> {
    let s = s.trim();
    if !rfc3339_shape(s) {
        return None;
    }
    let p = Pieces::parse(s).ok()?;
    match p.offset()? {
        PiecesOffset::Numeric(_) | PiecesOffset::Zulu => {}
        _ => return None,
    }
    if p.time_zone_annotation().is_some() {
        return None;
    }
    Some(p.date().to_datetime(p.time()?))
}

/// Exactly the RFC 3339 layout chrono accepted: `2026-11-25T10:00:00[.fraction]` then `Z` or
/// `±HH:MM`. jiff alone would also take `-0800`, `-08`, a comma fraction and `20261125T100000`.
fn rfc3339_shape(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return false;
    }
    let rest = &s[19..];
    let (frac, offset) = match rest.find(['Z', 'z', '+', '-']) {
        Some(i) => rest.split_at(i),
        None => return false,
    };
    let frac_ok = frac.is_empty()
        || (frac.len() > 1
            && frac.starts_with('.')
            && frac[1..].bytes().all(|c| c.is_ascii_digit()));
    let o = offset.as_bytes();
    let offset_ok = matches!(offset, "Z" | "z")
        || (o.len() == 6 && o[3] == b':' && [1, 2, 4, 5].iter().all(|&i| o[i].is_ascii_digit()));
    frac_ok && offset_ok
}

/// The instant an RFC 3339 string names (chrono's `parse_from_rfc3339`).
pub fn instant(s: &str) -> Option<Timestamp> {
    wall(s)?;
    s.trim().parse().ok()
}

/// Wall-clock time from `2026-11-25T17:29:29-0800` (chrono's
/// `parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z").naive_local()`).
pub fn wall_numeric_offset(s: &str) -> Option<DateTime> {
    // chrono's %z took both -0800 and -08:00.
    let t = jiff::fmt::strtime::parse("%Y-%m-%dT%H:%M:%S%z", s)
        .or_else(|_| jiff::fmt::strtime::parse("%Y-%m-%dT%H:%M:%S%:z", s))
        .ok()?;
    t.offset()?;
    t.to_datetime().ok()
}

/// A wall-clock time in the given layout (chrono's `NaiveDateTime::parse_from_str`).
pub fn parse(s: &str, layout: &str) -> Option<DateTime> {
    DateTime::strptime(layout, s).ok()
}

/// A file's time (modified, created) as the PC's wall-clock time.
pub fn from_system(t: SystemTime) -> Option<DateTime> {
    Some(
        Timestamp::try_from(t)
            .ok()?
            .to_zoned_datetime(TimeZone::system())
            .datetime(),
    )
}

/// The same, as RFC 3339 to the second.
pub fn system_rfc3339_secs(t: SystemTime) -> Option<String> {
    Some(rfc3339_secs(
        &Timestamp::try_from(t)
            .ok()?
            .to_zoned_datetime(TimeZone::system()),
    ))
}

/// A Unix time (UTC seconds, as Steam stores them) in the PC's time zone.
pub fn from_unix(secs: i64) -> Option<ZonedDateTime> {
    Some(
        Timestamp::from_second(secs)
            .ok()?
            .to_zoned_datetime(TimeZone::system()),
    )
}

/// Seconds since 1970 for a wall-clock time read as UTC (chrono's `and_utc().timestamp()`).
pub fn utc_seconds(t: DateTime) -> i64 {
    t.to_zoned_datetime(TimeZone::UTC)
        .map(|z| z.timestamp().as_second())
        .unwrap_or(0)
}

/// A wall-clock time in the PC's time zone.
pub fn local(t: DateTime) -> Option<ZonedDateTime> {
    t.to_zoned_datetime(TimeZone::system()).ok()
}

/// Build a wall-clock time. Out-of-range parts panic, as with chrono's `unwrap()`.
pub fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime {
    jiff::civil::date(y as i16, mo as i8, d as i8).at(h as i8, mi as i8, s as i8, 0)
}

/// `b - a`, as a duration (chrono's `b - a`).
pub fn between(a: DateTime, b: DateTime) -> SignedDuration {
    b.duration_since(a)
}

pub fn secs(n: i64) -> SignedDuration {
    SignedDuration::from_secs(n)
}
pub fn millis(n: i64) -> SignedDuration {
    SignedDuration::from_millis(n)
}
pub fn mins(n: i64) -> SignedDuration {
    SignedDuration::from_mins(n)
}
pub fn hours(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n)
}
pub fn days(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n * 24)
}

/// Whole days in a duration, rounded toward zero (chrono's `num_days`).
pub fn whole_days(d: SignedDuration) -> i64 {
    d.as_hours() / 24
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_keeps_the_clock_time_in_the_string() {
        // chrono's naive_local() kept 10:00 whatever the PC's zone; so must we.
        assert_eq!(
            wall("2026-11-25T10:00:00-08:00"),
            Some(at(2026, 11, 25, 10, 0, 0))
        );
        assert_eq!(
            wall("2026-11-25T10:00:00.123456+09:00").map(|t| t.time().hour()),
            Some(10)
        );
        assert_eq!(
            wall("2026-11-25T10:00:00Z"),
            Some(at(2026, 11, 25, 10, 0, 0))
        );
        assert_eq!(
            wall("2026-11-25T10:00:00"),
            None,
            "the offset is required, as before"
        );
        assert_eq!(wall("not a time"), None);
        // As strict as chrono was: these other spellings are refused.
        for s in [
            "2026-11-25T10:00:00-0800",
            "2026-11-25T10:00:00-08",
            "2026-11-25T10:00:00,5-08:00",
            "20261125T100000-0800",
            "2026-11-25T24:00:00-08:00",
        ] {
            assert_eq!(wall(s), None, "{s}");
        }
        assert!(wall("2026-11-25 10:00:00+14:00").is_some());
        assert!(wall("2026-11-25t10:00:00z").is_some());
    }

    #[test]
    fn instants_compare_across_offsets() {
        let a = instant("2026-11-25T10:00:00-08:00").unwrap();
        let b = instant("2026-11-26T03:00:00+09:00").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn numeric_offset_and_layouts() {
        assert_eq!(
            wall_numeric_offset("2026-11-25T17:29:29-0800"),
            Some(at(2026, 11, 25, 17, 29, 29))
        );
        assert_eq!(
            wall_numeric_offset("2026-11-25T17:29:29-08:00"),
            Some(at(2026, 11, 25, 17, 29, 29))
        );
        assert_eq!(wall_numeric_offset("2026-11-25T17:29:29"), None);
        assert_eq!(
            parse("2026-11-25 17:29", "%Y-%m-%d %H:%M"),
            Some(at(2026, 11, 25, 17, 29, 0))
        );
        assert_eq!(
            parse("2026 Nov 25 17:29:29", "%Y %b %d %H:%M:%S"),
            Some(at(2026, 11, 25, 17, 29, 29))
        );
        assert_eq!(parse("2026-11-25", "%Y-%m-%d %H:%M"), None);
        // GPU sample files written by chrono read back, and new ones look the same.
        let ts = "%Y/%m/%d %H:%M:%S%.3f";
        let t = parse("2026/11/25 17:29:29.120", ts).unwrap();
        assert_eq!(t.time().subsec_nanosecond(), 120_000_000);
        assert_eq!(t.strftime(ts).to_string(), "2026/11/25 17:29:29.120");
        assert_eq!(
            at(2026, 11, 25, 17, 29, 29).strftime(ts).to_string(),
            "2026/11/25 17:29:29.000"
        );
    }

    #[test]
    fn formats_match_chrono() {
        let t = at(2026, 11, 5, 7, 3, 9);
        assert_eq!(t.strftime("%b %-d %H:%M").to_string(), "Nov 5 07:03");
        assert_eq!(t.strftime("%b %-d at %H:%M").to_string(), "Nov 5 at 07:03");
        assert_eq!(
            t.strftime("%Y-%m-%dT%H:%M:%S").to_string(),
            "2026-11-05T07:03:09"
        );
        let z = t
            .to_zoned_datetime(TimeZone::fixed(jiff::tz::offset(-8)))
            .unwrap();
        assert_eq!(rfc3339(&z), "2026-11-05T07:03:09-08:00");
        assert_eq!(rfc3339_secs(&z), "2026-11-05T07:03:09-08:00");
        let z = z.with().subsec_nanosecond(120_000_000).build().unwrap();
        assert_eq!(rfc3339(&z), "2026-11-05T07:03:09.120-08:00");
        assert_eq!(wall(&rfc3339(&z)).map(|w| w.time().second()), Some(9));
    }

    #[test]
    fn durations() {
        let a = at(2026, 11, 25, 10, 0, 0);
        let b = a + hours(49);
        assert_eq!(between(a, b).as_secs(), 49 * 3600);
        assert_eq!(whole_days(between(a, b)), 2);
        assert_eq!(whole_days(between(b, a)), -2);
        assert_eq!(utc_seconds(at(1970, 1, 2, 0, 0, 0)), 86_400);
    }
}
