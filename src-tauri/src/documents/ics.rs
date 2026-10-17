//! Minimal iCalendar (RFC 5545) reader for the calendar import feature.
//!
//! Only what a student timetable needs: `VEVENT` with UID, SUMMARY, DESCRIPTION,
//! LOCATION, DTSTART, DTEND. Recurrence rules are *not* expanded — we import the
//! master event and record that recurrence was present so the UI can say so
//! rather than silently showing a single occurrence.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

use crate::error::{AppError, AppResult};
use crate::time::to_iso;

#[derive(Debug, Clone)]
pub struct IcsEvent {
    pub uid: String,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub starts_at: String,
    pub ends_at: Option<String>,
    pub all_day: bool,
    /// True when the source event had an RRULE we did not expand.
    pub recurring: bool,
}

/// Parse an ICS document into events. Malformed events are skipped with a log
/// line rather than failing the whole import.
pub fn parse(input: &str) -> AppResult<Vec<IcsEvent>> {
    let unfolded = unfold(input);
    let mut events = Vec::new();
    let mut current: Option<RawEvent> = None;

    for line in unfolded.lines() {
        let line = line.trim_end_matches('\r');
        if line.eq_ignore_ascii_case("BEGIN:VEVENT") {
            current = Some(RawEvent::default());
            continue;
        }
        if line.eq_ignore_ascii_case("END:VEVENT") {
            if let Some(raw) = current.take() {
                match raw.into_event() {
                    Ok(ev) => events.push(ev),
                    Err(e) => tracing::warn!(error = %e, "skipping malformed VEVENT"),
                }
            }
            continue;
        }
        let Some(raw) = current.as_mut() else { continue };
        let Some((name, value)) = split_property(line) else {
            continue;
        };
        match name.as_str() {
            "UID" => raw.uid = Some(value),
            "SUMMARY" => raw.summary = Some(unescape_text(&value)),
            "DESCRIPTION" => raw.description = Some(unescape_text(&value)),
            "LOCATION" => raw.location = Some(unescape_text(&value)),
            "DTSTART" => raw.start = Some(value),
            "DTEND" => raw.end = Some(value),
            "RRULE" => raw.recurring = true,
            _ => {}
        }
    }

    if events.is_empty() {
        return Err(AppError::invalid(
            "ICS 文件中未找到任何日程（VEVENT）",
        ));
    }
    Ok(events)
}

/// RFC 5545 line folding: a CRLF followed by a space/tab continues the line.
fn unfold(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut lines = input.split_inclusive('\n');
    let mut pending: Option<String> = None;
    for line in lines.by_ref() {
        let stripped = line.trim_end_matches(['\r', '\n']);
        let is_continuation = stripped.starts_with(' ') || stripped.starts_with('\t');
        if is_continuation {
            if let Some(buf) = pending.as_mut() {
                buf.push_str(&stripped[1..]);
                continue;
            }
        }
        if let Some(buf) = pending.take() {
            out.push_str(&buf);
            out.push('\n');
        }
        pending = Some(stripped.to_string());
    }
    if let Some(buf) = pending {
        out.push_str(&buf);
    }
    out
}

/// Split `KEY;PARAM=VAL:value` handling quoted parameter values.
fn split_property(line: &str) -> Option<(String, String)> {
    let mut in_quotes = false;
    for (idx, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                let key = &line[..idx];
                let value = &line[idx + 1..];
                let name = key.split(';').next()?.to_string();
                return Some((name, value.to_string()));
            }
            _ => {}
        }
    }
    None
}

fn unescape_text(value: &str) -> String {
    value
        .replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

#[derive(Default)]
struct RawEvent {
    uid: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    start: Option<String>,
    end: Option<String>,
    recurring: bool,
}

impl RawEvent {
    fn into_event(self) -> AppResult<IcsEvent> {
        let start_raw = self
            .start
            .ok_or_else(|| AppError::invalid("VEVENT 缺少 DTSTART"))?;
        let start = parse_ics_datetime(&start_raw)
            .ok_or_else(|| AppError::invalid(format!("无法解析 DTSTART：{start_raw}")))?;
        let end = self.end.as_ref().and_then(|v| parse_ics_datetime(v));

        let summary = self
            .summary
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "未命名日程".to_string());

        let uid = self.uid.filter(|u| !u.trim().is_empty()).unwrap_or_else(|| {
            // Deterministic fallback so re-importing the same feed still dedupes.
            format!("{}@{}", summary, start.format("%Y%m%dT%H%M%S"))
        });

        Ok(IcsEvent {
            uid,
            summary,
            description: self.description,
            location: self.location,
            starts_at: to_iso(start),
            ends_at: end.map(to_iso),
            all_day: start_raw.len() == 8,
            recurring: self.recurring,
        })
    }
}

/// Parse the ICS date/date-time forms. Floating (zoneless) times are treated as
/// UTC and the UI states that assumption.
fn parse_ics_datetime(raw: &str) -> Option<DateTime<Utc>> {
    let value = raw.trim();
    // YYYYMMDD
    if value.len() == 8 {
        let date = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?));
    }
    // YYYYMMDDTHHMMSS[Z]
    let trimmed = value.trim_end_matches('Z');
    if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, "%Y%m%dT%H%M%S") {
        return Some(Utc.from_utc_datetime(&naive));
    }
    // YYYY-MM-DDTHH:MM:SS variants
    if let Some(dt) = crate::time::parse_iso(value) {
        return Some(dt);
    }
    None
}

