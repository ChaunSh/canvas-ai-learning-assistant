//! Deterministic study scheduling.
//!
//! The LLM may suggest what to study and how hard it is, but the *timetable* is
//! produced by this pure function. That keeps plans reproducible, testable and
//! free of hallucinated time slots (task specification §34).

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc, Weekday};

use crate::domain::{StudyPreferences, UnscheduledTask};
use crate::time::parse_hhmm;

#[derive(Debug, Clone)]
pub struct TaskCandidate {
    pub title: String,
    pub estimated_minutes: i64,
    pub deadline: Option<DateTime<Utc>>,
    pub priority: i64,
    pub course_id: Option<String>,
    pub material_id: Option<String>,
    pub assignment_id: Option<String>,
    pub source_references: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ScheduledBlock {
    pub candidate_index: usize,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub minutes: i64,
}

#[derive(Debug, Clone)]
pub struct PlannerOutput {
    pub blocks: Vec<ScheduledBlock>,
    pub unscheduled: Vec<UnscheduledTask>,
    pub explanation: Vec<String>,
}

struct DaySlot {
    date: NaiveDate,
    window_start: i64,
    window_end: i64,
    /// Busy intervals in local minutes since midnight.
    busy: Vec<(i64, i64)>,
    used_minutes: i64,
    capacity: i64,
}

impl DaySlot {
    fn free_minutes(&self) -> i64 {
        (self.capacity - self.used_minutes).max(0)
    }
}

/// Build a timetable from preferences, existing commitments and candidates.
pub fn plan(
    start_date: NaiveDate,
    end_date: NaiveDate,
    preferences: &StudyPreferences,
    existing: &[(DateTime<Utc>, DateTime<Utc>)],
    candidates: Vec<TaskCandidate>,
) -> PlannerOutput {
    let mut explanation = Vec::new();
    let tz: chrono_tz::Tz = preferences.timezone.parse().unwrap_or(chrono_tz::UTC);
    let window_start = parse_hhmm(&preferences.preferred_start).unwrap_or(19 * 60) as i64;
    let window_end = parse_hhmm(&preferences.preferred_end).unwrap_or(22 * 60 + 30) as i64;
    let session = preferences.session_minutes.clamp(10, 240);
    let pause = preferences.break_minutes.clamp(0, 120);
    let daily_cap = preferences.daily_minutes.max(15);

    if window_end <= window_start {
        return PlannerOutput {
            blocks: Vec::new(),
            unscheduled: candidates
                .into_iter()
                .map(|c| UnscheduledTask {
                    title: c.title,
                    estimated_minutes: c.estimated_minutes,
                    deadline: c.deadline.map(crate::time::to_iso),
                    reason: "学习时段设置无效".into(),
                })
                .collect(),
            explanation: vec!["学习时段结束时间早于开始时间，未生成任何安排。".into()],
        };
    }

    // --- day slots ---------------------------------------------------------
    let mut days: Vec<DaySlot> = Vec::new();
    let mut cursor_date = start_date;
    while cursor_date <= end_date {
        let weekday = cursor_date.weekday();
        let iso = weekday.number_from_monday() as i64;
        let is_study_day = preferences.study_days.contains(&iso);
        let is_weekend = matches!(weekday, Weekday::Sat | Weekday::Sun);
        if is_study_day && !(preferences.avoid_weekends && is_weekend) {
            let mut busy = Vec::new();
            for (from, to) in existing {
                if let Some(range) = clamp_to_local_day(&tz, cursor_date, *from, *to, window_start, window_end) {
                    busy.push(range);
                }
            }
            busy.sort_by_key(|(a, _)| *a);
            let window_len = window_end - window_start;
            days.push(DaySlot {
                date: cursor_date,
                window_start,
                window_end,
                busy,
                used_minutes: 0,
                capacity: daily_cap.min(window_len),
            });
        }
        cursor_date += Duration::days(1);
    }

    if days.is_empty() {
        return PlannerOutput {
            blocks: Vec::new(),
            unscheduled: candidates
                .into_iter()
                .map(|c| UnscheduledTask {
                    title: c.title,
                    estimated_minutes: c.estimated_minutes,
                    deadline: c.deadline.map(crate::time::to_iso),
                    reason: "所选时间范围内没有学习日".into(),
                })
                .collect(),
            explanation: vec!["所选时间范围内没有可用的学习日（请检查学习日设置）。".into()],
        };
    }

    // Highest urgency first: earlier deadline, then higher priority (1 is most
    // important in our schema), then longer work first so big items get room.
    // Sort indices rather than the vector so `candidate_index` keeps referring to
    // the caller's original position (the task → source mapping depends on it).
    let mut order: Vec<(usize, &TaskCandidate)> = candidates.iter().enumerate().collect();
    order.sort_by(|(_, a), (_, b)| {
        a.deadline
            .cmp(&b.deadline)
            .then_with(|| a.deadline.is_none().cmp(&b.deadline.is_none()))
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| b.estimated_minutes.cmp(&a.estimated_minutes))
            .then_with(|| a.title.cmp(&b.title))
    });

