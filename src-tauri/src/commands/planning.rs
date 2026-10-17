//! Study planning, tasks and calendar commands.

use tauri::{State, WebviewWindow};

use crate::commands::guard::require_main_window;
use crate::domain::{
    CalendarEvent, CreateEventInput, CreateTaskInput, GeneratePlanInput, PlanResult,
    StudyPlan, StudyPreferences, StudySessionRecord, StudyTask, TaskPatch,
    UpdatePreferencesInput, UpcomingDeadline,
};
use crate::error::{AppError, AppResult};
use crate::planning;
use crate::state::AppState;
use crate::storage::{coursework_repo, planning_repo};

#[tauri::command]
pub async fn get_study_preferences(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<StudyPreferences> {
    require_main_window(&window)?;
    state.db.with(|c| planning_repo::get_preferences(c))
}

#[tauri::command]
pub async fn update_study_preferences(
    window: WebviewWindow,
    state: State<'_, AppState>,
    patch: UpdatePreferencesInput,
) -> AppResult<StudyPreferences> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| planning_repo::update_preferences(c, patch))
}

#[tauri::command]
pub async fn generate_study_plan(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: GeneratePlanInput,
) -> AppResult<PlanResult> {
    require_main_window(&window)?;
    planning::generate(&state.db, input)
}

#[tauri::command]
pub async fn replan_schedule(
    window: WebviewWindow,
    state: State<'_, AppState>,
    plan_id: String,
) -> AppResult<PlanResult> {
    require_main_window(&window)?;
    planning::replan(&state.db, &plan_id)
}

#[tauri::command]
pub async fn list_study_plans(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<Vec<StudyPlan>> {
    require_main_window(&window)?;
    state.db.with(|c| planning_repo::list_plans(c))
}

#[tauri::command]
pub async fn delete_study_plan(
    window: WebviewWindow,
    state: State<'_, AppState>,
    plan_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state.db.with(|c| planning_repo::delete_plan(c, &plan_id))
}

#[tauri::command]
pub async fn set_study_plan_status(
    window: WebviewWindow,
    state: State<'_, AppState>,
    plan_id: String,
    status: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| planning_repo::set_plan_status(c, &plan_id, &status))
}

#[tauri::command]
pub async fn list_study_tasks(
    window: WebviewWindow,
    state: State<'_, AppState>,
    plan_id: Option<String>,
    course_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
    include_done: Option<bool>,
) -> AppResult<Vec<StudyTask>> {
    require_main_window(&window)?;
    state.db.with(|c| {
        planning_repo::list_tasks(
            c,
            plan_id.as_deref(),
            course_id.as_deref(),
            from.as_deref(),
            to.as_deref(),
            include_done.unwrap_or(true),
        )
    })
}

#[tauri::command]
pub async fn create_study_task(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: CreateTaskInput,
) -> AppResult<StudyTask> {
    require_main_window(&window)?;
    state.db.with(|c| {
        planning_repo::create_task(
            c,
            &planning_repo::TaskSeed {
                plan_id: input.plan_id,
                course_id: input.course_id,
                material_id: None,
                assignment_id: None,
                title: input.title,
                description: input.description,
                estimated_minutes: input.estimated_minutes.unwrap_or(30),
                scheduled_start: input.scheduled_start,
                scheduled_end: input.scheduled_end,
                deadline: input.deadline,
                priority: input.priority.unwrap_or(3),
                generated_by: "user".into(),
                source_references: vec![],
            },
        )
    })
}

#[tauri::command]
pub async fn update_study_task(
    window: WebviewWindow,
    state: State<'_, AppState>,
    task_id: String,
    patch: TaskPatch,
) -> AppResult<StudyTask> {
    require_main_window(&window)?;
    state.db.with(|c| planning_repo::update_task(c, &task_id, &patch))
}

#[tauri::command]
pub async fn complete_study_task(
    window: WebviewWindow,
    state: State<'_, AppState>,
    task_id: String,
    done: bool,
) -> AppResult<StudyTask> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| planning_repo::complete_task(c, &task_id, done))
}

#[tauri::command]
pub async fn delete_study_task(
    window: WebviewWindow,
    state: State<'_, AppState>,
    task_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state.db.with(|c| planning_repo::delete_task(c, &task_id))
}

