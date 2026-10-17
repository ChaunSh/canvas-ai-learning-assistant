//! Study plans, tasks, preferences and study sessions.

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::domain::{
    StudyPlan, StudyPreferences, StudySessionRecord, StudyTask, TaskPatch, UpdatePreferencesInput,
};
use crate::error::{AppError, AppResult};
use crate::storage::db::{b, i};
use crate::time::{now_iso, parse_hhmm};

const TASK_SELECT: &str = r#"
SELECT t.id, t.plan_id, t.course_id, c.name, t.material_id, m.title, t.assignment_id, t.title,
       t.description, t.estimated_minutes, t.scheduled_start, t.scheduled_end, t.deadline,
       t.priority, t.status, t.locked, t.generated_by, t.source_references, t.completed_at,
       t.created_at, t.updated_at
FROM study_tasks t
LEFT JOIN courses c ON c.id = t.course_id
LEFT JOIN materials m ON m.id = t.material_id
"#;

fn map_task(row: &Row<'_>) -> rusqlite::Result<StudyTask> {
    let refs: String = row.get(17)?;
    Ok(StudyTask {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        course_id: row.get(2)?,
        course_name: row.get(3)?,
        material_id: row.get(4)?,
        material_title: row.get(5)?,
        assignment_id: row.get(6)?,
        title: row.get(7)?,
        description: row.get(8)?,
        estimated_minutes: row.get(9)?,
        scheduled_start: row.get(10)?,
        scheduled_end: row.get(11)?,
        deadline: row.get(12)?,
        priority: row.get(13)?,
        status: row.get(14)?,
        locked: b(row.get::<_, i64>(15)?),
        generated_by: row.get(16)?,
        source_references: serde_json::from_str(&refs).unwrap_or_default(),
        completed_at: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

pub fn get_preferences(conn: &Connection) -> AppResult<StudyPreferences> {
    let existing = conn
        .query_row(
            "SELECT daily_minutes, session_minutes, break_minutes, preferred_start,
                    preferred_end, study_days, timezone, avoid_weekends, updated_at
             FROM study_preferences LIMIT 1",
            [],
            |row| {
                let days: String = row.get(5)?;
                Ok(StudyPreferences {
                    daily_minutes: row.get(0)?,
                    session_minutes: row.get(1)?,
                    break_minutes: row.get(2)?,
                    preferred_start: row.get(3)?,
                    preferred_end: row.get(4)?,
                    study_days: serde_json::from_str(&days).unwrap_or_else(|_| vec![1, 2, 3, 4, 5, 6, 7]),
                    timezone: row.get(6)?,
                    avoid_weekends: b(row.get::<_, i64>(7)?),
                    updated_at: row.get(8)?,
                })
            },
        )
        .optional()?;

    match existing {
        Some(prefs) => Ok(prefs),
        None => {
            let id = Uuid::new_v4().to_string();
            let now = now_iso();
            conn.execute(
                "INSERT INTO study_preferences (id, updated_at) VALUES (?1, ?2)",
                params![id, now],
            )?;
            get_preferences(conn)
        }
    }
}

pub fn update_preferences(
    conn: &Connection,
    patch: UpdatePreferencesInput,
) -> AppResult<StudyPreferences> {
    let current = get_preferences(conn)?;

    let daily = patch.daily_minutes.unwrap_or(current.daily_minutes);
    if !(15..=960).contains(&daily) {
        return Err(AppError::invalid("每日学习时长需在 15 到 960 分钟之间"));
    }
    let session = patch.session_minutes.unwrap_or(current.session_minutes);
    if !(10..=240).contains(&session) {
        return Err(AppError::invalid("单次学习时长需在 10 到 240 分钟之间"));
    }
    let pause = patch.break_minutes.unwrap_or(current.break_minutes);
    if !(0..=120).contains(&pause) {
        return Err(AppError::invalid("休息时长需在 0 到 120 分钟之间"));
    }

    let start = patch.preferred_start.unwrap_or(current.preferred_start);
    let end = patch.preferred_end.unwrap_or(current.preferred_end);
    let start_min = parse_hhmm(&start).ok_or_else(|| AppError::invalid("开始时间格式应为 HH:MM"))?;
    let end_min = parse_hhmm(&end).ok_or_else(|| AppError::invalid("结束时间格式应为 HH:MM"))?;
    if end_min <= start_min {
        return Err(AppError::invalid("结束时间必须晚于开始时间"));
    }

    let mut days = patch.study_days.unwrap_or(current.study_days);
    days.retain(|d| (1..=7).contains(d));
    days.sort_unstable();
    days.dedup();
    if days.is_empty() {
        return Err(AppError::invalid("至少需要选择一个学习日"));
    }

    let timezone = patch.timezone.unwrap_or(current.timezone);
    if timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(AppError::invalid("时区名称无效（应为 IANA 时区，如 Asia/Shanghai）"));
    }

    conn.execute(
        "UPDATE study_preferences SET daily_minutes = ?1, session_minutes = ?2, break_minutes = ?3,
                preferred_start = ?4, preferred_end = ?5, study_days = ?6, timezone = ?7,
                avoid_weekends = ?8, updated_at = ?9",
        params![
            daily,
            session,
            pause,
            start,
            end,
            serde_json::to_string(&days)?,
            timezone,
            i(patch.avoid_weekends.unwrap_or(current.avoid_weekends)),
            now_iso()
        ],
    )?;
    get_preferences(conn)
}

// ---------------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------------

fn map_plan(row: &Row<'_>) -> rusqlite::Result<StudyPlan> {
    Ok(StudyPlan {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        start_date: row.get(3)?,
        end_date: row.get(4)?,
        status: row.get(5)?,
        notes: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        task_count: row.get(9)?,
        done_count: row.get(10)?,
    })
}

const PLAN_SELECT: &str = r#"
SELECT p.id, p.name, p.kind, p.start_date, p.end_date, p.status, p.notes, p.created_at, p.updated_at,
       (SELECT COUNT(*) FROM study_tasks t WHERE t.plan_id = p.id) AS task_count,
       (SELECT COUNT(*) FROM study_tasks t WHERE t.plan_id = p.id AND t.status = 'done') AS done_count
FROM study_plans p
"#;

pub fn create_plan(
    conn: &Connection,
    name: &str,
    kind: &str,
    start_date: &str,
    end_date: &str,
    notes: Option<&str>,
) -> AppResult<StudyPlan> {
    if name.trim().is_empty() {
        return Err(AppError::invalid("计划名称不能为空"));
    }
    if crate::time::parse_date(start_date).is_none() || crate::time::parse_date(end_date).is_none() {
        return Err(AppError::invalid("计划日期格式应为 YYYY-MM-DD"));
    }
    if end_date < start_date {
        return Err(AppError::invalid("结束日期不能早于开始日期"));
    }
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO study_plans (id, name, kind, start_date, end_date, status, notes, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?7)",
        params![id, name.trim(), kind, start_date, end_date, notes, now],
    )?;
    get_plan(conn, &id)
}

pub fn get_plan(conn: &Connection, id: &str) -> AppResult<StudyPlan> {
    let sql = format!("{PLAN_SELECT} WHERE p.id = ?1");
    conn.query_row(&sql, [id], map_plan)
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("学习计划不存在：{id}")))
}

