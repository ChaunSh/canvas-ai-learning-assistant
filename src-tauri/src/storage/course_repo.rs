//! Courses and course modules.

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::domain::{Course, CourseModule, CreateCourseInput, CreateModuleInput, UpdateCourseInput};
use crate::error::{AppError, AppResult};
use crate::time::{now_iso, parse_iso};

const COURSE_SELECT: &str = r#"
SELECT
    c.id, c.external_id, c.code, c.name, c.term, c.instructor, c.description,
    c.source, c.sync_status, c.archived, c.color, c.position, c.last_synced_at,
    c.created_at, c.updated_at,
    (SELECT COUNT(*) FROM materials m WHERE m.course_id = c.id) AS material_count,
    (SELECT COUNT(*) FROM study_tasks t
        WHERE t.course_id = c.id AND t.status IN ('todo', 'in_progress')) AS open_task_count,
    (SELECT COUNT(*) FROM assignments a
        WHERE a.course_id = c.id AND a.due_at IS NOT NULL
          AND a.due_at >= ?1 AND a.due_at <= ?2) AS due_soon_count,
    COALESCE((SELECT AVG(p.completion) FROM study_progress p WHERE p.course_id = c.id), 0.0) AS progress
FROM courses c
"#;

fn map_course(row: &Row<'_>) -> rusqlite::Result<Course> {
    Ok(Course {
        id: row.get(0)?,
        external_id: row.get(1)?,
        code: row.get(2)?,
        name: row.get(3)?,
        term: row.get(4)?,
        instructor: row.get(5)?,
        description: row.get(6)?,
        source: row.get(7)?,
        sync_status: row.get(8)?,
        archived: crate::storage::db::b(row.get::<_, i64>(9)?),
        color: row.get(10)?,
        position: row.get(11)?,
        last_synced_at: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        material_count: row.get(15)?,
        open_task_count: row.get(16)?,
        due_soon_count: row.get(17)?,
        progress: row.get::<_, f64>(18)?.clamp(0.0, 1.0),
    })
}

/// Window used to count "due soon" assignments: now .. now + 14 days.
fn due_window() -> (String, String) {
    let now = chrono::Utc::now();
    (
        crate::time::to_iso(now - chrono::Duration::hours(24)),
        crate::time::to_iso(now + chrono::Duration::days(14)),
    )
}

pub fn list(conn: &Connection, include_archived: bool) -> AppResult<Vec<Course>> {
    let (from, to) = due_window();
    let sql = format!(
        "{COURSE_SELECT} WHERE (?3 = 1 OR c.archived = 0) ORDER BY c.archived ASC, c.position ASC, c.name ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![from, to, if include_archived { 1 } else { 0 }], |r| {
        map_course(r)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Course> {
    let (from, to) = due_window();
    let sql = format!("{COURSE_SELECT} WHERE c.id = ?3");
    conn.query_row(&sql, params![from, to, id], |r| map_course(r))
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("课程不存在：{id}")))
}

pub fn create(conn: &Connection, input: CreateCourseInput, source: &str) -> AppResult<Course> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::invalid("课程名称不能为空"));
    }
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    let position: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM courses",
        [],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO courses (id, external_id, code, name, term, instructor, description, source,
                              sync_status, archived, color, position, last_synced_at,
                              created_at, updated_at)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, 'local', 0, ?8, ?9, NULL, ?10, ?10)",
        params![
            id,
            normalize_opt(&input.code),
            name,
            normalize_opt(&input.term),
            normalize_opt(&input.instructor),
            normalize_opt(&input.description),
            source,
            normalize_opt(&input.color),
            position,
            now
        ],
    )?;
    get(conn, &id)
}

pub fn update(conn: &Connection, id: &str, patch: UpdateCourseInput) -> AppResult<Course> {
    // Ensure it exists and is editable.
    let current = get(conn, id)?;
    if current.source == "canvas" && patch.name.is_some() {
        // Renaming a Canvas course locally would desynchronise the mapping on the
        // next sync; allow it but the sync keeps the remote name authoritative.
        tracing::debug!(course = %id, "renaming canvas-sourced course locally");
    }
    if let Some(name) = &patch.name {
        if name.trim().is_empty() {
            return Err(AppError::invalid("课程名称不能为空"));
        }
    }
    conn.execute(
        "UPDATE courses SET
            name        = COALESCE(?2, name),
            code        = COALESCE(?3, code),
            term        = COALESCE(?4, term),
            instructor  = COALESCE(?5, instructor),
            description = COALESCE(?6, description),
            color       = COALESCE(?7, color),
            archived    = COALESCE(?8, archived),
            position    = COALESCE(?9, position),
            sync_status = CASE WHEN source = 'canvas' THEN sync_status ELSE 'local' END,
            updated_at  = ?10
         WHERE id = ?1",
        params![
            id,
            patch.name.map(|v| v.trim().to_string()),
            patch.code,
            patch.term,
            patch.instructor,
            patch.description,
            patch.color,
            patch.archived.map(crate::storage::db::i),
            patch.position,
            now_iso()
        ],
    )?;
    get(conn, id)
}