#[tauri::command]
pub async fn upcoming_deadlines(
    window: WebviewWindow,
    state: State<'_, AppState>,
    horizon_days: Option<i64>,
    limit: Option<i64>,
) -> AppResult<Vec<UpcomingDeadline>> {
    require_main_window(&window)?;
    planning::upcoming(
        &state.db,
        horizon_days.unwrap_or(30).clamp(1, 365),
        limit.unwrap_or(40).clamp(1, 200),
    )
}

#[tauri::command]
pub async fn start_study_session(
    window: WebviewWindow,
    state: State<'_, AppState>,
    task_id: Option<String>,
    course_id: Option<String>,
) -> AppResult<String> {
    require_main_window(&window)?;
    state.db.with(|c| {
        planning_repo::start_study_session(c, task_id.as_deref(), course_id.as_deref())
    })
}

#[tauri::command]
pub async fn finish_study_session(
    window: WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
    minutes: i64,
) -> AppResult<()> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| planning_repo::finish_study_session(c, &session_id, minutes))
}

#[tauri::command]
pub async fn list_study_sessions(
    window: WebviewWindow,
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> AppResult<Vec<StudySessionRecord>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| planning_repo::list_study_sessions(c, limit.unwrap_or(50)))
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_calendar_events(
    window: WebviewWindow,
    state: State<'_, AppState>,
    from: Option<String>,
    to: Option<String>,
) -> AppResult<Vec<CalendarEvent>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| coursework_repo::list_events(c, from.as_deref(), to.as_deref()))
}

#[tauri::command]
pub async fn create_calendar_event(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: CreateEventInput,
) -> AppResult<CalendarEvent> {
    require_main_window(&window)?;
    state.db.with(|c| coursework_repo::create_event(c, input))
}

#[tauri::command]
pub async fn delete_calendar_event(
    window: WebviewWindow,
    state: State<'_, AppState>,
    event_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state.db.with(|c| coursework_repo::delete_event(c, &event_id))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IcsImportResult {
    pub inserted: i64,
    pub updated: i64,
    pub recurring: i64,
}

/// Import an `.ics` file chosen through the native file dialog.
#[tauri::command]
pub async fn import_ics(
    window: WebviewWindow,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<IcsImportResult> {
    require_main_window(&window)?;
    let raw = crate::documents::plain::read_text(std::path::Path::new(&path))?;
    let events = crate::documents::ics::parse(&raw)?;

    let mut inserted = 0i64;
    let mut updated = 0i64;
    let mut recurring = 0i64;
    state.db.transaction(|tx| {
        for event in &events {
            if event.recurring {
                recurring += 1;
            }
            let created = coursework_repo::upsert_ics_event(
                tx,
                &coursework_repo::IcsEventInput {
                    uid: &event.uid,
                    title: &event.summary,
                    description: event.description.as_deref(),
                    location: event.location.as_deref(),
                    starts_at: &event.starts_at,
                    ends_at: event.ends_at.as_deref(),
                    all_day: event.all_day,
                },
            )?;
            if created {
                inserted += 1;
            } else {
                updated += 1;
            }
        }
        Ok(())
    })?;

    Ok(IcsImportResult {
        inserted,
        updated,
        recurring,
    })
}

/// Write the current calendar to an `.ics` file at a user-chosen path.
#[tauri::command]
pub async fn export_ics(
    window: WebviewWindow,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<i64> {
    require_main_window(&window)?;
    let events = state
        .db
        .with(|c| coursework_repo::list_events(c, None, None))?;
    let ics = crate::documents::ics::render(&events);
    let target = std::path::Path::new(&path);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(target, ics)?;
    Ok(events.len() as i64)
}

/// Calendar events for a month, used by the month grid.
#[tauri::command]
pub async fn calendar_month(
    window: WebviewWindow,
    state: State<'_, AppState>,
    year: i32,
    month: u32,
) -> AppResult<Vec<CalendarEvent>> {
    require_main_window(&window)?;
    if !(1..=12).contains(&month) {
        return Err(AppError::invalid("月份必须在 1 到 12 之间"));
    }
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| AppError::invalid("日期无效"))?;
    let next = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .ok_or_else(|| AppError::invalid("日期无效"))?;
    let from = crate::time::to_iso(first.and_hms_opt(0, 0, 0).unwrap().and_utc());
    let to = crate::time::to_iso(next.and_hms_opt(0, 0, 0).unwrap().and_utc());
    state
        .db
        .with(|c| coursework_repo::list_events(c, Some(&from), Some(&to)))
}