pub fn list_plans(conn: &Connection) -> AppResult<Vec<StudyPlan>> {
    let sql = format!("{PLAN_SELECT} ORDER BY p.status ASC, p.start_date DESC LIMIT 100");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], map_plan)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn delete_plan(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM study_plans WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AppError::not_found(format!("学习计划不存在：{id}")));
    }
    Ok(())
}

pub fn set_plan_status(conn: &Connection, id: &str, status: &str) -> AppResult<()> {
    if !matches!(status, "draft" | "active" | "archived") {
        return Err(AppError::invalid("计划状态无效"));
    }
    conn.execute(
        "UPDATE study_plans SET status = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, status, now_iso()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

pub struct TaskSeed {
    pub plan_id: Option<String>,
    pub course_id: Option<String>,
    pub material_id: Option<String>,
    pub assignment_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub estimated_minutes: i64,
    pub scheduled_start: Option<String>,
    pub scheduled_end: Option<String>,
    pub deadline: Option<String>,
    pub priority: i64,
    pub generated_by: String,
    pub source_references: Vec<String>,
}

pub fn create_task(conn: &Connection, seed: &TaskSeed) -> AppResult<StudyTask> {
    if seed.title.trim().is_empty() {
        return Err(AppError::invalid("任务标题不能为空"));
    }
    if seed.estimated_minutes <= 0 {
        return Err(AppError::invalid("预计时长必须大于 0"));
    }
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO study_tasks (id, plan_id, course_id, material_id, assignment_id, title,
                                  description, estimated_minutes, scheduled_start, scheduled_end,
                                  deadline, priority, status, locked, generated_by,
                                  source_references, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'todo', 0, ?13, ?14, ?15, ?15)",
        params![
            id,
            seed.plan_id,
            seed.course_id,
            seed.material_id,
            seed.assignment_id,
            seed.title.trim(),
            seed.description,
            seed.estimated_minutes,
            seed.scheduled_start,
            seed.scheduled_end,
            seed.deadline,
            seed.priority.clamp(1, 5),
            seed.generated_by,
            serde_json::to_string(&seed.source_references)?,
            now
        ],
    )?;
    get_task(conn, &id)
}

pub fn create_tasks(conn: &Connection, seeds: &[TaskSeed]) -> AppResult<Vec<StudyTask>> {
    let mut out = Vec::with_capacity(seeds.len());
    for seed in seeds {
        out.push(create_task(conn, seed)?);
    }
    Ok(out)
}

pub fn get_task(conn: &Connection, id: &str) -> AppResult<StudyTask> {
    let sql = format!("{TASK_SELECT} WHERE t.id = ?1");
    conn.query_row(&sql, [id], map_task)
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("学习任务不存在：{id}")))
}

