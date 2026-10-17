//! Coursework commands: assignments, announcements, grades, discussions, pages.

use tauri::{State, WebviewWindow};

use crate::commands::guard::require_main_window;
use crate::domain::{
    Announcement, Assignment, CoursePage, CreateAssignmentInput, Dashboard, DashboardStats,
    DiscussionEntry, DiscussionTopic, GradeRecord, UpdateAssignmentInput,
};
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::{coursework_repo, document_repo, planning_repo};

#[tauri::command]
pub async fn list_assignments(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: Option<String>,
    upcoming_only: Option<bool>,
    limit: Option<i64>,
) -> AppResult<Vec<Assignment>> {
    require_main_window(&window)?;
    state.db.with(|c| {
        coursework_repo::list_assignments(
            c,
            course_id.as_deref(),
            upcoming_only.unwrap_or(false),
            limit.unwrap_or(200).clamp(1, 1000),
        )
    })
}

#[tauri::command]
pub async fn create_assignment(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: CreateAssignmentInput,
) -> AppResult<Assignment> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| coursework_repo::create_assignment(c, input))
}

#[tauri::command]
pub async fn update_assignment(
    window: WebviewWindow,
    state: State<'_, AppState>,
    assignment_id: String,
    patch: UpdateAssignmentInput,
) -> AppResult<Assignment> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| coursework_repo::update_assignment(c, &assignment_id, patch))
}

#[tauri::command]
pub async fn delete_assignment(
    window: WebviewWindow,
    state: State<'_, AppState>,
    assignment_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| coursework_repo::delete_assignment(c, &assignment_id))
}

#[tauri::command]
pub async fn list_announcements(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: Option<String>,
    limit: Option<i64>,
) -> AppResult<Vec<Announcement>> {
    require_main_window(&window)?;
    state.db.with(|c| {
        coursework_repo::list_announcements(c, course_id.as_deref(), limit.unwrap_or(50))
    })
}

#[tauri::command]
pub async fn mark_announcement_read(
    window: WebviewWindow,
    state: State<'_, AppState>,
    announcement_id: String,
    read: bool,
) -> AppResult<()> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| coursework_repo::mark_announcement_read(c, &announcement_id, read))
}

#[tauri::command]
pub async fn list_grades(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: Option<String>,
) -> AppResult<Vec<GradeRecord>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| coursework_repo::list_grades(c, course_id.as_deref()))
}

#[tauri::command]
pub async fn list_discussions(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: String,
) -> AppResult<Vec<DiscussionTopic>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| coursework_repo::list_discussions(c, &course_id))
}

#[tauri::command]
pub async fn list_discussion_entries(
    window: WebviewWindow,
    state: State<'_, AppState>,
    topic_id: String,
) -> AppResult<Vec<DiscussionEntry>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| coursework_repo::list_discussion_entries(c, &topic_id))
}

#[tauri::command]
pub async fn list_course_pages(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: String,
) -> AppResult<Vec<CoursePage>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| coursework_repo::list_pages(c, &course_id))
}

/// Aggregated payload for the home screen (one round trip).
#[tauri::command]
pub async fn get_dashboard(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<Dashboard> {
    require_main_window(&window)?;

    let preferences = state.db.with(|c| planning_repo::get_preferences(c))?;
    let today = crate::time::today_in_zone(&preferences.timezone);
    let day_start = crate::time::to_iso(
        today
            .and_hms_opt(0, 0, 0)
            .expect("midnight is valid")
            .and_utc(),
    );
    let day_end = crate::time::to_iso(
        (today + chrono::Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .expect("midnight is valid")
            .and_utc(),
    );

    state.db.with(|c| {
        let courses = crate::storage::course_repo::list(c, false)?;
        let course_total = courses.len() as i64;
        let upcoming = coursework_repo::list_assignments(c, None, true, 8)?;
        let today_tasks = planning_repo::tasks_for_day(c, &day_start, &day_end)?;
        let recent_materials = document_repo::list(
            c,
            &crate::domain::MaterialFilter {
                course_id: None,
                kind: None,
                query: None,
                favorites_only: false,
                index_status: None,
                limit: Some(6),
                offset: None,
            },
        )?;
        let announcements = coursework_repo::list_announcements(c, None, 5)?;
        let (total_materials, indexed, pages) = document_repo::index_stats(c)?;
        let chat_count: i64 =
            c.query_row("SELECT COUNT(*) FROM chat_sessions", [], |r| r.get(0))?;
        let (open_tasks, done_tasks) = planning_repo::task_stats(c)?;
        let week_end = crate::time::to_iso(chrono::Utc::now() + chrono::Duration::days(7));
        let due_this_week: i64 = c.query_row(
            "SELECT COUNT(*) FROM assignments WHERE due_at IS NOT NULL AND due_at >= ?1 AND due_at <= ?2",
            rusqlite::params![crate::time::now_iso(), week_end],
            |r| r.get(0),
        )?;

        Ok(Dashboard {
            courses,
            upcoming,
            today_tasks,
            recent_materials,
            announcements,
            stats: DashboardStats {
                course_count: course_total,
                material_count: total_materials,
                indexed_count: indexed,
                page_count: pages,
                chat_count,
                open_tasks,
                done_tasks,
                due_this_week,
            },
        })
    })
}
