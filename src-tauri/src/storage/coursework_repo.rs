//! Assignments, announcements, calendar events, grades, discussions, pages.

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::domain::{
    Announcement, Assignment, CalendarEvent, CoursePage, CreateAssignmentInput, CreateEventInput,
    DiscussionEntry, DiscussionTopic, GradeRecord, UpdateAssignmentInput, UpcomingDeadline,
};
use crate::error::{AppError, AppResult};
use crate::time::now_iso;

// ---------------------------------------------------------------------------
// Assignments
// ---------------------------------------------------------------------------

const ASSIGNMENT_SELECT: &str = r#"
SELECT a.id, a.course_id, c.name, a.external_id, a.title, a.description, a.due_at, a.unlock_at,
       a.points_possible, a.submission_status, a.html_url, a.source, a.created_at, a.updated_at
FROM assignments a LEFT JOIN courses c ON c.id = a.course_id
"#;

fn map_assignment(row: &Row<'_>) -> rusqlite::Result<Assignment> {
    Ok(Assignment {
        id: row.get(0)?,
        course_id: row.get(1)?,
        course_name: row.get(2)?,
        external_id: row.get(3)?,
        title: row.get(4)?,
        description: row.get(5)?,
        due_at: row.get(6)?,
        unlock_at: row.get(7)?,
        points_possible: row.get(8)?,
        submission_status: row.get(9)?,
        html_url: row.get(10)?,
        source: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

/// List assignments, optionally filtered by course. `upcoming_only` restricts
/// to items with a due date at or after the start of today (UTC).
pub fn list_assignments(
    conn: &Connection,
    course_id: Option<&str>,
    upcoming_only: bool,
    limit: i64,
) -> AppResult<Vec<Assignment>> {
    let cutoff = crate::time::to_iso(chrono::Utc::now() - chrono::Duration::days(1));
    let sql = format!(
        "{ASSIGNMENT_SELECT}
         WHERE (?1 IS NULL OR a.course_id = ?1)
           AND (?2 = 0 OR (a.due_at IS NOT NULL AND a.due_at >= ?3))
         ORDER BY (a.due_at IS NULL), a.due_at ASC, a.title ASC
         LIMIT ?4"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            course_id,
            if upcoming_only { 1 } else { 0 },
            cutoff,
            limit
        ],
        map_assignment,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn get_assignment(conn: &Connection, id: &str) -> AppResult<Assignment> {
    let sql = format!("{ASSIGNMENT_SELECT} WHERE a.id = ?1");
    conn.query_row(&sql, [id], map_assignment)
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("作业不存在：{id}")))
}

pub fn create_assignment(conn: &Connection, input: CreateAssignmentInput) -> AppResult<Assignment> {
    if input.title.trim().is_empty() {
        return Err(AppError::invalid("作业标题不能为空"));
    }
    if let Some(due) = &input.due_at {
        if crate::time::parse_iso(due).is_none() {
            return Err(AppError::invalid("截止时间格式无效，应为 ISO-8601"));
        }
    }
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO assignments (id, course_id, external_id, title, description, due_at, unlock_at,
                                  points_possible, submission_status, html_url, source, created_at, updated_at)
         VALUES (?1, ?2, NULL, ?3, ?4, ?5, NULL, ?6, 'unknown', NULL, 'manual', ?7, ?7)",
        params![
            id,
            input.course_id,
            input.title.trim(),
            input.description,
            input.due_at,
            input.points_possible,
            now
        ],
    )?;
    get_assignment(conn, &id)
}

pub fn update_assignment(
    conn: &Connection,
    id: &str,
    patch: UpdateAssignmentInput,
) -> AppResult<Assignment> {
    get_assignment(conn, id)?;
    conn.execute(
        "UPDATE assignments SET
            title             = COALESCE(?2, title),
            description       = COALESCE(?3, description),
            due_at            = CASE WHEN ?4 = 1 THEN ?5 ELSE due_at END,
            points_possible   = CASE WHEN ?6 = 1 THEN ?7 ELSE points_possible END,
            submission_status = COALESCE(?8, submission_status),
            updated_at        = ?9
         WHERE id = ?1",
        params![
            id,
            patch.title,
            patch.description,
            if patch.due_at.is_some() { 1 } else { 0 },
            patch.due_at.flatten(),
            if patch.points_possible.is_some() { 1 } else { 0 },
            patch.points_possible.flatten(),
            patch.submission_status,
            now_iso()
        ],
    )?;
    get_assignment(conn, id)
}

pub fn delete_assignment(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM assignments WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AppError::not_found(format!("作业不存在：{id}")));
    }
    Ok(())
}