pub fn list_tasks(
    conn: &Connection,
    plan_id: Option<&str>,
    course_id: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    include_done: bool,
) -> AppResult<Vec<StudyTask>> {
    let sql = format!(
        "{TASK_SELECT}
         WHERE (?1 IS NULL OR t.plan_id = ?1)
           AND (?2 IS NULL OR t.course_id = ?2)
           AND (?3 IS NULL OR (t.scheduled_start IS NOT NULL AND t.scheduled_start >= ?3))
           AND (?4 IS NULL OR (t.scheduled_start IS NOT NULL AND t.scheduled_start <= ?4))
           AND (?5 = 1 OR t.status IN ('todo','in_progress'))
         ORDER BY (t.scheduled_start IS NULL), t.scheduled_start ASC, t.priority ASC, t.title ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![plan_id, course_id, from, to, if include_done { 1 } else { 0 }],
        map_task,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn update_task(conn: &Connection, id: &str, patch: &TaskPatch) -> AppResult<StudyTask> {
    let current = get_task(conn, id)?;

    if let Some(minutes) = patch.estimated_minutes {
        if minutes <= 0 {
            return Err(AppError::invalid("预计时长必须大于 0"));
        }
    }
    if let Some(status) = &patch.status {
        if !matches!(status.as_str(), "todo" | "in_progress" | "done" | "skipped") {
            return Err(AppError::invalid("任务状态无效"));
        }
    }
    if let Some(priority) = patch.priority {
        if !(1..=5).contains(&priority) {
            return Err(AppError::invalid("优先级需在 1 到 5 之间"));
        }
    }

    let scheduled_start = match &patch.scheduled_start {
        Some(value) => value.clone(),
        None => current.scheduled_start.clone(),
    };
    let scheduled_end = match &patch.scheduled_end {
        Some(value) => value.clone(),
        None => current.scheduled_end.clone(),
    };
    if let (Some(start), Some(end)) = (&scheduled_start, &scheduled_end) {
        if let (Some(s), Some(e)) = (crate::time::parse_iso(start), crate::time::parse_iso(end)) {
            if e < s {
                return Err(AppError::invalid("结束时间不能早于开始时间"));
            }
        } else {
            return Err(AppError::invalid("时间格式无效"));
        }
    }

    // Completing a task records the completion instant; reopening clears it.
    let status = patch.status.clone().unwrap_or(current.status.clone());
    let completed_at = match status.as_str() {
        "done" => Some(current.completed_at.clone().unwrap_or_else(now_iso)),
        _ => None,
    };

    conn.execute(
        "UPDATE study_tasks SET
            title = COALESCE(?2, title),
            description = COALESCE(?3, description),
            estimated_minutes = COALESCE(?4, estimated_minutes),
            scheduled_start = CASE WHEN ?5 = 1 THEN ?6 ELSE scheduled_start END,
            scheduled_end = CASE WHEN ?7 = 1 THEN ?8 ELSE scheduled_end END,
            deadline = CASE WHEN ?9 = 1 THEN ?10 ELSE deadline END,
            priority = COALESCE(?11, priority),
            status = ?12,
            locked = COALESCE(?13, locked),
            course_id = CASE WHEN ?14 = 1 THEN ?15 ELSE course_id END,
            completed_at = ?16,
            updated_at = ?17
         WHERE id = ?1",
        params![
            id,
            patch.title.as_ref().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()),
            patch.description,
            patch.estimated_minutes,
            if patch.scheduled_start.is_some() { 1 } else { 0 },
            patch.scheduled_start.clone().flatten(),
            if patch.scheduled_end.is_some() { 1 } else { 0 },
            patch.scheduled_end.clone().flatten(),
            if patch.deadline.is_some() { 1 } else { 0 },
            patch.deadline.clone().flatten(),
            patch.priority,
            status,
            patch.locked.map(i),
            if patch.course_id.is_some() { 1 } else { 0 },
            patch.course_id.clone().flatten(),
            completed_at,
            now_iso()
        ],
    )?;
    get_task(conn, id)
}

pub fn complete_task(conn: &Connection, id: &str, done: bool) -> AppResult<StudyTask> {
    let patch = TaskPatch {
        status: Some(if done { "done".into() } else { "todo".into() }),
        ..Default::default()
    };
    update_task(conn, id, &patch)
}

pub fn delete_task(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM study_tasks WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AppError::not_found(format!("学习任务不存在：{id}")));
    }
    Ok(())
}