/// Delete a course. Cascades to modules, assignments, materials links, tasks etc.
pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM courses WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AppError::not_found(format!("课程不存在：{id}")));
    }
    Ok(())
}

/// Data required to persist a course discovered through Canvas.
pub struct CourseUpsert {
    pub external_id: String,
    pub code: Option<String>,
    pub name: String,
    pub term: Option<String>,
    pub instructor: Option<String>,
    pub description: Option<String>,
}

/// Insert or update a Canvas-sourced course, returning its local id and whether
/// it was newly created.
pub fn upsert_canvas(conn: &Connection, seed: &CourseUpsert) -> AppResult<(String, bool)> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM courses WHERE source = 'canvas' AND external_id = ?1",
            [&seed.external_id],
            |r| r.get(0),
        )
        .optional()?;

    let now = now_iso();
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE courses SET code = ?2, name = ?3, term = ?4, instructor = ?5,
                        description = ?6, sync_status = 'synced', last_synced_at = ?7, updated_at = ?7
                 WHERE id = ?1",
                params![
                    id,
                    seed.code,
                    seed.name,
                    seed.term,
                    seed.instructor,
                    seed.description,
                    now
                ],
            )?;
            Ok((id, false))
        }
        None => {
            let id = Uuid::new_v4().to_string();
            let position: i64 = conn.query_row(
                "SELECT COALESCE(MAX(position), 0) + 1 FROM courses",
                [],
                |r| r.get(0),
            )?;
            conn.execute(
                "INSERT INTO courses (id, external_id, code, name, term, instructor, description,
                                      source, sync_status, archived, color, position,
                                      last_synced_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'canvas', 'synced', 0, NULL, ?8, ?9, ?9, ?9)",
                params![
                    id,
                    seed.external_id,
                    seed.code,
                    seed.name,
                    seed.term,
                    seed.instructor,
                    seed.description,
                    position,
                    now
                ],
            )?;
            Ok((id, true))
        }
    }
}

/// Local id for a Canvas course id, if already synced.
pub fn find_by_external(conn: &Connection, external_id: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM courses WHERE source = 'canvas' AND external_id = ?1",
            [external_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?)
}

pub fn mark_synced(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE courses SET sync_status = 'synced', last_synced_at = ?2, updated_at = ?2 WHERE id = ?1",
        params![id, now_iso()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Modules
// ---------------------------------------------------------------------------

const MODULE_SELECT: &str = "SELECT id, course_id, external_id, name, position, unlock_at, item_count, created_at, updated_at FROM course_modules";

fn map_module(row: &Row<'_>) -> rusqlite::Result<CourseModule> {
    Ok(CourseModule {
        id: row.get(0)?,
        course_id: row.get(1)?,
        external_id: row.get(2)?,
        name: row.get(3)?,
        position: row.get(4)?,
        unlock_at: row.get(5)?,
        item_count: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub fn list_modules(conn: &Connection, course_id: &str) -> AppResult<Vec<CourseModule>> {
    let sql = format!("{MODULE_SELECT} WHERE course_id = ?1 ORDER BY position ASC, name ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([course_id], map_module)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn create_module(conn: &Connection, input: CreateModuleInput) -> AppResult<CourseModule> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::invalid("章节名称不能为空"));
    }
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    let position: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM course_modules WHERE course_id = ?1",
        [&input.course_id],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO course_modules (id, course_id, external_id, name, position, unlock_at, item_count, created_at, updated_at)
         VALUES (?1, ?2, NULL, ?3, ?4, ?5, 0, ?6, ?6)",
        params![id, input.course_id, name, position, input.unlock_at, now],
    )?;
    conn.query_row(&format!("{MODULE_SELECT} WHERE id = ?1"), [&id], map_module)
        .map_err(AppError::from)
}

pub fn delete_module(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM course_modules WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AppError::not_found(format!("章节不存在：{id}")));
    }
    Ok(())
}

pub fn upsert_canvas_module(
    conn: &Connection,
    course_id: &str,
    external_id: &str,
    name: &str,
    position: i64,
    unlock_at: Option<&str>,
) -> AppResult<String> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM course_modules WHERE course_id = ?1 AND external_id = ?2",
            params![course_id, external_id],
            |r| r.get(0),
        )
        .optional()?;
    let now = now_iso();
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE course_modules SET name = ?2, position = ?3, unlock_at = ?4, updated_at = ?5 WHERE id = ?1",
                params![id, name, position, unlock_at, now],
            )?;
            Ok(id)
        }
        None => {
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO course_modules (id, course_id, external_id, name, position, unlock_at, item_count, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)",
                params![id, course_id, external_id, name, position, unlock_at, now],
            )?;
            Ok(id)
        }
    }
}