pub struct AssignmentUpsert<'a> {
    pub course_id: &'a str,
    pub external_id: &'a str,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub due_at: Option<&'a str>,
    pub unlock_at: Option<&'a str>,
    pub points_possible: Option<f64>,
    pub submission_status: &'a str,
    pub html_url: Option<&'a str>,
}

pub fn upsert_canvas_assignment(conn: &Connection, seed: &AssignmentUpsert<'_>) -> AppResult<bool> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM assignments WHERE course_id = ?1 AND external_id = ?2",
            params![seed.course_id, seed.external_id],
            |r| r.get(0),
        )
        .optional()?;
    let now = now_iso();
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE assignments SET title = ?2, description = ?3, due_at = ?4, unlock_at = ?5,
                        points_possible = ?6, submission_status = ?7, html_url = ?8,
                        source = 'canvas', updated_at = ?9
                 WHERE id = ?1",
                params![
                    id,
                    seed.title,
                    seed.description,
                    seed.due_at,
                    seed.unlock_at,
                    seed.points_possible,
                    seed.submission_status,
                    seed.html_url,
                    now
                ],
            )?;
            Ok(false)
        }
        None => {
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO assignments (id, course_id, external_id, title, description, due_at,
                                          unlock_at, points_possible, submission_status, html_url,
                                          source, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'canvas', ?11, ?11)",
                params![
                    id,
                    seed.course_id,
                    seed.external_id,
                    seed.title,
                    seed.description,
                    seed.due_at,
                    seed.unlock_at,
                    seed.points_possible,
                    seed.submission_status,
                    seed.html_url,
                    now
                ],
            )?;
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------------------
// Announcements
// ---------------------------------------------------------------------------

const ANNOUNCEMENT_SELECT: &str = r#"
SELECT n.id, n.course_id, c.name, n.external_id, n.title, n.body, n.author, n.posted_at,
       n.html_url, n.read, n.created_at, n.updated_at
FROM announcements n LEFT JOIN courses c ON c.id = n.course_id
"#;

fn map_announcement(row: &Row<'_>) -> rusqlite::Result<Announcement> {
    Ok(Announcement {
        id: row.get(0)?,
        course_id: row.get(1)?,
        course_name: row.get(2)?,
        external_id: row.get(3)?,
        title: row.get(4)?,
        body: row.get(5)?,
        author: row.get(6)?,
        posted_at: row.get(7)?,
        html_url: row.get(8)?,
        read: crate::storage::db::b(row.get::<_, i64>(9)?),
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

pub fn list_announcements(
    conn: &Connection,
    course_id: Option<&str>,
    limit: i64,
) -> AppResult<Vec<Announcement>> {
    let sql = format!(
        "{ANNOUNCEMENT_SELECT} WHERE (?1 IS NULL OR n.course_id = ?1)
         ORDER BY n.posted_at DESC LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![course_id, limit], map_announcement)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn mark_announcement_read(conn: &Connection, id: &str, read: bool) -> AppResult<()> {
    conn.execute(
        "UPDATE announcements SET read = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, crate::storage::db::i(read), now_iso()],
    )?;
    Ok(())
}

pub struct AnnouncementUpsert<'a> {
    pub course_id: &'a str,
    pub external_id: &'a str,
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub author: Option<&'a str>,
    pub posted_at: Option<&'a str>,
    pub html_url: Option<&'a str>,
}

pub fn upsert_canvas_announcement(
    conn: &Connection,
    seed: &AnnouncementUpsert<'_>,
) -> AppResult<bool> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM announcements WHERE course_id = ?1 AND external_id = ?2",
            params![seed.course_id, seed.external_id],
            |r| r.get(0),
        )
        .optional()?;
    let now = now_iso();
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE announcements SET title = ?2, body = ?3, author = ?4, posted_at = ?5,
                        html_url = ?6, updated_at = ?7 WHERE id = ?1",
                params![
                    id,
                    seed.title,
                    seed.body,
                    seed.author,
                    seed.posted_at,
                    seed.html_url,
                    now
                ],
            )?;
            Ok(false)
        }
        None => {
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO announcements (id, course_id, external_id, title, body, author,
                                            posted_at, html_url, read, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?9)",
                params![
                    id,
                    seed.course_id,
                    seed.external_id,
                    seed.title,
                    seed.body,
                    seed.author,
                    seed.posted_at,
                    seed.html_url,
                    now
                ],
            )?;
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------------------
// Calendar events
// ---------------------------------------------------------------------------