/// Remove auto-generated, unlocked tasks of a plan before replanning.
///
/// User-created or locked tasks are preserved — the planner must never silently
/// discard manual edits (task specification §34).
pub fn clear_generated_tasks(conn: &Connection, plan_id: &str) -> AppResult<i64> {
    let deleted = conn.execute(
        "DELETE FROM study_tasks WHERE plan_id = ?1 AND locked = 0 AND generated_by <> 'user'
         AND status IN ('todo','in_progress')",
        [plan_id],
    )?;
    Ok(deleted as i64)
}

/// Tasks already occupying time in a window, used to avoid double-booking.
pub fn occupied_intervals(
    conn: &Connection,
    from: &str,
    to: &str,
) -> AppResult<Vec<(String, String, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT scheduled_start, scheduled_end, plan_id FROM study_tasks
         WHERE scheduled_start IS NOT NULL AND scheduled_end IS NOT NULL
           AND scheduled_start < ?2 AND scheduled_end > ?1
           AND status IN ('todo','in_progress','done')",
    )?;
    let rows = stmt.query_map(params![from, to], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn task_stats(conn: &Connection) -> AppResult<(i64, i64)> {
    let open: i64 = conn.query_row(
        "SELECT COUNT(*) FROM study_tasks WHERE status IN ('todo','in_progress')",
        [],
        |r| r.get(0),
    )?;
    let done: i64 = conn.query_row(
        "SELECT COUNT(*) FROM study_tasks WHERE status = 'done'",
        [],
        |r| r.get(0),
    )?;
    Ok((open, done))
}

pub fn tasks_for_day(conn: &Connection, day_start: &str, day_end: &str) -> AppResult<Vec<StudyTask>> {
    let sql = format!(
        "{TASK_SELECT} WHERE t.scheduled_start >= ?1 AND t.scheduled_start < ?2
         ORDER BY t.scheduled_start ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![day_start, day_end], map_task)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Study sessions
// ---------------------------------------------------------------------------

pub fn start_study_session(
    conn: &Connection,
    task_id: Option<&str>,
    course_id: Option<&str>,
) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO study_sessions (id, task_id, course_id, started_at, minutes, created_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?4)",
        params![id, task_id, course_id, now_iso()],
    )?;
    Ok(id)
}

