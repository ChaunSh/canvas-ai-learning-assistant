//! Indexing pipeline: extract → chunk → persist → full-text index → embed.
//!
//! Runs off the UI thread and reports progress through the `materials://index`
//! event. A failure marks the material `failed` with a message but never deletes
//! the file or an existing good index (task specification §12 / §23).

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use crate::ai::openai::OpenAiClient;
use crate::domain::IndexProgress;
use crate::documents::{chunker, extract, tokenize};
use crate::error::{AppError, AppResult};
use crate::retrieval::vector::VectorIndex;
use crate::security::SecretStore;
use crate::storage::{ai_repo, document_repo, Db};
use crate::time::now_iso;

pub const INDEX_EVENT: &str = "materials://index";

pub struct IndexContext {
    pub db: Db,
    pub http: reqwest::Client,
    pub secrets: Arc<dyn SecretStore>,
    pub index: Arc<dyn VectorIndex>,
}

struct Progress<'a> {
    app: &'a AppHandle,
    job_id: String,
    material_id: String,
}

impl<'a> Progress<'a> {
    fn emit(&self, stage: &str, progress: f64, message: Option<&str>, pages: i64, chunks: i64) {
        let _ = self.app.emit(
            INDEX_EVENT,
            IndexProgress {
                job_id: self.job_id.clone(),
                material_id: self.material_id.clone(),
                stage: stage.to_string(),
                progress,
                message: message.map(|m| m.to_string()),
                pages,
                chunks,
            },
        );
    }
}

