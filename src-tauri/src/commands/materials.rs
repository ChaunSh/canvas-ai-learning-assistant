//! Material import, reading, indexing and search commands.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::commands::guard::require_main_window;
use crate::domain::{
    DocumentChunk, DocumentPage, DocumentSummary, ImportFailure, ImportInput, ImportOutcome,
    ImportSkip, IndexProgress, Material, MaterialFilter, SearchQuery, SearchResponse,
    SummaryInput, UpdateMaterialInput,
};
use crate::documents::{import, pipeline};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::{document_repo, settings};
use crate::time::now_iso;

/// Build the indexing context from managed state.
pub(crate) fn index_context(state: &AppState) -> pipeline::IndexContext {
    pipeline::IndexContext {
        db: state.db.clone(),
        http: state.http.clone(),
        secrets: state.secrets.clone(),
        index: state.index.clone(),
    }
}

#[tauri::command]
pub async fn import_materials(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    input: ImportInput,
) -> AppResult<ImportOutcome> {
    require_main_window(&window)?;
    if input.paths.is_empty() {
        return Err(AppError::invalid("未选择任何文件"));
    }

    let db = state.db.clone();
    let paths = state.paths.clone();
    let course_id = input.course_id.clone();
    let module_id = input.module_id.clone();
    let job_id = db.with(|c| {
        crate::storage::sync_repo::create_import_job(
            c,
            "material",
            Some(&format!("{} 个文件", input.paths.len())),
        )
    })?;

    let mut imported: Vec<Material> = Vec::new();
    let mut skipped: Vec<ImportSkip> = Vec::new();
    let mut failed: Vec<ImportFailure> = Vec::new();
    let total = input.paths.len() as i64;

    for (index, raw_path) in input.paths.iter().enumerate() {
        let path = std::path::PathBuf::from(raw_path);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(raw_path)
            .to_string();

        let is_zip = crate::documents::detect_kind(&name, None) == "zip";
        if is_zip && input.expand_archives {
            match import::import_zip(
                &db,
                &paths,
                &path,
                course_id.as_deref(),
                module_id.as_deref(),
                "import",
            ) {
                Ok((materials, notes)) => {
                    imported.extend(materials);
                    for note in notes {
                        skipped.push(ImportSkip {
                            path: name.clone(),
                            reason: note,
                        });
                    }
                }
                Err(error) => failed.push(ImportFailure {
                    path: raw_path.clone(),
                    error: error.to_string(),
                }),
            }
        } else if is_zip {
            // Stored as an opaque file: the user chose not to expand it.
            match import::import_file(
                &db,
                &paths,
                &path,
                course_id.as_deref(),
                module_id.as_deref(),
                "import",
                None,
                None,
            ) {
                Ok(result) => {
                    if result.duplicate {
                        skipped.push(ImportSkip {
                            path: name,
                            reason: "已存在相同内容的课件".into(),
                        });
                    } else {
                        imported.push(result.material);
                    }
                }
                Err(error) => failed.push(ImportFailure {
                    path: raw_path.clone(),
                    error: error.to_string(),
                }),
            }
        } else {
            match import::import_file(
                &db,
                &paths,
                &path,
                course_id.as_deref(),
                module_id.as_deref(),
                "import",
                None,
                None,
            ) {
                Ok(result) => {
                    if result.duplicate {
                        skipped.push(ImportSkip {
                            path: name,
                            reason: "已存在相同内容的课件".into(),
                        });
                    } else {
                        imported.push(result.material);
                    }
                }
                Err(error) => failed.push(ImportFailure {
                    path: raw_path.clone(),
                    error: error.to_string(),
                }),
            }
        }

        db.with(|c| {
            crate::storage::sync_repo::update_import_job(
                c,
                &job_id,
                "running",
                index as i64 + 1,
                total,
                Some(&format!("已处理 {}", index + 1)),
            )
        })?;
    }

    let status = if failed.is_empty() { "completed" } else { "completed" };
    db.with(|c| {
        crate::storage::sync_repo::update_import_job(
            c,
            &job_id,
            status,
            imported.len() as i64,
            total,
            Some(&format!(
                "导入 {} 个，跳过 {} 个，失败 {} 个",
                imported.len(),
                skipped.len(),
                failed.len()
            )),
        )
    })?;

    // Index new materials in the background so the UI is usable immediately.
    let ids: Vec<String> = imported.iter().map(|m| m.id.clone()).collect();
    if !ids.is_empty() {
        let ctx = index_context(&state);
        let app_handle = app.clone();
        let language = db
            .with(|c| settings::get(c, settings::KEY_LOCALE))
            .ok()
            .flatten()
            .unwrap_or_else(|| "zh".into());
        tauri::async_runtime::spawn(async move {
            for id in ids {
                if let Err(error) = pipeline::index_material(&app_handle, &ctx, &id, &language).await {
                    tracing::warn!(material = %id, error = %error, "background indexing failed");
                }
            }
        });
    }

    Ok(ImportOutcome {
        job_id,
        imported,
        skipped,
        failed,
    })
}