    let mut blocks = Vec::new();
    let mut unscheduled = Vec::new();
    let mut total_scheduled = 0i64;

    for (index, candidate) in order.iter().copied() {
        let mut remaining = candidate.estimated_minutes.max(5);
        let deadline_local_date = candidate
            .deadline
            .map(|d| d.with_timezone(&tz).date_naive());

        for day in days.iter_mut() {
            if remaining <= 0 {
                break;
            }
            if let Some(limit) = deadline_local_date {
                if day.date > limit {
                    break;
                }
            }
            if day.free_minutes() <= 0 {
                continue;
            }

            // Local midnight for this day; DST gaps fall back to the latest
            // valid instant and finally to a UTC interpretation.
            let midnight = day
                .date
                .and_hms_opt(0, 0, 0)
                .expect("midnight is always valid");
            let anchor = tz
                .from_local_datetime(&midnight)
                .earliest()
                .or_else(|| tz.from_local_datetime(&midnight).latest())
                .unwrap_or_else(|| tz.from_utc_datetime(&midnight));

            loop {
                if remaining <= 0 {
                    break;
                }
                let free = day.free_minutes();
                if free <= 0 {
                    break;
                }
                let desired = remaining.min(session).min(free);
                if desired < 5 {
                    break;
                }
                let Some((start_min, end_min)) = next_free(day, desired) else {
                    break;
                };

                let start = (anchor + Duration::minutes(start_min)).with_timezone(&Utc);
                let end = (anchor + Duration::minutes(end_min)).with_timezone(&Utc);

                // Never schedule past a hard deadline.
                if let Some(deadline) = candidate.deadline {
                    if end > deadline {
                        break;
                    }
                }

                blocks.push(ScheduledBlock {
                    candidate_index: index,
                    start,
                    end,
                    minutes: end_min - start_min,
                });
                day.busy.push((start_min, end_min));
                day.busy.sort_by_key(|(a, _)| *a);
                day.used_minutes += end_min - start_min;
                remaining -= end_min - start_min;
                total_scheduled += end_min - start_min;

                // Advance the cursor past the break so sessions are not back-to-back.
                if pause > 0 {
                    let resume = end_min + pause;
                    if resume < day.window_end {
                        day.busy.push((end_min, resume));
                        day.busy.sort_by_key(|(a, _)| *a);
                    }
                }
            }
        }

        if remaining > 0 {
            unscheduled.push(UnscheduledTask {
                title: candidate.title.clone(),
                estimated_minutes: remaining,
                deadline: candidate.deadline.map(crate::time::to_iso),
                reason: if deadline_local_date.is_some() {
                    "截止时间前没有足够的空闲时段".into()
                } else {
                    "计划范围内的可用时段已用尽".into()
                },
            });
        }
    }

    explanation.push(format!(
        "共 {} 个学习日，每日上限 {} 分钟；每次 {} 分钟，间隔 {} 分钟。",
        days.len(),
        daily_cap,
        session,
        pause
    ));
    explanation.push(format!(
        "已安排 {} 段，共 {} 分钟；未能安排 {} 项。",
        blocks.len(),
        total_scheduled,
        unscheduled.len()
    ));
    if !unscheduled.is_empty() {
        explanation.push(
            "未能安排的任务通常意味着截止时间过近或每日时长不足，可提高每日时长或缩小任务范围。"
                .into(),
        );
    }

    PlannerOutput {
        blocks,
        unscheduled,
        explanation,
    }
}

/// Find the next free interval of `duration` minutes inside the day's window.
fn next_free(day: &DaySlot, duration: i64) -> Option<(i64, i64)> {
    let mut cursor = day.window_start;
    for (busy_start, busy_end) in &day.busy {
        if *busy_end <= cursor {
            continue;
        }
        if cursor + duration <= *busy_start {
            return Some((cursor, cursor + duration));
        }
        cursor = (*busy_end).max(cursor);
    }
    if cursor + duration <= day.window_end {
        Some((cursor, cursor + duration))
    } else {
        None
    }
}