/// Extract, chunk and index one material.
pub async fn index_material(
    app: &AppHandle,
    ctx: &IndexContext,
    material_id: &str,
    language: &str,
) -> AppResult<()> {
    let material = ctx.db.with(|c| document_repo::get(c, material_id))?;
    let job_id = ctx.db.with(|c| {
        document_repo::set_index_status(c, material_id, "processing", None)?;
        document_repo::start_job(c, material_id)
    })?;
    let progress = Progress {
        app,
        job_id: job_id.clone(),
        material_id: material_id.to_string(),
    };
    progress.emit("extracting", 0.05, Some("正在解析文件…"), 0, 0);

    let path = std::path::PathBuf::from(&material.storage_path);
    let kind = material.kind.clone();
    let extraction_result = {
        let path = path.clone();
        tokio::task::spawn_blocking(move || extract::extract(&path, &kind)).await
    };

    let extraction = match extraction_result {
        Ok(Ok(extraction)) => extraction,
        Ok(Err(error)) => {
            fail(ctx, &progress, material_id, &error)?;
            return Err(error);
        }
        Err(join_error) => {
            let error = AppError::Message(format!("解析线程异常：{join_error}"));
            fail(ctx, &progress, material_id, &error)?;
            return Err(error);
        }
    };

    let page_count = extraction.pages.len() as i64;
    progress.emit(
        "chunking",
        0.4,
        Some(&format!("已解析 {page_count} 页，正在分块…")),
        page_count,
        0,
    );

    let options = chunker::ChunkOptions::default();
    let chunks = chunker::chunk_pages(&extraction.pages, &options);

    if chunks.is_empty() && extraction.char_count > 0 {
        let error = AppError::invalid("文本提取成功但未能生成任何可检索片段");
        fail(ctx, &progress, material_id, &error)?;
        return Err(error);
    }

    progress.emit("indexing", 0.6, Some("正在建立全文索引…"), page_count, chunks.len() as i64);

    // Persist pages, chunks and the FTS rows in one transaction so a crash can
    // never leave a half-indexed document.
    let chunk_ids = {
        let pages: Vec<(i64, String, Option<f64>, Option<f64>, bool)> = extraction
            .pages
            .iter()
            .map(|p| (p.index, p.text.clone(), p.width, p.height, p.has_image))
            .collect();
        ctx.db.transaction(|tx| {
            document_repo::replace_pages(tx, material_id, &pages)?;
            let seeds: Vec<document_repo::ChunkSeed> = chunks
                .iter()
                .map(|chunk| document_repo::ChunkSeed {
                    page_index: Some(chunk.page_index),
                    ordinal: chunk.ordinal,
                    kind: chunk.kind.as_str().to_string(),
                    section: chunk.section.clone(),
                    content: chunk.content.clone(),
                    context_prefix: chunk.context_prefix.clone(),
                    token_estimate: chunk.token_estimate,
                    start_char: chunk.start_char,
                    end_char: chunk.end_char,
                })
                .collect();
            let ids = document_repo::replace_chunks(tx, material_id, &seeds)?;

            let fts_rows: Vec<crate::retrieval::fts::FtsChunk<'_>> = chunks
                .iter()
                .zip(ids.iter())
                .map(|(chunk, id)| crate::retrieval::fts::FtsChunk {
                    chunk_id: id,
                    material_id,
                    course_id: material.course_id.as_deref(),
                    page_index: Some(chunk.page_index),
                    index_text: tokenize::index_text(&chunk.content),
                })
                .collect();
            crate::retrieval::fts::replace_material(tx, material_id, &fts_rows)?;

            document_repo::set_index_counts(
                tx,
                material_id,
                &document_repo::IndexCounts {
                    pages: if extraction.page_count > 0 {
                        extraction.page_count
                    } else {
                        extraction.slide_count
                    },
                    slides: extraction.slide_count,
                    chars: extraction.char_count,
                },
            )?;
            document_repo::invalidate_summaries(tx, material_id)?;
            Ok(ids)
        })?
    };

    // --- optional embeddings ----------------------------------------------
    let mut embedding_note: Option<String> = None;
    let embedding = resolve_embedding_provider(ctx).await;
    match embedding {
        Some((provider_id, base_url, api_key, model, timeout)) => {
            progress.emit(
                "embedding",
                0.75,
                Some("正在生成语义向量…"),
                page_count,
                chunks.len() as i64,
            );
            let client = OpenAiClient::new(&ctx.http, &base_url, &api_key, timeout);
            let mut done = 0usize;
            for (batch_start, batch) in chunks.chunks(32).enumerate() {
                let ids: Vec<String> = chunk_ids[batch_start * 32..batch_start * 32 + batch.len()].to_vec();
                let inputs: Vec<String> = batch.iter().map(|c| c.content.clone()).collect();
                match client.embed(&model, &inputs).await {
                    Ok(vectors) if vectors.len() == ids.len() => {
                        if let Err(error) = ctx.db.transaction(|tx| {
                            for (chunk_id, vector) in ids.iter().zip(vectors.iter()) {
                                ctx.index.upsert(tx, chunk_id, material_id, &model, vector)?;
                            }
                            Ok(())
                        }) {
                            embedding_note = Some(format!("向量写入失败：{error}"));
                            break;
                        }
                        done += ids.len();
                        let ratio = 0.75 + 0.2 * (done as f64 / chunks.len().max(1) as f64);
                        progress.emit(
                            "embedding",
                            ratio,
                            Some(&format!("已生成 {done}/{} 条向量", chunks.len())),
                            page_count,
                            chunks.len() as i64,
                        );
                    }
                    Ok(_) => {
                        embedding_note = Some("向量服务返回数量不匹配，已跳过语义索引".into());
                        break;
                    }
                    Err(error) => {
                        embedding_note = Some(format!("向量生成失败：{error}"));
                        let _ = ctx.db.with(|c| {
                            ai_repo::record_usage(
                                c,
                                Some(&provider_id),
                                Some(&model),
                                "embedding",
                                None,
                                None,
                                None,
                                false,
                                Some(&error.to_string()),
                            )
                        });
                        break;
                    }
                }
            }
        }
        None => {
            embedding_note = Some("未配置向量模型：仅建立全文索引（关键词检索）".into());
        }
    }
    let _ = language;

    let status = if extraction.needs_ocr && extraction.char_count == 0 {
        "partial"
    } else {
        "indexed"
    };
    ctx.db.with(|c| {
        document_repo::set_index_status(c, material_id, status, embedding_note.as_deref())?;
        document_repo::set_needs_ocr(c, material_id, extraction.needs_ocr)?;
        if let Err(error) = crate::storage::course_repo::refresh_module_counts(
            c,
            material.course_id.as_deref().unwrap_or(""),
        ) {
            tracing::debug!(error = %error, "module count refresh skipped");
        }
        document_repo::update_job(c, &job_id, "completed", "completed", 1.0, None)
    })?;
    progress.emit(
        "completed",
        1.0,
        Some(if extraction.needs_ocr {
            "索引完成（部分页面为图像，需视觉模型补充）"
        } else {
            "索引完成"
        }),
        page_count,
        chunks.len() as i64,
    );
    tracing::info!(
        material = %material_id,
        pages = page_count,
        chunks = chunks.len(),
        "indexing finished"
    );
    Ok(())
}