pub fn finish_study_session(conn: &Connection, id: &str, minutes: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE study_sessions SET ended_at = ?2, minutes = ?3 WHERE id = ?1",
        params![id, now_iso(), minutes.max(0)],
    )?;
    Ok(())
}

pub fn list_study_sessions(conn: &Connection, limit: i64) -> AppResult<Vec<StudySessionRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, course_id, started_at, ended_at, minutes, notes, created_at
         FROM study_sessions ORDER BY started_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |row| {
        Ok(StudySessionRecord {
            id: row.get(0)?,
            task_id: row.get(1)?,
            course_id: row.get(2)?,
            started_at: row.get(3)?,
            ended_at: row.get(4)?,
            minutes: row.get(5)?,
            notes: row.get(6)?,
            created_at: row.get(7)?,
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
    use crate::storage::Db;

    fn seed_course(db: &Db) -> String {
        db.with(|c| {
            c.execute(
                "INSERT INTO courses (id, name, source, sync_status, archived, position, created_at, updated_at)
                 VALUES ('k1','线性代数','manual','local',0,1,'t','t')",
                [],
            )?;
            Ok("k1".to_string())
        })
        .unwrap()
    }

    #[test]
    fn preferences_defaults_and_validation() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let prefs = get_preferences(c)?;
            assert_eq!(prefs.daily_minutes, 120);
            assert_eq!(prefs.timezone, "Asia/Shanghai");

            let err = update_preferences(
                c,
                UpdatePreferencesInput {
                    preferred_start: Some("22:00".into()),
                    preferred_end: Some("20:00".into()),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert_eq!(err.code(), "invalid_input");

            let updated = update_preferences(
                c,
                UpdatePreferencesInput {
                    daily_minutes: Some(180),
                    study_days: Some(vec![1, 3, 5, 9]),
                    ..Default::default()
                },
            )?;
            assert_eq!(updated.daily_minutes, 180);
            assert_eq!(updated.study_days, vec![1, 3, 5], "invalid weekday dropped");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn task_lifecycle_and_completion() {
        let db = Db::open_in_memory().unwrap();
        let course = seed_course(&db);
        db.with(|c| {
            let plan = create_plan(c, "期中复习", "exam", "2026-11-01", "2026-11-07", None)?;
            let task = create_task(
                c,
                &TaskSeed {
                    plan_id: Some(plan.id.clone()),
                    course_id: Some(course.clone()),
                    material_id: None,
                    assignment_id: None,
                    title: "复习特征值".into(),
                    description: None,
                    estimated_minutes: 60,
                    scheduled_start: Some("2026-11-02T11:00:00Z".into()),
                    scheduled_end: Some("2026-11-02T12:00:00Z".into()),
                    deadline: None,
                    priority: 2,
                    generated_by: "rule".into(),
                    source_references: vec![],
                },
            )?;
            assert_eq!(task.status, "todo");
            assert_eq!(task.course_name.as_deref(), Some("线性代数"));

            let done = complete_task(c, &task.id, true)?;
            assert_eq!(done.status, "done");
            assert!(done.completed_at.is_some());

            let reopened = complete_task(c, &task.id, false)?;
            assert_eq!(reopened.status, "todo");
            assert!(reopened.completed_at.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn clearing_generated_tasks_preserves_locked_and_user_tasks() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let plan = create_plan(c, "P", "weekly", "2026-11-01", "2026-11-07", None)?;
            let generated = create_task(
                c,
                &TaskSeed {
                    plan_id: Some(plan.id.clone()),
                    course_id: None,
                    material_id: None,
                    assignment_id: None,
                    title: "auto".into(),
                    description: None,
                    estimated_minutes: 30,
                    scheduled_start: None,
                    scheduled_end: None,
                    deadline: None,
                    priority: 3,
                    generated_by: "rule".into(),
                    source_references: vec![],
                },
            )?;
            let locked = create_task(
                c,
                &TaskSeed {
                    plan_id: Some(plan.id.clone()),
                    course_id: None,
                    material_id: None,
                    assignment_id: None,
                    title: "locked".into(),
                    description: None,
                    estimated_minutes: 30,
                    scheduled_start: None,
                    scheduled_end: None,
                    deadline: None,
                    priority: 3,
                    generated_by: "rule".into(),
                    source_references: vec![],
                },
            )?;
            update_task(
                c,
                &locked.id,
                &TaskPatch {
                    locked: Some(true),
                    ..Default::default()
                },
            )?;
            let user = create_task(
                c,
                &TaskSeed {
                    plan_id: Some(plan.id.clone()),
                    course_id: None,
                    material_id: None,
                    assignment_id: None,
                    title: "user".into(),
                    description: None,
                    estimated_minutes: 30,
                    scheduled_start: None,
                    scheduled_end: None,
                    deadline: None,
                    priority: 3,
                    generated_by: "user".into(),
                    source_references: vec![],
                },
            )?;

            let deleted = clear_generated_tasks(c, &plan.id)?;
            assert_eq!(deleted, 1);
            assert!(get_task(c, &generated.id).is_err());
            assert!(get_task(c, &locked.id).is_ok());
            assert!(get_task(c, &user.id).is_ok());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn occupied_intervals_are_reported() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            create_task(
                c,
                &TaskSeed {
                    plan_id: None,
                    course_id: None,
                    material_id: None,
                    assignment_id: None,
                    title: "block".into(),
                    description: None,
                    estimated_minutes: 60,
                    scheduled_start: Some("2026-11-02T10:00:00Z".into()),
                    scheduled_end: Some("2026-11-02T11:00:00Z".into()),
                    deadline: None,
                    priority: 3,
                    generated_by: "user".into(),
                    source_references: vec![],
                },
            )?;
            let intervals = occupied_intervals(c, "2026-11-02T00:00:00Z", "2026-11-03T00:00:00Z")?;
            assert_eq!(intervals.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn task_validation() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let err = create_task(
                c,
                &TaskSeed {
                    plan_id: None,
                    course_id: None,
                    material_id: None,
                    assignment_id: None,
                    title: "  ".into(),
                    description: None,
                    estimated_minutes: 30,
                    scheduled_start: None,
                    scheduled_end: None,
                    deadline: None,
                    priority: 3,
                    generated_by: "user".into(),
                    source_references: vec![],
                },
            )
            .unwrap_err();
            assert_eq!(err.code(), "invalid_input");
            Ok(())
        })
        .unwrap();
    }
}
