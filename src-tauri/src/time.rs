//! Time handling.
//!
//! Persisted instants are ISO-8601 UTC strings with a trailing `Z`
//! (`2026-10-17T12:34:56Z`). They sort lexicographically, parse in JS via
//! `new Date(...)`, and avoid implicit local-time maths in SQLite.
//!
//! Calendar *dates* (study-plan boundaries) are local calendar days
//! (`YYYY-MM-DD`) and are deliberately a different type from instants.

use chrono::{DateTime, NaiveDate, SecondsFormat, TimeZone, Utc};

/// Current instant as a database timestamp.
pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Format an arbitrary instant.
pub fn to_iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Parse an ISO-8601 instant. Accepts `Z` and explicit offsets; returns `None`
/// for empty or malformed input so callers can surface a field-specific error.
pub fn parse_iso(value: &str) -> Option<DateTime<Utc>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    match DateTime::parse_from_rfc3339(trimmed) {
        Ok(dt) => Some(dt.with_timezone(&Utc)),
        Err(_) => NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|naive| Utc.from_utc_datetime(&naive)),
    }
}

/// Parse a calendar day (`YYYY-MM-DD`).
pub fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()
}

/// Today's calendar day in the supplied IANA zone, falling back to UTC.
pub fn today_in_zone(tz: &str) -> NaiveDate {
    let zone: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    Utc::now().with_timezone(&zone).date_naive()
}

/// Local wall-clock time (`HH:MM` or `HH:MM:SS`) to minutes since midnight.
pub fn parse_hhmm(value: &str) -> Option<u32> {
    let mut parts = value.trim().split(':');
    let h: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(h * 60 + m)
}

/// Minutes since midnight to `HH:MM`.
pub fn format_hhmm(minutes: u32) -> String {
    format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_iso_is_utc_with_z() {
        let s = now_iso();
        assert!(s.ends_with('Z'), "expected UTC Z suffix, got {s}");
        assert_eq!(s.len(), 20, "expected second precision, got {s}");
    }

    #[test]
    fn parse_iso_accepts_offsets_and_dates() {
        let a = parse_iso("2026-10-17T20:00:00+08:00").unwrap();
        assert_eq!(to_iso(a), "2026-10-17T12:00:00Z");
        let b = parse_iso("2026-10-17").unwrap();
        assert_eq!(to_iso(b), "2026-10-17T00:00:00Z");
    }

    #[test]
    fn parse_iso_rejects_garbage() {
        assert!(parse_iso("").is_none());
        assert!(parse_iso("not a date").is_none());
    }

    #[test]
    fn hhmm_roundtrip() {
        assert_eq!(parse_hhmm("19:30"), Some(19 * 60 + 30));
        assert_eq!(format_hhmm(19 * 60 + 30), "19:30");
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("07:60"), None);
    }

    #[test]
    fn today_uses_zone() {
        let d = today_in_zone("Asia/Shanghai");
        let u = today_in_zone("UTC");
        // Shanghai is never behind UTC, so it is the same or the next day.
        assert!(d >= u);
    }
}