use chrono::TimeZone;

/// Render calendar events as an RFC 5545 document.
///
/// Times are emitted in UTC (`...Z`) so the export is unambiguous regardless of
/// the receiving client's timezone.
pub fn render(events: &[crate::domain::CalendarEvent]) -> String {
    let mut out = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Canvas AI Learning Assistant//EN\r\nCALSCALE:GREGORIAN\r\n",
    );
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();

    for event in events {
        out.push_str("BEGIN:VEVENT\r\n");
        let uid = event
            .ics_uid
            .clone()
            .or_else(|| event.external_id.clone())
            .unwrap_or_else(|| format!("{}@canvas-ai-assistant", event.id));
        out.push_str(&format!("UID:{}\r\n", escape_text(&uid)));
        out.push_str(&format!("DTSTAMP:{stamp}\r\n"));
        out.push_str(&format!("SUMMARY:{}\r\n", escape_text(&event.title)));

        if let Some(start) = crate::time::parse_iso(&event.starts_at) {
            if event.all_day {
                out.push_str(&format!("DTSTART;VALUE=DATE:{}\r\n", start.format("%Y%m%d")));
            } else {
                out.push_str(&format!("DTSTART:{}\r\n", start.format("%Y%m%dT%H%M%SZ")));
            }
        }
        if let Some(end) = event.ends_at.as_deref().and_then(crate::time::parse_iso) {
            if event.all_day {
                out.push_str(&format!("DTEND;VALUE=DATE:{}\r\n", end.format("%Y%m%d")));
            } else {
                out.push_str(&format!("DTEND:{}\r\n", end.format("%Y%m%dT%H%M%SZ")));
            }
        }
        if let Some(description) = &event.description {
            if !description.trim().is_empty() {
                out.push_str(&format!("DESCRIPTION:{}\r\n", escape_text(description)));
            }
        }
        if let Some(location) = &event.location {
            if !location.trim().is_empty() {
                out.push_str(&format!("LOCATION:{}\r\n", escape_text(location)));
            }
        }
        out.push_str(&format!("CATEGORIES:{}\r\n", escape_text(&event.kind)));
        out.push_str("END:VEVENT\r\n");
    }

    out.push_str("END:VCALENDAR\r\n");
    out
}

fn escape_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\r', "")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:event-1@example.edu\r\n\
SUMMARY:Midterm Exam\r\n\
LOCATION:Room 2405\r\n\
DTSTART:20261101T020000Z\r\n\
DTEND:20261101T040000Z\r\n\
DESCRIPTION:Chapters 1-4\\nClosed book\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn parses_basic_event() {
        let events = parse(SAMPLE).unwrap();
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.uid, "event-1@example.edu");
        assert_eq!(e.summary, "Midterm Exam");
        assert_eq!(e.location.as_deref(), Some("Room 2405"));
        assert_eq!(e.starts_at, "2026-11-01T02:00:00Z");
        assert_eq!(e.ends_at.as_deref(), Some("2026-11-01T04:00:00Z"));
        assert!(!e.all_day);
        assert_eq!(e.description.as_deref(), Some("Chapters 1-4\nClosed book"));
    }

    #[test]
    fn unfolds_long_lines() {
        let ics = "BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:A very long summary that has been\r\n folded\r\nDTSTART:20261102T000000Z\r\nEND:VEVENT\r\n";
        let events = parse(ics).unwrap();
        assert_eq!(events[0].summary, "A very long summary that has beenfolded");
    }

    #[test]
    fn all_day_events() {
        let ics = "BEGIN:VEVENT\r\nUID:y\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261225\r\nEND:VEVENT\r\n";
        let events = parse(ics).unwrap();
        assert!(events[0].all_day);
        assert_eq!(events[0].starts_at, "2026-12-25T00:00:00Z");
    }

    #[test]
    fn recurrence_is_flagged_not_expanded() {
        let ics = "BEGIN:VEVENT\r\nUID:z\r\nSUMMARY:Weekly Lab\r\nDTSTART:20261103T060000Z\r\nRRULE:FREQ=WEEKLY;COUNT=5\r\nEND:VEVENT\r\n";
        let events = parse(ics).unwrap();
        assert!(events[0].recurring);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn events_without_start_are_skipped() {
        let ics = "BEGIN:VEVENT\r\nUID:bad\r\nSUMMARY:No start\r\nEND:VEVENT\r\n";
        assert!(parse(ics).is_err());
    }

    #[test]
    fn missing_uid_gets_deterministic_fallback() {
        let ics = "BEGIN:VEVENT\r\nSUMMARY:No uid\r\nDTSTART:20261101T000000Z\r\nEND:VEVENT\r\n";
        let a = parse(ics).unwrap();
        let b = parse(ics).unwrap();
        assert_eq!(a[0].uid, b[0].uid);
    }
}