fn normalize_opt(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Recompute `course_modules.item_count` from linked materials.
pub fn refresh_module_counts(conn: &Connection, course_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE course_modules SET item_count = (
            SELECT COUNT(*) FROM materials m WHERE m.module_id = course_modules.id
         ) WHERE course_id = ?1",
        [course_id],
    )?;
    Ok(())
}

/// Normalised term label used for grouping on the courses page.
pub fn term_of(course: &Course) -> String {
    course
        .term
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "未分学期".to_string())
}

/// True when a course's cached data is older than `hours`.
pub fn is_stale(course: &Course, hours: i64) -> bool {
    match course.last_synced_at.as_deref().and_then(parse_iso) {
        Some(at) => (chrono::Utc::now() - at).num_hours() >= hours,
        None => course.source == "canvas",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    fn course_input(name: &str) -> CreateCourseInput {
        CreateCourseInput {
            name: name.into(),
            code: Some("COMP 1022".into()),
            term: Some("2026 Fall".into()),
            instructor: Some("Dr. Test".into()),
            description: None,
            color: None,
        }
    }

    #[test]
    fn create_list_update_delete() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let created = create(c, course_input("线性代数"), "manual")?;
            assert_eq!(created.name, "线性代数");
            assert_eq!(created.source, "manual");
            assert!(created.external_id.is_none());

            let listed = list(c, false)?;
            assert_eq!(listed.len(), 1);

            let updated = update(
                c,
                &created.id,
                UpdateCourseInput {
                    name: Some("线性代数 II".into()),
                    archived: Some(true),
                    ..Default::default()
                },
            )?;
            assert_eq!(updated.name, "线性代数 II");
            assert!(updated.archived);
            assert!(list(c, false)?.is_empty());
            assert_eq!(list(c, true)?.len(), 1);

            delete(c, &created.id)?;
            assert!(list(c, true)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn rejects_blank_names() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let err = create(c, course_input("   "), "manual").unwrap_err();
            assert_eq!(err.code(), "invalid_input");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn canvas_upsert_is_idempotent() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let seed = CourseUpsert {
                external_id: "123456789012345678".into(), // 64-bit-ish id as string
                code: Some("MATH 101".into()),
                name: "Calculus".into(),
                term: None,
                instructor: None,
                description: None,
            };
            let (id1, created1) = upsert_canvas(c, &seed)?;
            let (id2, created2) = upsert_canvas(c, &seed)?;
            assert!(created1);
            assert!(!created2);
            assert_eq!(id1, id2);
            assert_eq!(find_by_external(c, "123456789012345678")?, Some(id1));
            assert_eq!(list(c, false)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn modules_are_ordered_and_counted() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let course = create(c, course_input("离散数学"), "manual")?;
            let m1 = create_module(
                c,
                CreateModuleInput {
                    course_id: course.id.clone(),
                    name: "第一章".into(),
                    unlock_at: None,
                },
            )?;
            let m2 = create_module(
                c,
                CreateModuleInput {
                    course_id: course.id.clone(),
                    name: "第二章".into(),
                    unlock_at: None,
                },
            )?;
            let mods = list_modules(c, &course.id)?;
            assert_eq!(mods.len(), 2);
            assert_eq!(mods[0].id, m1.id);
            assert_eq!(mods[1].id, m2.id);
            Ok(())
        })
        .unwrap();
    }
}