/// Project a UTC interval onto one local calendar day and clip it to the window.
fn clamp_to_local_day(
    tz: &chrono_tz::Tz,
    date: NaiveDate,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    window_start: i64,
    window_end: i64,
) -> Option<(i64, i64)> {
    let day_start = tz
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()?;
    let day_end = day_start + Duration::days(1);
    let from_local = from.with_timezone(tz);
    let to_local = to.with_timezone(tz);
    if from_local >= day_end || to_local <= day_start {
        return None;
    }
    let start_minutes = (from_local - day_start).num_minutes().max(window_start);
    let end_minutes = (to_local - day_start).num_minutes().min(window_end);
    if end_minutes <= start_minutes {
        return None;
    }
    Some((start_minutes, end_minutes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs() -> StudyPreferences {
        StudyPreferences {
            daily_minutes: 120,
            session_minutes: 50,
            break_minutes: 10,
            preferred_start: "19:00".into(),
            preferred_end: "22:30".into(),
            study_days: vec![1, 2, 3, 4, 5, 6, 7],
            timezone: "Asia/Shanghai".into(),
            avoid_weekends: false,
            updated_at: "t".into(),
        }
    }

    fn candidate(title: &str, minutes: i64, deadline: Option<&str>) -> TaskCandidate {
        TaskCandidate {
            title: title.into(),
            estimated_minutes: minutes,
            deadline: deadline.and_then(crate::time::parse_iso),
            priority: 3,
            course_id: None,
            material_id: None,
            assignment_id: None,
            source_references: vec![],
        }
    }

    fn date(s: &str) -> NaiveDate {
        crate::time::parse_date(s).unwrap()
    }

    #[test]
    fn respects_daily_capacity() {
        let out = plan(
            date("2026-11-02"),
            date("2026-11-02"),
            &prefs(),
            &[],
            vec![candidate("Big task", 300, None)],
        );
        let total: i64 = out.blocks.iter().map(|b| b.minutes).sum();
        assert_eq!(total, 120);
        assert_eq!(out.unscheduled.len(), 1);
        assert_eq!(out.unscheduled[0].estimated_minutes, 180);
    }

    #[test]
    fn splits_into_sessions_with_breaks() {
        let out = plan(
            date("2026-11-02"),
            date("2026-11-02"),
            &prefs(),
            &[],
            vec![candidate("Reading", 100, None)],
        );
        // 50 + 10 break + 50 = within a 120-minute budget.
        assert_eq!(out.blocks.len(), 2);
        assert!(out.blocks[0].minutes <= 50);
        let gap = out.blocks[1].start - out.blocks[0].end;
        assert_eq!(gap.num_minutes(), 10);
    }

    #[test]
    fn never_exceeds_deadline() {
        let deadline = "2026-11-02T12:00:00Z"; // 20:00 Shanghai on the 2nd
        let out = plan(
            date("2026-11-01"),
            date("2026-11-05"),
            &prefs(),
            &[],
            vec![candidate("Lab report", 200, Some(deadline))],
        );
        let parsed = crate::time::parse_iso(deadline).unwrap();
        assert!(out.blocks.iter().all(|b| b.end <= parsed));
        assert!(!out.blocks.is_empty());
    }

    #[test]
    fn avoids_existing_commitments() {
        let busy_from = "2026-11-02T11:00:00Z".parse::<DateTime<Utc>>().unwrap(); // 19:00 local
        let busy_to = "2026-11-02T12:00:00Z".parse::<DateTime<Utc>>().unwrap(); // 20:00 local
        let out = plan(
            date("2026-11-02"),
            date("2026-11-02"),
            &prefs(),
            &[(busy_from, busy_to)],
            vec![candidate("Study", 50, None)],
        );
        assert_eq!(out.blocks.len(), 1);
        assert!(out.blocks[0].start >= busy_to);
    }

    #[test]
    fn skips_days_not_selected() {
        let mut p = prefs();
        p.study_days = vec![1]; // Monday only
        let out = plan(
            date("2026-11-03"), // Tuesday
            date("2026-11-03"),
            &p,
            &[],
            vec![candidate("Study", 50, None)],
        );
        assert!(out.blocks.is_empty());
        assert_eq!(out.unscheduled.len(), 1);
        assert!(out.unscheduled[0].reason.contains("学习日"));
    }

    #[test]
    fn avoid_weekends_is_honoured() {
        let mut p = prefs();
        p.avoid_weekends = true;
        // 2026-11-07 is a Saturday.
        let out = plan(
            date("2026-11-07"),
            date("2026-11-08"),
            &p,
            &[],
            vec![candidate("Weekend study", 50, None)],
        );
        assert!(out.blocks.is_empty());
    }

    #[test]
    fn earlier_deadline_gets_scheduled_first() {
        let out = plan(
            date("2026-11-02"),
            date("2026-11-02"),
            &prefs(),
            &[],
            vec![
                candidate("Later", 90, Some("2026-11-10T00:00:00Z")),
                candidate("Sooner", 90, Some("2026-11-03T00:00:00Z")),
            ],
        );
        assert!(!out.blocks.is_empty());
        // The first block must belong to "Sooner" (candidate index 1 after sorting).
        assert_eq!(out.blocks[0].candidate_index, 1);
    }

    #[test]
    fn empty_range_produces_explanation() {
        let out = plan(
            date("2026-11-02"),
            date("2026-11-01"),
            &prefs(),
            &[],
            vec![candidate("x", 30, None)],
        );
        assert!(out.blocks.is_empty());
        assert!(out.unscheduled[0].reason.contains("学习日"));
    }

    #[test]
    fn invalid_window_is_rejected() {
        let mut p = prefs();
        p.preferred_start = "22:00".into();
        p.preferred_end = "20:00".into();
        let out = plan(
            date("2026-11-02"),
            date("2026-11-02"),
            &p,
            &[],
            vec![candidate("x", 30, None)],
        );
        assert!(out.blocks.is_empty());
        assert!(out.explanation[0].contains("早于开始时间"));
    }
}