#[tauri::command]
pub async fn list_materials(
    window: WebviewWindow,
    state: State<'_, AppState>,
    filter: MaterialFilter,
) -> AppResult<Vec<Material>> {
    require_main_window(&window)?;
    state.db.with(|c| document_repo::list(c, &filter))
}

#[tauri::command]
pub async fn get_material(
    window: WebviewWindow,
    state: State<'_, AppState>,
    material_id: String,
) -> AppResult<Material> {
    require_main_window(&window)?;
    state.db.with(|c| {
        document_repo::touch_opened(c, &material_id)?;
        document_repo::get(c, &material_id)
    })
}

#[tauri::command]
pub async fn update_material(
    window: WebviewWindow,
    state: State<'_, AppState>,
    material_id: String,
    patch: UpdateMaterialInput,
) -> AppResult<Material> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| document_repo::update(c, &material_id, patch))
}

#[tauri::command]
pub async fn delete_material(
    window: WebviewWindow,
    state: State<'_, AppState>,
    material_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    let material = state.db.with(|c| document_repo::get(c, &material_id))?;
    let ctx = index_context(&state);
    pipeline::purge_derivatives(&ctx, &material_id)?;
    state.db.with(|c| document_repo::delete(c, &material_id))?;

    // Remove the stored blob only when no other row references the same hash.
    let still_used = state
        .db
        .with(|c| document_repo::find_by_sha(c, &material.sha256))?;
    if still_used.is_none() {
        if let Err(error) = std::fs::remove_file(&material.storage_path) {
            tracing::warn!(path = %material.storage_path, %error, "could not delete material blob");
        }
    }
    Ok(())
}

/// Raw file bytes for the PDF/image reader.
///
/// Returned as an IPC byte response (not base64) so large PDFs transfer
/// efficiently on both desktop and Android.
#[tauri::command]
pub async fn material_bytes(
    window: WebviewWindow,
    state: State<'_, AppState>,
    material_id: String,
) -> AppResult<tauri::ipc::Response> {
    require_main_window(&window)?;
    let material = state.db.with(|c| document_repo::get(c, &material_id))?;
    let root = state.paths.materials_dir.clone();
    let path = crate::security::paths_guard::ensure_within(
        &root,
        std::path::Path::new(&material.storage_path),
    )?;
    let bytes = std::fs::read(&path)?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn material_pages(
    window: WebviewWindow,
    state: State<'_, AppState>,
    material_id: String,
) -> AppResult<Vec<DocumentPage>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| document_repo::list_pages(c, &material_id))
}

#[tauri::command]
pub async fn material_chunks(
    window: WebviewWindow,
    state: State<'_, AppState>,
    material_id: String,
) -> AppResult<Vec<DocumentChunk>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| document_repo::list_chunks(c, &material_id))
}

