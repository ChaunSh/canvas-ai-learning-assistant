//! Data ownership commands: export, import, cleanup and reset.
//!
//! All destructive operations require an explicit call from the UI; none run
//! implicitly. Exports are plain JSON so users are never locked in.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::{State, WebviewWindow};

use crate::commands::guard::require_main_window;
use crate::domain::{Course, Material, StudyPlan, StudyTask};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::document_repo;
use crate::time::now_iso;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDataExport {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub app_version: String,
    pub courses: Vec<Course>,
    pub materials: Vec<Material>,
    pub study_plans: Vec<StudyPlan>,
    pub study_tasks: Vec<StudyTask>,
    pub settings: std::collections::HashMap<String, String>,
    /// Notes are exported as Markdown; binary files are copied separately.
    pub note: String,
}

/// Write a JSON export of the user's own data.
#[tauri::command]
pub async fn export_user_data(
    window: WebviewWindow,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<u64> {
    require_main_window(&window)?;
    let export = state.db.with(|c| build_export(c))?;
    let text = serde_json::to_string_pretty(&export)?;
    std::fs::write(&path, &text)?;
    Ok(text.len() as u64)
}

pub fn build_export(conn: &Connection) -> AppResult<UserDataExport> {
    Ok(UserDataExport {
        format: "canvas-ai-assistant/user-data".into(),
        version: 1,
        exported_at: now_iso(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        courses: crate::storage::course_repo::list(conn, true)?,
        materials: document_repo::list(
            conn,
            &crate::domain::MaterialFilter {
                course_id: None,
                kind: None,
                query: None,
                favorites_only: false,
                index_status: None,
                limit: Some(10_000),
                offset: None,
            },
        )?,
        study_plans: crate::storage::planning_repo::list_plans(conn)?,
        study_tasks: crate::storage::planning_repo::list_tasks(conn, None, None, None, None, true)?,
        settings: crate::storage::settings::all(conn)?,
        note: "课件二进制文件不在本 JSON 中；请一并备份应用数据目录下的 materials/ 文件夹。学习附件与 AI 密钥不导出。".into(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub courses_added: i64,
    pub plans_added: i64,
    pub tasks_added: i64,
    pub skipped: i64,
}

/// Import a JSON export produced by [`export_user_data`].
///
/// Materials are intentionally not re-created: their bytes live in the file
/// store and a JSON row without bytes would be a broken record. The report tells
/// the user exactly what was and was not imported.
#[tauri::command]
pub async fn import_user_data(
    window: WebviewWindow,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<ImportReport> {
    require_main_window(&window)?;
    let raw = std::fs::read_to_string(&path)?;
    let export: UserDataExport = serde_json::from_str(&raw)
        .map_err(|e| AppError::invalid(format!("不是有效的导出文件：{e}")))?;
    if export.format != "canvas-ai-assistant/user-data" {
        return Err(AppError::invalid("导出文件类型不匹配"));
    }

    apply_import(&state.db, &export)
}

/// Insert exported records, skipping anything that already exists.
///
/// Kept separate from the command so the merge rules can be tested without a
/// webview (see the unit tests at the bottom of this file).
pub fn apply_import(
    db: &crate::storage::Db,
    export: &UserDataExport,
) -> AppResult<ImportReport> {
    let mut courses_added = 0i64;
    let mut plans_added = 0i64;
    let mut tasks_added = 0i64;
    let mut skipped = 0i64;

    db.transaction(|tx| {
        for course in &export.courses {
            // Manual courses only: Canvas courses are owned by sync.
            if course.source != "manual" {
                skipped += 1;
                continue;
            }
            let exists: i64 = tx.query_row(
                "SELECT COUNT(*) FROM courses WHERE id = ?1",
                [&course.id],
                |r| r.get(0),
            )?;
            if exists > 0 {
                skipped += 1;
                continue;
            }
            tx.execute(
                "INSERT INTO courses (id, external_id, code, name, term, instructor, description,
                                      source, sync_status, archived, color, position,
                                      last_synced_at, created_at, updated_at)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, 'manual', 'local', ?7, ?8, ?9, NULL, ?10, ?11)",
                rusqlite::params![
                    course.id,
                    course.code,
                    course.name,
                    course.term,
                    course.instructor,
                    course.description,
                    if course.archived { 1 } else { 0 },
                    course.color,
                    course.position,
                    course.created_at,
                    course.updated_at
                ],
            )?;
            courses_added += 1;
        }

        for plan in &export.study_plans {
            let exists: i64 = tx.query_row(
                "SELECT COUNT(*) FROM study_plans WHERE id = ?1",
                [&plan.id],
                |r| r.get(0),
            )?;
            if exists > 0 {
                skipped += 1;
                continue;
            }
            tx.execute(
                "INSERT INTO study_plans (id, name, kind, start_date, end_date, status, notes, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    plan.id,
                    plan.name,
                    plan.kind,
                    plan.start_date,
                    plan.end_date,
                    plan.status,
                    plan.notes,
                    plan.created_at,
                    plan.updated_at
                ],
            )?;
            plans_added += 1;
        }

        for task in &export.study_tasks {
            let exists: i64 = tx.query_row(
                "SELECT COUNT(*) FROM study_tasks WHERE id = ?1",
                [&task.id],
                |r| r.get(0),
            )?;
            if exists > 0 {
                skipped += 1;
                continue;
            }
            // A task pointing at a course that was skipped (Canvas-owned, or a
            // course the target already has under another id) would violate the
            // foreign key and abort the whole import. Skip it instead so a
            // partial restore still succeeds and reports what was dropped.
            if let Some(course_id) = task.course_id.as_deref() {
                let course_exists: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM courses WHERE id = ?1",
                    [course_id],
                    |r| r.get(0),
                )?;
                if course_exists == 0 {
                    skipped += 1;
                    continue;
                }
            }
            tx.execute(
                "INSERT INTO study_tasks (id, plan_id, course_id, material_id, assignment_id, title,
                                          description, estimated_minutes, scheduled_start,
                                          scheduled_end, deadline, priority, status, locked,
                                          generated_by, source_references, completed_at,
                                          created_at, updated_at)
                 VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                rusqlite::params![
                    task.id,
                    task.plan_id,
                    task.course_id,
                    task.title,
                    task.description,
                    task.estimated_minutes,
                    task.scheduled_start,
                    task.scheduled_end,
                    task.deadline,
                    task.priority,
                    task.status,
                    if task.locked { 1 } else { 0 },
                    task.generated_by,
                    serde_json::to_string(&task.source_references)?,
                    task.completed_at,
                    task.created_at,
                    task.updated_at
                ],
            )?;
            tasks_added += 1;
        }
        Ok(())
    })?;

    Ok(ImportReport {
        courses_added,
        plans_added,
        tasks_added,
        skipped,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub path: String,
    pub bytes: u64,
}

/// Create a consistent SQLite snapshot in the app's backup directory.
#[tauri::command]
pub async fn backup_database(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<BackupResult> {
    require_main_window(&window)?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let target = state
        .paths
        .backups_dir
        .join(format!("canvas-ai-{stamp}.sqlite"));
    let bytes = state.db.backup_to(&target)?;
    Ok(BackupResult {
        path: target.to_string_lossy().to_string(),
        bytes,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheInfo {
    pub database_bytes: u64,
    pub material_bytes: i64,
    pub material_count: i64,
    pub cache_dir_bytes: u64,
}

#[tauri::command]
pub async fn cache_info(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<CacheInfo> {
    require_main_window(&window)?;
    let material_bytes = state.db.with(|c| document_repo::storage_bytes(c))?;
    let material_count = state.db.with(|c| {
        Ok(c.query_row("SELECT COUNT(*) FROM materials", [], |r| r.get::<_, i64>(0))?)
    })?;
    Ok(CacheInfo {
        database_bytes: state.db.file_size(),
        material_bytes,
        material_count,
        cache_dir_bytes: dir_size(&state.paths.cache_dir),
    })
}

fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => total += dir_size(&entry.path()),
                Ok(meta) => total += meta.len(),
                Err(_) => {}
            }
        }
    }
    total
}

/// Delete derived caches (page renders, temporary files). Never touches originals.
#[tauri::command]
pub async fn clear_cached_files(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    require_main_window(&window)?;
    let before = dir_size(&state.paths.cache_dir);
    if state.paths.cache_dir.exists() {
        std::fs::remove_dir_all(&state.paths.cache_dir)?;
        std::fs::create_dir_all(&state.paths.cache_dir)?;
    }
    Ok(before)
}

/// Delete a course together with its materials' stored bytes.
#[tauri::command]
pub async fn delete_course_data(
    window: WebviewWindow,
    state: State<'_, AppState>,
    course_id: String,
) -> AppResult<u64> {
    require_main_window(&window)?;
    let materials = state.db.with(|c| {
        document_repo::list(
            c,
            &crate::domain::MaterialFilter {
                course_id: Some(course_id.clone()),
                kind: None,
                query: None,
                favorites_only: false,
                index_status: None,
                limit: Some(10_000),
                offset: None,
            },
        )
    })?;

    let mut removed = 0u64;
    for material in &materials {
        let ctx = crate::documents::pipeline::IndexContext {
            db: state.db.clone(),
            http: state.http.clone(),
            secrets: state.secrets.clone(),
            index: state.index.clone(),
        };
        let _ = crate::documents::pipeline::purge_derivatives(&ctx, &material.id);
        if std::fs::remove_file(&material.storage_path).is_ok() {
            removed += 1;
        }
    }

    state.db.with(|c| {
        c.execute("DELETE FROM materials WHERE course_id = ?1", [&course_id])?;
        crate::storage::course_repo::delete(c, &course_id)
    })?;
    Ok(removed)
}

/// Full reset: remove all local data and derived files.
#[tauri::command]
pub async fn reset_application_data(
    window: WebviewWindow,
    state: State<'_, AppState>,
    confirm: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    // A typed confirmation prevents an accidental destructive click.
    if confirm != "RESET" {
        return Err(AppError::invalid("需要输入 RESET 以确认删除全部本地数据"));
    }

    state.ai.cancel_all();
    state
        .canvas
        .disconnect(&tauri::Manager::app_handle(&window))
        .await
        .ok();

    state.db.with(|c| {
        // Children first: the schema uses ON DELETE CASCADE, but deleting the
        // FTS table explicitly guarantees no orphaned index rows.
        c.execute_batch(
            "DELETE FROM chunk_fts;
             DELETE FROM chunk_embeddings;
             DELETE FROM document_chunks;
             DELETE FROM document_pages;
             DELETE FROM document_summaries;
             DELETE FROM document_processing_jobs;
             DELETE FROM chat_citations;
             DELETE FROM chat_messages;
             DELETE FROM chat_sessions;
             DELETE FROM ai_usage_records;
             DELETE FROM study_sessions;
             DELETE FROM study_tasks;
             DELETE FROM study_plans;
             DELETE FROM study_progress;
             DELETE FROM sync_errors;
             DELETE FROM sync_checkpoints;
             DELETE FROM sync_runs;
             DELETE FROM integration_capabilities;
             DELETE FROM integration_connections;
             DELETE FROM resource_mappings;
             DELETE FROM import_jobs;
             DELETE FROM materials;
             DELETE FROM material_versions;
             DELETE FROM grade_records;
             DELETE FROM discussion_entries;
             DELETE FROM discussion_topics;
             DELETE FROM course_pages;
             DELETE FROM announcements;
             DELETE FROM assignments;
             DELETE FROM calendar_events;
             DELETE FROM course_modules;
             DELETE FROM courses;
             DELETE FROM app_settings;",
        )?;
        Ok(())
    })?;

    if state.paths.materials_dir.exists() {
        std::fs::remove_dir_all(&state.paths.materials_dir)?;
        std::fs::create_dir_all(&state.paths.materials_dir)?;
    }
    if state.paths.cache_dir.exists() {
        std::fs::remove_dir_all(&state.paths.cache_dir)?;
        std::fs::create_dir_all(&state.paths.cache_dir)?;
    }

    // Remove stored API keys.
    for provider_id in state
        .db
        .with(|c| crate::storage::ai_repo::list_providers(c))
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.id)
    {
        let _ = state
            .secrets
            .delete(&crate::security::secrets::provider_key(&provider_id));
    }

    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageOverview {
    pub data_dir: String,
    pub materials_dir: String,
    pub backups_dir: String,
    pub secrets_backing: String,
    pub secrets_secure: bool,
}

#[tauri::command]
pub async fn storage_overview(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<StorageOverview> {
    require_main_window(&window)?;
    Ok(StorageOverview {
        data_dir: state.paths.data_dir.to_string_lossy().to_string(),
        materials_dir: state.paths.materials_dir.to_string_lossy().to_string(),
        backups_dir: state.paths.backups_dir.to_string_lossy().to_string(),
        secrets_backing: state.secrets.backing().as_str().to_string(),
        secrets_secure: state.secrets.backing().is_secure(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::CreateCourseInput;
    use crate::storage::{course_repo, planning_repo, Db};

    fn seed(db: &Db) -> (String, String) {
        let course = db
            .with(|c| {
                course_repo::create(
                    c,
                    CreateCourseInput {
                        name: "线性代数".into(),
                        code: Some("MATH 201".into()),
                        term: Some("2026 Fall".into()),
                        instructor: Some("Dr. Li".into()),
                        description: Some("矩阵与特征值".into()),
                        color: None,
                    },
                    "manual",
                )
            })
            .expect("create course");

        let plan = db
            .with(|c| planning_repo::create_plan(c, "期末复习", "exam", "2026-12-01", "2026-12-10", Some("第一轮")))
            .expect("create plan");

        db.with(|c| {
            planning_repo::create_task(
                c,
                &planning_repo::TaskSeed {
                    plan_id: Some(plan.id.clone()),
                    course_id: Some(course.id.clone()),
                    material_id: None,
                    assignment_id: None,
                    title: "复习第一章".into(),
                    description: None,
                    estimated_minutes: 45,
                    scheduled_start: None,
                    scheduled_end: None,
                    deadline: None,
                    priority: 2,
                    generated_by: "user".into(),
                    source_references: vec![],
                },
            )
        })
        .expect("create task");

        (course.id, plan.id)
    }

    /// Export -> JSON text -> parse -> import, i.e. exactly what the UI does.
    fn roundtrip(source: &Db, target: &Db) -> ImportReport {
        let export = source.with(|c| build_export(c)).expect("build export");
        let text = serde_json::to_string_pretty(&export).expect("serialize");
        let parsed: UserDataExport = serde_json::from_str(&text).expect("parse export");
        assert_eq!(parsed.format, "canvas-ai-assistant/user-data");
        apply_import(target, &parsed).expect("apply import")
    }

    #[test]
    fn export_then_import_restores_courses_plans_and_tasks() {
        let source = Db::open_in_memory().expect("db");
        let (course_id, plan_id) = seed(&source);
        let target = Db::open_in_memory().expect("db");

        let report = roundtrip(&source, &target);
        assert_eq!(report.courses_added, 1);
        assert_eq!(report.plans_added, 1);
        assert_eq!(report.tasks_added, 1);
        assert_eq!(report.skipped, 0);

        let courses = target.with(|c| course_repo::list(c, true)).expect("list");
        assert_eq!(courses.len(), 1);
        assert_eq!(courses[0].id, course_id);
        assert_eq!(courses[0].name, "线性代数");
        assert_eq!(courses[0].code.as_deref(), Some("MATH 201"));

        let plans = target.with(|c| planning_repo::list_plans(c)).expect("plans");
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].id, plan_id);
        assert_eq!(plans[0].start_date, "2026-12-01");

        let tasks = target
            .with(|c| planning_repo::list_tasks(c, None, None, None, None, true))
            .expect("tasks");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "复习第一章");
        assert_eq!(tasks[0].estimated_minutes, 45);
        // The link back to the plan must survive the roundtrip.
        assert_eq!(tasks[0].plan_id.as_deref(), Some(plan_id.as_str()));
    }

    #[test]
    fn importing_the_same_export_twice_is_idempotent() {
        let source = Db::open_in_memory().expect("db");
        seed(&source);
        let target = Db::open_in_memory().expect("db");

        let first = roundtrip(&source, &target);
        assert_eq!(first.courses_added + first.plans_added + first.tasks_added, 3);

        let second = roundtrip(&source, &target);
        assert_eq!(second.courses_added, 0, "no duplicate courses");
        assert_eq!(second.plans_added, 0, "no duplicate plans");
        assert_eq!(second.tasks_added, 0, "no duplicate tasks");
        assert_eq!(second.skipped, 3, "everything reported as skipped");

        let courses = target.with(|c| course_repo::list(c, true)).expect("list");
        assert_eq!(courses.len(), 1);
    }

    #[test]
    fn canvas_owned_courses_are_left_to_sync() {
        let source = Db::open_in_memory().expect("db");
        seed(&source);
        let mut export = source.with(|c| build_export(c)).expect("build export");

        // Pretend the export came from a machine where this course was synced
        // from Canvas: importing it must not fight the sync engine.
        for course in &mut export.courses {
            course.source = "canvas".into();
        }

        let target = Db::open_in_memory().expect("db");
        let report = apply_import(&target, &export).expect("apply import");

        assert_eq!(report.courses_added, 0);
        assert!(report.skipped >= 1);
        let courses = target.with(|c| course_repo::list(c, true)).expect("list");
        assert!(courses.is_empty());
    }

    #[test]
    fn export_declares_its_format_and_never_contains_secrets() {
        let source = Db::open_in_memory().expect("db");
        seed(&source);
        let export = source.with(|c| build_export(c)).expect("build export");
        let text = serde_json::to_string(&export).expect("serialize");

        assert_eq!(export.format, "canvas-ai-assistant/user-data");
        assert_eq!(export.version, 1);
        assert!(!export.exported_at.is_empty());
        // Nothing key-shaped may leak into an export users may share.
        let lowered = text.to_ascii_lowercase();
        assert!(!lowered.contains("api_key"));
        assert!(!lowered.contains("api key"));
        assert!(!lowered.contains("sk-"));
        assert!(!lowered.contains("bearer "));
    }
}