const EVENT_SELECT: &str = r#"
SELECT e.id, e.course_id, c.name, e.external_id, e.ics_uid, e.title, e.description, e.location,
       e.starts_at, e.ends_at, e.all_day, e.kind, e.source, e.assignable_id, e.created_at, e.updated_at
FROM calendar_events e LEFT JOIN courses c ON c.id = e.course_id
"#;

fn map_event(row: &Row<'_>) -> rusqlite::Result<CalendarEvent> {
    Ok(CalendarEvent {
        id: row.get(0)?,
        course_id: row.get(1)?,
        course_name: row.get(2)?,
        external_id: row.get(3)?,
        ics_uid: row.get(4)?,
        title: row.get(5)?,
        description: row.get(6)?,
        location: row.get(7)?,
        starts_at: row.get(8)?,
        ends_at: row.get(9)?,
        all_day: crate::storage::db::b(row.get::<_, i64>(10)?),
        kind: row.get(11)?,
        source: row.get(12)?,
        assignable_id: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

pub fn list_events(
    conn: &Connection,
    from: Option<&str>,
    to: Option<&str>,
) -> AppResult<Vec<CalendarEvent>> {
    let sql = format!(
        "{EVENT_SELECT}
         WHERE (?1 IS NULL OR e.starts_at >= ?1) AND (?2 IS NULL OR e.starts_at <= ?2)
         ORDER BY e.starts_at ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![from, to], map_event)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn create_event(conn: &Connection, input: CreateEventInput) -> AppResult<CalendarEvent> {
    if input.title.trim().is_empty() {
        return Err(AppError::invalid("日程标题不能为空"));
    }
    let starts = crate::time::parse_iso(&input.starts_at)
        .ok_or_else(|| AppError::invalid("开始时间格式无效"))?;
    if let Some(end) = &input.ends_at {
        let end = crate::time::parse_iso(end).ok_or_else(|| AppError::invalid("结束时间格式无效"))?;
        if end < starts {
            return Err(AppError::invalid("结束时间不能早于开始时间"));
        }
    }
    let kind = input.kind.unwrap_or_else(|| "personal".into());
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO calendar_events (id, course_id, external_id, ics_uid, title, description,
                                      location, starts_at, ends_at, all_day, kind, source,
                                      assignable_id, created_at, updated_at)
         VALUES (?1, ?2, NULL, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'manual', NULL, ?10, ?10)",
        params![
            id,
            input.course_id,
            input.title.trim(),
            input.description,
            input.location,
            crate::time::to_iso(starts),
            input.ends_at.as_deref().and_then(crate::time::parse_iso).map(crate::time::to_iso),
            crate::storage::db::i(input.all_day),
            kind,
            now
        ],
    )?;
    conn.query_row(&format!("{EVENT_SELECT} WHERE e.id = ?1"), [&id], map_event)
        .map_err(AppError::from)
}

pub fn delete_event(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM calendar_events WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AppError::not_found(format!("日程不存在：{id}")));
    }
    Ok(())
}

pub struct IcsEventInput<'a> {
    pub uid: &'a str,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    pub starts_at: &'a str,
    pub ends_at: Option<&'a str>,
    pub all_day: bool,
}