fn fail(
    ctx: &IndexContext,
    progress: &Progress<'_>,
    material_id: &str,
    error: &AppError,
) -> AppResult<()> {
    ctx.db.with(|c| {
        document_repo::set_index_status(c, material_id, "failed", Some(&error.to_string()))?;
        document_repo::update_job(
            c,
            &progress.job_id,
            "failed",
            "failed",
            0.0,
            Some(&error.to_string()),
        )
    })?;
    progress.emit("failed", 0.0, Some(&error.to_string()), 0, 0);
    Ok(())
}

async fn resolve_embedding_provider(ctx: &IndexContext) -> Option<(String, String, String, String, i64)> {
    let provider = {
        let conn = ctx.db.lock();
        ai_repo::default_provider(&conn).ok().flatten()
    }?;
    let model = provider.embedding_model.clone().filter(|m| !m.trim().is_empty())?;
    let key = ctx
        .secrets
        .get(&crate::security::secrets::provider_key(&provider.id))
        .ok()??;
    Some((
        provider.id,
        provider.base_url,
        key,
        model,
        provider.timeout_seconds,
    ))
}

/// Convenience wrapper used by the import command.
pub async fn index_after_import(
    app: &AppHandle,
    ctx: &IndexContext,
    material_ids: &[String],
    language: &str,
) -> Vec<AppResult<()>> {
    let mut out = Vec::new();
    for material_id in material_ids {
        out.push(index_material(app, ctx, material_id, language).await);
    }
    out
}

/// Remove every derived artifact for a material (called on delete).
pub fn purge_derivatives(ctx: &IndexContext, material_id: &str) -> AppResult<()> {
    ctx.db.transaction(|tx| {
        crate::retrieval::fts::delete_material(tx, material_id)?;
        ctx.index.delete_material(tx, material_id)?;
        Ok(())
    })
}

/// Stamp a job as cancelled so the UI stops showing a spinner.
pub fn mark_cancelled(db: &Db, material_id: &str) -> AppResult<()> {
    db.with(|c| {
        if let Some(job) = document_repo::latest_job(c, material_id)? {
            document_repo::update_job(
                c,
                &job.job_id,
                "cancelled",
                "cancelled",
                job.progress,
                Some("已取消"),
            )?;
        }
        document_repo::set_index_status(c, material_id, "pending", Some("已取消索引"))
    })
}

/// Re-index every material, used after a chunking configuration change.
pub async fn reindex_all(app: &AppHandle, ctx: &IndexContext, language: &str) -> Vec<(String, String)> {
    let materials = ctx
        .db
        .with(|c| {
            document_repo::list(
                c,
                &crate::domain::MaterialFilter {
                    course_id: None,
                    kind: None,
                    query: None,
                    favorites_only: false,
                    index_status: None,
                    limit: Some(1000),
                    offset: None,
                },
            )
        })
        .unwrap_or_default();
    let mut failures = Vec::new();
    for material in materials {
        if material.kind == "image" || material.kind == "zip" {
            continue;
        }
        if let Err(error) = index_material(app, ctx, &material.id, language).await {
            failures.push((material.id, error.to_string()));
        }
    }
    failures
}

/// Human label for the current stage.
pub fn stage_label(stage: &str) -> &'static str {
    match stage {
        "queued" => "排队中",
        "extracting" => "解析文件",
        "chunking" => "切分内容",
        "indexing" => "建立索引",
        "embedding" => "生成向量",
        "completed" => "已完成",
        "failed" => "失败",
        "cancelled" => "已取消",
        _ => "处理中",
    }
}

/// Timestamp helper for diagnostics.
pub fn now() -> String {
    now_iso()
}