#[tauri::command]
pub async fn index_material(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    material_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    let ctx = index_context(&state);
    let language = state
        .db
        .with(|c| settings::get(c, settings::KEY_LOCALE))
        .ok()
        .flatten()
        .unwrap_or_else(|| "zh".into());
    let app_handle = app.clone();
    let id = material_id.clone();
    // Run in the background; progress arrives via `materials://index`.
    tauri::async_runtime::spawn(async move {
        if let Err(error) = pipeline::index_material(&app_handle, &ctx, &id, &language).await {
            tracing::warn!(material = %id, error = %error, "manual indexing failed");
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn cancel_indexing(
    window: WebviewWindow,
    state: State<'_, AppState>,
    material_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    pipeline::mark_cancelled(&state.db, &material_id)
}

#[tauri::command]
pub async fn search_materials(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    query: SearchQuery,
) -> AppResult<SearchResponse> {
    require_main_window(&window)?;

    // Hybrid retrieval: embed the query only when an embedding model exists.
    let embedding = crate::ai::embed_query(
        &state.db,
        state.secrets.as_ref(),
        &state.http,
        None,
        &query.query,
    )
    .await;

    let db = state.db.clone();
    let index = state.index.clone();
    let _ = app;
    let outcome = db.with(|c| {
        crate::retrieval::search(
            c,
            crate::retrieval::RetrievalRequest {
                query: &query.query,
                material_ids: &query.material_ids,
                course_ids: &query.course_id.clone().map(|id| vec![id]).unwrap_or_default(),
                limit: query.limit.unwrap_or(20).clamp(1, 100) as usize,
                query_embedding: embedding.as_ref().map(|(v, _)| v.as_slice()),
                embedding_model: embedding.as_ref().map(|(_, m)| m.as_str()),
            },
            index.as_ref(),
        )
    })?;
    Ok(outcome.into_response())
}

#[tauri::command]
pub async fn list_summaries(
    window: WebviewWindow,
    state: State<'_, AppState>,
    material_id: String,
) -> AppResult<Vec<DocumentSummary>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| document_repo::list_summaries(c, &material_id))
}

#[tauri::command]
pub async fn generate_summary(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: SummaryInput,
) -> AppResult<DocumentSummary> {
    require_main_window(&window)?;
    let kind = input.kind.clone().unwrap_or_else(|| "full".into());
    let language = input
        .language
        .clone()
        .unwrap_or_else(|| "zh".into());
    crate::ai::generate_summary(
        &state.db,
        state.secrets.as_ref(),
        &state.http,
        &input.material_id,
        &kind,
        input.scope.as_deref(),
        &language,
        input.force,
    )
    .await
}

#[tauri::command]
pub async fn delete_summary(
    window: WebviewWindow,
    state: State<'_, AppState>,
    material_id: String,
    kind: String,
    scope: Option<String>,
) -> AppResult<()> {
    require_main_window(&window)?;
    state.db.with(|c| {
        c.execute(
            "DELETE FROM document_summaries WHERE material_id = ?1 AND kind = ?2
             AND COALESCE(scope,'') = COALESCE(?3,'')",
            rusqlite::params![material_id, kind, scope],
        )?;
        Ok(())
    })
}

#[tauri::command]
pub async fn reindex_all(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    require_main_window(&window)?;
    let ctx = index_context(&state);
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let failures = pipeline::reindex_all(&app_handle, &ctx, "zh").await;
        if !failures.is_empty() {
            tracing::warn!(count = failures.len(), "reindex completed with failures");
        }
    });
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    pub total: i64,
    pub indexed: i64,
    pub pages: i64,
    pub bytes: i64,
    pub needs_ocr: i64,
    pub pending: i64,
}

#[tauri::command]
pub async fn library_stats(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<LibraryStats> {
    require_main_window(&window)?;
    state.db.with(|c| {
        let (total, indexed, pages) = document_repo::index_stats(c)?;
        let bytes = document_repo::storage_bytes(c)?;
        let needs_ocr: i64 = c.query_row(
            "SELECT COUNT(*) FROM materials WHERE needs_ocr = 1",
            [],
            |r| r.get(0),
        )?;
        let pending: i64 = c.query_row(
            "SELECT COUNT(*) FROM materials WHERE index_status IN ('pending','queued','processing')",
            [],
            |r| r.get(0),
        )?;
        Ok(LibraryStats {
            total,
            indexed,
            pages,
            bytes,
            needs_ocr,
            pending,
        })
    })
}

/// Latest indexing job for a material (used to restore progress after a reload).
#[tauri::command]
pub async fn indexing_status(
    window: WebviewWindow,
    state: State<'_, AppState>,
    material_id: String,
) -> AppResult<Option<IndexProgress>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| document_repo::latest_job(c, &material_id))
}

/// Re-emit the current progress so a newly opened page can render state.
pub async fn broadcast_index_state(app: &AppHandle, progress: IndexProgress) {
    let _ = app.emit(pipeline::INDEX_EVENT, progress);
}

/// Helper used by tests and diagnostics.
pub fn stage_label(stage: &str) -> &'static str {
    pipeline::stage_label(stage)
}

/// Timestamp used in import summaries.
pub fn imported_at() -> String {
    now_iso()
}

/// Expose the manager trait for the AppHandle-based helpers above.
pub fn app_state(app: &AppHandle) -> Option<State<'_, AppState>> {
    app.try_state::<AppState>()
}

/// Allow callers to construct an indexing context from an `Arc<AppState>`.
pub fn context_from(state: &Arc<AppState>) -> pipeline::IndexContext {
    IndexContextAlias {
        db: state.db.clone(),
        http: state.http.clone(),
        secrets: state.secrets.clone(),
        index: state.index.clone(),
    }
}

type IndexContextAlias = pipeline::IndexContext;