/// Import one ICS VEVENT. Repeated imports of the same feed must not duplicate
/// rows, so `(ics_uid)` is unique and updates in place.
pub fn upsert_ics_event(conn: &Connection, input: &IcsEventInput<'_>) -> AppResult<bool> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM calendar_events WHERE ics_uid = ?1",
            [input.uid],
            |r| r.get(0),
        )
        .optional()?;
    let now = now_iso();
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE calendar_events SET title = ?2, description = ?3, location = ?4,
                        starts_at = ?5, ends_at = ?6, all_day = ?7, source = 'import', updated_at = ?8
                 WHERE id = ?1",
                params![
                    id,
                    input.title,
                    input.description,
                    input.location,
                    input.starts_at,
                    input.ends_at,
                    crate::storage::db::i(input.all_day),
                    now
                ],
            )?;
            Ok(false)
        }
        None => {
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO calendar_events (id, course_id, external_id, ics_uid, title, description,
                                              location, starts_at, ends_at, all_day, kind, source,
                                              assignable_id, created_at, updated_at)
                 VALUES (?1, NULL, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'event', 'import', NULL, ?9, ?9)",
                params![
                    id,
                    input.uid,
                    input.title,
                    input.description,
                    input.location,
                    input.starts_at,
                    input.ends_at,
                    crate::storage::db::i(input.all_day),
                    now
                ],
            )?;
            Ok(true)
        }
    }
}

pub struct CanvasEventInput<'a> {
    pub external_id: &'a str,
    pub course_id: Option<&'a str>,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub location: Option<&'a str>,
    pub starts_at: &'a str,
    pub ends_at: Option<&'a str>,
    pub all_day: bool,
    pub kind: &'a str,
}

pub fn upsert_canvas_event(conn: &Connection, input: &CanvasEventInput<'_>) -> AppResult<bool> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM calendar_events WHERE external_id = ?1 AND source = 'canvas'",
            [input.external_id],
            |r| r.get(0),
        )
        .optional()?;
    let now = now_iso();
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE calendar_events SET course_id = ?2, title = ?3, description = ?4,
                        location = ?5, starts_at = ?6, ends_at = ?7, all_day = ?8, kind = ?9,
                        updated_at = ?10
                 WHERE id = ?1",
                params![
                    id,
                    input.course_id,
                    input.title,
                    input.description,
                    input.location,
                    input.starts_at,
                    input.ends_at,
                    crate::storage::db::i(input.all_day),
                    input.kind,
                    now
                ],
            )?;
            Ok(false)
        }
        None => {
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO calendar_events (id, course_id, external_id, ics_uid, title, description,
                                              location, starts_at, ends_at, all_day, kind, source,
                                              assignable_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'canvas', NULL, ?11, ?11)",
                params![
                    id,
                    input.course_id,
                    input.external_id,
                    input.title,
                    input.description,
                    input.location,
                    input.starts_at,
                    input.ends_at,
                    crate::storage::db::i(input.all_day),
                    input.kind,
                    now
                ],
            )?;
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------------------
// Grades
// ---------------------------------------------------------------------------

const GRADE_SELECT: &str = r#"
SELECT g.id, g.course_id, c.name, g.external_id, g.assignment_name, g.score,
       g.points_possible, g.grade, g.feedback, g.posted_at, g.source, g.created_at, g.updated_at
FROM grade_records g LEFT JOIN courses c ON c.id = g.course_id
"#;

