//! Files tab: Canvas course files grouped by course.
//!
//! There is no manual upload any more — a course file exists locally only if it
//! was downloaded from Canvas, so this module is the single source for what the
//! Files tab shows: what is already downloaded and what is still pending.

use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::commands::guard::require_main_window;
use crate::domain::{CanvasFile, CourseFileGroup, DownloadReport, Material};
use crate::error::AppResult;
use crate::integrations::canvas::download;
use crate::state::AppState;
use crate::storage::{canvas_file_repo, document_repo};

/// Emitted while a download run progresses.
pub const DOWNLOAD_EVENT: &str = "canvas://download";

/// Setting key: download every listed file automatically after a sync.
pub const AUTO_DOWNLOAD_KEY: &str = "canvas.auto_download";

/// Per-run cap so a huge course cannot hold the queue for minutes.
const RUN_LIMIT: i64 = 200;

#[tauri::command]
pub async fn list_course_files(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: Option<String>,
) -> AppResult<Vec<CourseFileGroup>> {
    require_main_window(&window)?;
    state.db.with(|c| build_groups(c, course_id.as_deref()))
}

fn build_groups(
    conn: &rusqlite::Connection,
    course_id: Option<&str>,
) -> AppResult<Vec<CourseFileGroup>> {
    let mut stmt = conn.prepare(
        "SELECT id, name FROM courses
         WHERE (?1 IS NULL OR id = ?1)
         ORDER BY archived, position, name COLLATE NOCASE",
    )?;
    let courses: Vec<(String, String)> = stmt
        .query_map([course_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;

    let mut groups = Vec::new();
    for (id, name) in courses {
        let filter = crate::domain::MaterialFilter {
            course_id: Some(id.clone()),
            kind: None,
            query: None,
            favorites_only: false,
            index_status: None,
            limit: Some(2000),
            offset: None,
        };
        let materials: Vec<Material> = document_repo::list(conn, &filter)?;
        let files: Vec<CanvasFile> = canvas_file_repo::list_by_course(conn, &id)?;

        let mut folder_by_material = std::collections::HashMap::new();
        for file in &files {
            if let Some(material_id) = &file.material_id {
                folder_by_material.insert(material_id.clone(), file.folder_path.clone());
            }
        }

        let pending: Vec<CanvasFile> = files
            .into_iter()
            .filter(|f| f.state == canvas_file_repo::STATE_PENDING || f.state == canvas_file_repo::STATE_FAILED)
            .collect();
        if materials.is_empty() && pending.is_empty() {
            continue;
        }
        groups.push(CourseFileGroup {
            course_id: id,
            course_name: name,
            downloaded: materials,
            pending,
            folder_by_material,
        });
    }

    // Materials that were imported before files came only from Canvas have no
    // course. Hiding them would look like data loss, so they get their own
    // bucket instead.
    if course_id.is_none() {
        let mut stmt = conn.prepare(
            "SELECT id FROM materials WHERE course_id IS NULL ORDER BY title COLLATE NOCASE LIMIT 2000",
        )?;
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        let mut loose = Vec::new();
        for id in ids {
            if let Ok(material) = document_repo::get(conn, &id) {
                loose.push(material);
            }
        }
        if !loose.is_empty() {
            groups.push(CourseFileGroup {
                course_id: String::new(),
                course_name: "未归类".into(),
                downloaded: loose,
                pending: Vec::new(),
                folder_by_material: Default::default(),
            });
        }
    }

    Ok(groups)
}

#[tauri::command]
pub async fn download_canvas_file(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    file_id: String,
) -> AppResult<CanvasFile> {
    require_main_window(&window)?;
    let file = state.db.with(|c| canvas_file_repo::get(c, &file_id))?;

    let outcome = download::download_one(
        &state.db,
        &state.paths,
        state.canvas.as_ref(),
        &app,
        &file,
    )
    .await?;

    index_async(&app, &state, vec![outcome.id.clone()]);
    emit_progress(&app, &file.display_name, "downloaded", None);
    state.db.with(|c| canvas_file_repo::get(c, &file_id))
}

#[tauri::command]
pub async fn download_course_files(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    course_id: Option<String>,
) -> AppResult<DownloadReport> {
    require_main_window(&window)?;
    let result = download::download_pending(
        &state.db,
        &state.paths,
        state.canvas.as_ref(),
        &app,
        course_id.as_deref(),
        RUN_LIMIT,
    )
    .await;

    let ids: Vec<String> = result.materials.iter().map(|m| m.id.clone()).collect();
    index_async(&app, &state, ids);
    emit_progress(&app, "", "batch-done", Some(&result.report));
    Ok(result.report)
}

/// Start indexing the freshly downloaded materials in the background.
fn index_async(app: &AppHandle, state: &AppState, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    let ctx = crate::commands::materials::index_context(state);
    let language = state
        .db
        .with(|c| crate::storage::settings::get(c, crate::storage::settings::KEY_LOCALE))
        .ok()
        .flatten()
        .unwrap_or_else(|| "zh".into());
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        for id in ids {
            if let Err(error) =
                crate::documents::pipeline::index_material(&app_handle, &ctx, &id, &language).await
            {
                tracing::warn!(material = %id, error = %error, "indexing a downloaded file failed");
            }
        }
    });
}

fn emit_progress(
    app: &AppHandle,
    name: &str,
    state: &str,
    report: Option<&DownloadReport>,
) {
    use tauri::Emitter;
    let payload = serde_json::json!({
        "name": name,
        "state": state,
        "downloaded": report.map(|r| r.downloaded),
        "failed": report.map(|r| r.failed),
        "errors": report.map(|r| r.errors.clone()),
    });
    if let Err(error) = app.emit(DOWNLOAD_EVENT, payload) {
        tracing::warn!(%error, "could not emit download progress");
    }
}

/// Whether the user wants files downloaded automatically after a sync.
pub fn auto_download_enabled(state: &AppState) -> bool {
    state
        .db
        .with(|c| crate::storage::settings::get(c, AUTO_DOWNLOAD_KEY))
        .ok()
        .flatten()
        .map(|v| v != "false")
        .unwrap_or(true)
}

/// Spawn a background run over every pending file, used after a sync.
pub fn spawn_auto_download(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if !auto_download_enabled(&state) {
            return;
        }
        let pending = state
            .db
            .with(|c| canvas_file_repo::list_pending(c, None))
            .unwrap_or_default();
        if pending.is_empty() {
            return;
        }
        let result = download::download_pending(
            &state.db,
            &state.paths,
            state.canvas.as_ref(),
            &app,
            None,
            RUN_LIMIT,
        )
        .await;
        let ids: Vec<String> = result.materials.iter().map(|m| m.id.clone()).collect();
        index_async(&app, &state, ids);
        emit_progress(&app, "", "batch-done", Some(&result.report));
    });
}
