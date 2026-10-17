//! Course and module commands.

use serde::Serialize;
use tauri::{State, WebviewWindow};

use crate::commands::guard::require_main_window;
use crate::domain::{
    Course, CourseModule, CreateCourseInput, CreateModuleInput, IdName, UpdateCourseInput,
};
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::course_repo;

#[tauri::command]
pub async fn list_courses(
    window: WebviewWindow,
    state: State<'_, AppState>,
    include_archived: Option<bool>,
) -> AppResult<Vec<Course>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| course_repo::list(c, include_archived.unwrap_or(false)))
}

#[tauri::command]
pub async fn get_course(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: String,
) -> AppResult<Course> {
    require_main_window(&window)?;
    state.db.with(|c| course_repo::get(c, &course_id))
}

#[tauri::command]
pub async fn create_course(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: CreateCourseInput,
) -> AppResult<Course> {
    require_main_window(&window)?;
    state.db.with(|c| course_repo::create(c, input, "manual"))
}

#[tauri::command]
pub async fn update_course(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: String,
    patch: UpdateCourseInput,
) -> AppResult<Course> {
    require_main_window(&window)?;
    state.db.with(|c| course_repo::update(c, &course_id, patch))
}

#[tauri::command]
pub async fn delete_course(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state.db.with(|c| course_repo::delete(c, &course_id))
}

#[tauri::command]
pub async fn list_course_modules(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: String,
) -> AppResult<Vec<CourseModule>> {
    require_main_window(&window)?;
    state.db.with(|c| course_repo::list_modules(c, &course_id))
}

#[tauri::command]
pub async fn create_course_module(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: CreateModuleInput,
) -> AppResult<CourseModule> {
    require_main_window(&window)?;
    state.db.with(|c| course_repo::create_module(c, input))
}

#[tauri::command]
pub async fn delete_course_module(
    window: WebviewWindow,
    state: State<'_, AppState>,
    module_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state.db.with(|c| course_repo::delete_module(c, &module_id))
}

/// Lightweight course list for filter dropdowns.
#[tauri::command]
pub async fn course_options(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<Vec<IdName>> {
    require_main_window(&window)?;
    state.db.with(|c| {
        Ok(course_repo::list(c, false)?
            .into_iter()
            .map(|course| IdName {
                id: course.id,
                name: course.name,
            })
            .collect())
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseDetail {
    pub course: Course,
    pub modules: Vec<CourseModule>,
}

#[tauri::command]
pub async fn get_course_detail(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: String,
) -> AppResult<CourseDetail> {
    require_main_window(&window)?;
    state.db.with(|c| {
        Ok(CourseDetail {
            course: course_repo::get(c, &course_id)?,
            modules: course_repo::list_modules(c, &course_id)?,
        })
    })
}