fn map_grade(row: &Row<'_>) -> rusqlite::Result<GradeRecord> {
    Ok(GradeRecord {
        id: row.get(0)?,
        course_id: row.get(1)?,
        course_name: row.get(2)?,
        external_id: row.get(3)?,
        assignment_name: row.get(4)?,
        score: row.get(5)?,
        points_possible: row.get(6)?,
        grade: row.get(7)?,
        feedback: row.get(8)?,
        posted_at: row.get(9)?,
        source: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

pub fn list_grades(conn: &Connection, course_id: Option<&str>) -> AppResult<Vec<GradeRecord>> {
    let sql = format!(
        "{GRADE_SELECT} WHERE (?1 IS NULL OR g.course_id = ?1) ORDER BY g.posted_at DESC, g.assignment_name ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![course_id], map_grade)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub struct GradeUpsert<'a> {
    pub course_id: &'a str,
    pub external_id: &'a str,
    pub assignment_name: &'a str,
    pub score: Option<f64>,
    pub points_possible: Option<f64>,
    pub grade: Option<&'a str>,
    pub feedback: Option<&'a str>,
    pub posted_at: Option<&'a str>,
}

pub fn upsert_canvas_grade(conn: &Connection, seed: &GradeUpsert<'_>) -> AppResult<bool> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM grade_records WHERE course_id = ?1 AND external_id = ?2",
            params![seed.course_id, seed.external_id],
            |r| r.get(0),
        )
        .optional()?;
    let now = now_iso();
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE grade_records SET assignment_name = ?2, score = ?3, points_possible = ?4,
                        grade = ?5, feedback = ?6, posted_at = ?7, source = 'canvas', updated_at = ?8
                 WHERE id = ?1",
                params![
                    id,
                    seed.assignment_name,
                    seed.score,
                    seed.points_possible,
                    seed.grade,
                    seed.feedback,
                    seed.posted_at,
                    now
                ],
            )?;
            Ok(false)
        }
        None => {
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO grade_records (id, course_id, external_id, assignment_name, score,
                                            points_possible, grade, feedback, posted_at, source,
                                            created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'canvas', ?10, ?10)",
                params![
                    id,
                    seed.course_id,
                    seed.external_id,
                    seed.assignment_name,
                    seed.score,
                    seed.points_possible,
                    seed.grade,
                    seed.feedback,
                    seed.posted_at,
                    now
                ],
            )?;
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------------------
// Discussions
// ---------------------------------------------------------------------------

const TOPIC_SELECT: &str = "SELECT id, course_id, external_id, title, body, author, posted_at, reply_count, html_url, created_at, updated_at FROM discussion_topics";

fn map_topic(row: &Row<'_>) -> rusqlite::Result<DiscussionTopic> {
    Ok(DiscussionTopic {
        id: row.get(0)?,
        course_id: row.get(1)?,
        external_id: row.get(2)?,
        title: row.get(3)?,
        body: row.get(4)?,
        author: row.get(5)?,
        posted_at: row.get(6)?,
        reply_count: row.get(7)?,
        html_url: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

pub fn list_discussions(conn: &Connection, course_id: &str) -> AppResult<Vec<DiscussionTopic>> {
    let sql = format!("{TOPIC_SELECT} WHERE course_id = ?1 ORDER BY posted_at DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([course_id], map_topic)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub struct TopicUpsert<'a> {
    pub course_id: &'a str,
    pub external_id: &'a str,
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub author: Option<&'a str>,
    pub posted_at: Option<&'a str>,
    pub reply_count: i64,
    pub html_url: Option<&'a str>,
}

pub fn upsert_canvas_topic(conn: &Connection, seed: &TopicUpsert<'_>) -> AppResult<String> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM discussion_topics WHERE course_id = ?1 AND external_id = ?2",
            params![seed.course_id, seed.external_id],
            |r| r.get(0),
        )
        .optional()?;
    let now = now_iso();
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE discussion_topics SET title = ?2, body = ?3, author = ?4, posted_at = ?5,
                        reply_count = ?6, html_url = ?7, updated_at = ?8 WHERE id = ?1",
                params![
                    id,
                    seed.title,
                    seed.body,
                    seed.author,
                    seed.posted_at,
                    seed.reply_count,
                    seed.html_url,
                    now
                ],
            )?;
            Ok(id)
        }
        None => {
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO discussion_topics (id, course_id, external_id, title, body, author,
                                                posted_at, reply_count, html_url, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    id,
                    seed.course_id,
                    seed.external_id,
                    seed.title,
                    seed.body,
                    seed.author,
                    seed.posted_at,
                    seed.reply_count,
                    seed.html_url,
                    now
                ],
            )?;
            Ok(id)
        }
    }
}

pub fn list_discussion_entries(
    conn: &Connection,
    topic_id: &str,
) -> AppResult<Vec<DiscussionEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, topic_id, external_id, parent_id, author, body, posted_at, created_at, updated_at
         FROM discussion_entries WHERE topic_id = ?1 ORDER BY posted_at ASC",
    )?;
    let rows = stmt.query_map([topic_id], |row| {
        Ok(DiscussionEntry {
            id: row.get(0)?,
            topic_id: row.get(1)?,
            external_id: row.get(2)?,
            parent_id: row.get(3)?,
            author: row.get(4)?,
            body: row.get(5)?,
            posted_at: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn upsert_canvas_entry(
    conn: &Connection,
    topic_id: &str,
    external_id: &str,
    parent_id: Option<&str>,
    author: Option<&str>,
    body: Option<&str>,
    posted_at: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO discussion_entries (id, topic_id, external_id, parent_id, author, body, posted_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
         ON CONFLICT(topic_id, external_id) DO UPDATE SET
            author = excluded.author, body = excluded.body, posted_at = excluded.posted_at,
            parent_id = excluded.parent_id, updated_at = excluded.updated_at",
        params![
            Uuid::new_v4().to_string(),
            topic_id,
            external_id,
            parent_id,
            author,
            body,
            posted_at,
            now_iso()
        ],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Course pages
// ---------------------------------------------------------------------------

pub fn list_pages(conn: &Connection, course_id: &str) -> AppResult<Vec<CoursePage>> {
    let mut stmt = conn.prepare(
        "SELECT id, course_id, external_id, title, body, url, updated_at_remote, html_url, created_at, updated_at
         FROM course_pages WHERE course_id = ?1 ORDER BY title ASC",
    )?;
    let rows = stmt.query_map([course_id], |row| {
        Ok(CoursePage {
            id: row.get(0)?,
            course_id: row.get(1)?,
            external_id: row.get(2)?,
            title: row.get(3)?,
            body: row.get(4)?,
            url: row.get(5)?,
            updated_at_remote: row.get(6)?,
            html_url: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub struct PageUpsert<'a> {
    pub course_id: &'a str,
    pub external_id: &'a str,
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub url: Option<&'a str>,
    pub updated_at_remote: Option<&'a str>,
    pub html_url: Option<&'a str>,
}

pub fn upsert_canvas_page(conn: &Connection, seed: &PageUpsert<'_>) -> AppResult<bool> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM course_pages WHERE course_id = ?1 AND external_id = ?2",
            params![seed.course_id, seed.external_id],
            |r| r.get(0),
        )
        .optional()?;
    let now = now_iso();
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE course_pages SET title = ?2, body = ?3, url = ?4, updated_at_remote = ?5,
                        html_url = ?6, updated_at = ?7 WHERE id = ?1",
                params![
                    id,
                    seed.title,
                    seed.body,
                    seed.url,
                    seed.updated_at_remote,
                    seed.html_url,
                    now
                ],
            )?;
            Ok(false)
        }
        None => {
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO course_pages (id, course_id, external_id, title, body, url,
                                           updated_at_remote, html_url, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![
                    id,
                    seed.course_id,
                    seed.external_id,
                    seed.title,
                    seed.body,
                    seed.url,
                    seed.updated_at_remote,
                    seed.html_url,
                    now
                ],
            )?;
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------------------
// Deadlines
// ---------------------------------------------------------------------------

/// Upcoming deadlines from assignments (Canvas or manual) and dated study tasks.
pub fn upcoming_deadlines(
    conn: &Connection,
    horizon_days: i64,
    limit: i64,
) -> AppResult<Vec<UpcomingDeadline>> {
    let from = crate::time::to_iso(chrono::Utc::now() - chrono::Duration::days(1));
    let to = crate::time::to_iso(chrono::Utc::now() + chrono::Duration::days(horizon_days));
    let mut stmt = conn.prepare(
        "SELECT a.id, 'assignment', a.title, a.course_id, c.name, a.due_at, a.source
           FROM assignments a LEFT JOIN courses c ON c.id = a.course_id
          WHERE a.due_at IS NOT NULL AND a.due_at >= ?1 AND a.due_at <= ?2
         UNION ALL
         SELECT t.id, 'task', t.title, t.course_id, c.name, t.deadline, 'manual'
           FROM study_tasks t LEFT JOIN courses c ON c.id = t.course_id
          WHERE t.deadline IS NOT NULL AND t.deadline >= ?1 AND t.deadline <= ?2
            AND t.status IN ('todo', 'in_progress')
         UNION ALL
         SELECT e.id, 'event', e.title, e.course_id, c.name, e.starts_at, e.source
           FROM calendar_events e LEFT JOIN courses c ON c.id = e.course_id
          WHERE e.starts_at >= ?1 AND e.starts_at <= ?2 AND e.kind IN ('exam', 'assignment_due')
         ORDER BY 6 ASC LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![from, to, limit], |row| {
        Ok(UpcomingDeadline {
            id: row.get(0)?,
            kind: row.get(1)?,
            title: row.get(2)?,
            course_id: row.get(3)?,
            course_name: row.get(4)?,
            due_at: row.get(5)?,
            source: row.get(6)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::CreateCourseInput;
    use crate::storage::{course_repo, Db};

    fn setup_course(db: &Db) -> String {
        db.with(|c| {
            let course = course_repo::create(
                c,
                CreateCourseInput {
                    name: "数据结构".into(),
                    code: Some("COMP 2012".into()),
                    term: Some("2026 Fall".into()),
                    instructor: None,
                    description: None,
                    color: None,
                },
                "manual",
            )?;
            Ok(course.id)
        })
        .unwrap()
    }

    #[test]
    fn ics_import_deduplicates_by_uid() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let event = IcsEventInput {
                uid: "exam-1@hkust-gz",
                title: "期中考试".into(),
                description: None,
                location: Some("Room 101".into()),
                starts_at: "2026-11-01T02:00:00Z",
                ends_at: Some("2026-11-01T04:00:00Z"),
                all_day: false,
            };
            assert!(upsert_ics_event(c, &event)?, "first import inserts");
            assert!(!upsert_ics_event(c, &event)?, "second import updates");

            let moved = IcsEventInput {
                starts_at: "2026-11-02T02:00:00Z",
                ..event
            };
            assert!(!upsert_ics_event(c, &moved)?);
            let events = list_events(c, None, None)?;
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].starts_at, "2026-11-02T02:00:00Z");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn grades_keep_source_provenance() {
        let db = Db::open_in_memory().unwrap();
        let course_id = setup_course(&db);
        db.with(|c| {
            upsert_canvas_grade(
                c,
                &GradeUpsert {
                    course_id: &course_id,
                    external_id: "g1",
                    assignment_name: "作业 1",
                    score: Some(88.0),
                    points_possible: Some(100.0),
                    grade: Some("88"),
                    feedback: None,
                    posted_at: None,
                },
            )?;
            let grades = list_grades(c, Some(&course_id))?;
            assert_eq!(grades.len(), 1);
            assert_eq!(grades[0].source, "canvas");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn assignments_reject_bad_due_dates() {
        let db = Db::open_in_memory().unwrap();
        let course_id = setup_course(&db);
        db.with(|c| {
            let err = create_assignment(
                c,
                CreateAssignmentInput {
                    course_id: course_id.clone(),
                    title: "Problem set 1".into(),
                    description: None,
                    due_at: Some("next friday".into()),
                    points_possible: Some(50.0),
                },
            )
            .unwrap_err();
            assert_eq!(err.code(), "invalid_input");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn upcoming_deadlines_merges_sources() {
        let db = Db::open_in_memory().unwrap();
        let course_id = setup_course(&db);
        db.with(|c| {
            let due = crate::time::to_iso(chrono::Utc::now() + chrono::Duration::days(3));
            create_assignment(
                c,
                CreateAssignmentInput {
                    course_id: course_id.clone(),
                    title: "Lab 2".into(),
                    description: None,
                    due_at: Some(due.clone()),
                    points_possible: None,
                },
            )?;
            create_event(
                c,
                CreateEventInput {
                    course_id: Some(course_id.clone()),
                    title: "期末考试".into(),
                    description: None,
                    location: None,
                    starts_at: due,
                    ends_at: None,
                    all_day: false,
                    kind: Some("exam".into()),
                },
            )?;
            let deadlines = upcoming_deadlines(c, 14, 20)?;
            assert_eq!(deadlines.len(), 2);
            assert!(deadlines.iter().any(|d| d.kind == "assignment"));
            assert!(deadlines.iter().any(|d| d.kind == "event"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn assignment_tri_state_due_patch() {
        let db = Db::open_in_memory().unwrap();
        let course_id = setup_course(&db);
        db.with(|c| {
            let a = create_assignment(
                c,
                CreateAssignmentInput {
                    course_id: course_id.clone(),
                    title: "Quiz".into(),
                    description: None,
                    due_at: Some("2026-12-01T00:00:00Z".into()),
                    points_possible: None,
                },
            )?;
            // Absent -> unchanged.
            let same = update_assignment(
                c,
                &a.id,
                UpdateAssignmentInput {
                    title: None,
                    description: None,
                    due_at: None,
                    points_possible: None,
                    submission_status: None,
                },
            )?;
            assert_eq!(same.due_at.as_deref(), Some("2026-12-01T00:00:00Z"));
            // Explicit null -> cleared.
            let cleared = update_assignment(
                c,
                &a.id,
                UpdateAssignmentInput {
                    title: None,
                    description: None,
                    due_at: Some(None),
                    points_possible: None,
                    submission_status: None,
                },
            )?;
            assert!(cleared.due_at.is_none());
            Ok(())
        })
        .unwrap();
    }
}
