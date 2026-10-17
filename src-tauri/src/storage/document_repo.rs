//! Materials, extracted pages, chunks, embeddings bookkeeping and summaries.

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::domain::{
    DocumentChunk, DocumentPage, DocumentSummary, IndexProgress, Material, MaterialFilter,
    UpdateMaterialInput,
};
use crate::error::{AppError, AppResult};
use crate::storage::db::{b, i};
use crate::time::now_iso;

const MATERIAL_SELECT: &str = r#"
SELECT m.id, m.course_id, c.name, m.module_id, m.external_id, m.title, m.filename, m.mime_type,
       m.kind, m.byte_size, m.sha256, m.storage_path, m.source, m.remote_url, m.index_status,
       m.index_error, m.page_count, m.slide_count, m.char_count, m.needs_ocr, m.is_favorite,
       m.tags, m.ai_upload_allowed, m.last_opened_at, m.created_at, m.updated_at,
       (SELECT COUNT(*) FROM document_chunks ch WHERE ch.material_id = m.id) AS chunk_count
FROM materials m LEFT JOIN courses c ON c.id = m.course_id
"#;

fn map_material(row: &Row<'_>) -> rusqlite::Result<Material> {
    let tags_json: String = row.get(21)?;
    Ok(Material {
        id: row.get(0)?,
        course_id: row.get(1)?,
        course_name: row.get(2)?,
        module_id: row.get(3)?,
        external_id: row.get(4)?,
        title: row.get(5)?,
        filename: row.get(6)?,
        mime_type: row.get(7)?,
        kind: row.get(8)?,
        byte_size: row.get(9)?,
        sha256: row.get(10)?,
        storage_path: row.get(11)?,
        source: row.get(12)?,
        remote_url: row.get(13)?,
        index_status: row.get(14)?,
        index_error: row.get(15)?,
        page_count: row.get(16)?,
        slide_count: row.get(17)?,
        char_count: row.get(18)?,
        needs_ocr: b(row.get::<_, i64>(19)?),
        is_favorite: b(row.get::<_, i64>(20)?),
        tags: crate::storage::db::json_vec(&tags_json),
        ai_upload_allowed: b(row.get::<_, i64>(22)?),
        last_opened_at: row.get(23)?,
        created_at: row.get(24)?,
        updated_at: row.get(25)?,
        chunk_count: row.get(26)?,
    })
}

/// Fields required to persist a newly imported or downloaded material.
pub struct MaterialSeed {
    pub course_id: Option<String>,
    pub module_id: Option<String>,
    pub external_id: Option<String>,
    pub title: String,
    pub filename: String,
    pub mime_type: Option<String>,
    pub kind: String,
    pub byte_size: i64,
    pub sha256: String,
    pub storage_path: String,
    pub source: String,
    pub remote_url: Option<String>,
}

pub fn create(conn: &Connection, seed: &MaterialSeed) -> AppResult<Material> {
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO materials (id, course_id, module_id, external_id, title, filename, mime_type,
                                kind, byte_size, sha256, storage_path, source, remote_url,
                                index_status, page_count, slide_count, char_count, needs_ocr,
                                is_favorite, tags, ai_upload_allowed, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 'pending', 0, 0, 0, 0, 0, '[]', 1, ?14, ?14)",
        params![
            id,
            seed.course_id,
            seed.module_id,
            seed.external_id,
            seed.title,
            seed.filename,
            seed.mime_type,
            seed.kind,
            seed.byte_size,
            seed.sha256,
            seed.storage_path,
            seed.source,
            seed.remote_url,
            now
        ],
    )?;
    get(conn, &id)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Material> {
    let sql = format!("{MATERIAL_SELECT} WHERE m.id = ?1");
    conn.query_row(&sql, [id], map_material)
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("课件不存在：{id}")))
}

/// Locate an existing material with identical bytes (dedupe on import).
pub fn find_by_sha(conn: &Connection, sha256: &str) -> AppResult<Option<Material>> {
    let sql = format!("{MATERIAL_SELECT} WHERE m.sha256 = ?1 LIMIT 1");
    conn.query_row(&sql, [sha256], map_material)
        .optional()
        .map_err(AppError::from)
}

pub fn list(conn: &Connection, filter: &MaterialFilter) -> AppResult<Vec<Material>> {
    let limit = filter.limit.unwrap_or(200).min(1000) as i64;
    let offset = filter.offset.unwrap_or(0) as i64;
    let sql = format!(
        "{MATERIAL_SELECT}
         WHERE (?1 IS NULL OR m.course_id = ?1)
           AND (?2 IS NULL OR m.kind = ?2)
           AND (?3 = 0 OR m.is_favorite = 1)
           AND (?4 IS NULL OR m.index_status = ?4)
           AND (?5 IS NULL OR m.title LIKE ?5 OR m.filename LIKE ?5)
         ORDER BY m.created_at DESC
         LIMIT ?6 OFFSET ?7"
    );
    let like = filter
        .query
        .as_ref()
        .map(|q| format!("%{}%", q.trim()))
        .filter(|q| q.len() > 2);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            filter.course_id,
            filter.kind,
            if filter.favorites_only { 1 } else { 0 },
            filter.index_status,
            like,
            limit,
            offset
        ],
        map_material,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn update(conn: &Connection, id: &str, patch: UpdateMaterialInput) -> AppResult<Material> {
    get(conn, id)?;
    let tags_json = patch
        .tags
        .as_ref()
        .map(|t| serde_json::to_string(t))
        .transpose()?;
    conn.execute(
        "UPDATE materials SET
            title             = COALESCE(?2, title),
            course_id         = CASE WHEN ?3 = 1 THEN ?4 ELSE course_id END,
            module_id         = CASE WHEN ?5 = 1 THEN ?6 ELSE module_id END,
            is_favorite       = COALESCE(?7, is_favorite),
            tags              = COALESCE(?8, tags),
            ai_upload_allowed = COALESCE(?9, ai_upload_allowed),
            updated_at        = ?10
         WHERE id = ?1",
        params![
            id,
            patch.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()),
            if patch.course_id.is_some() { 1 } else { 0 },
            patch.course_id.flatten(),
            if patch.module_id.is_some() { 1 } else { 0 },
            patch.module_id.flatten(),
            patch.is_favorite.map(i),
            tags_json,
            patch.ai_upload_allowed.map(i),
            now_iso()
        ],
    )?;
    get(conn, id)
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM materials WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AppError::not_found(format!("课件不存在：{id}")));
    }
    Ok(())
}

pub fn touch_opened(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE materials SET last_opened_at = ?2 WHERE id = ?1",
        params![id, now_iso()],
    )?;
    Ok(())
}

pub fn set_index_status(
    conn: &Connection,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE materials SET index_status = ?2, index_error = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, status, error, now_iso()],
    )?;
    Ok(())
}

pub fn set_needs_ocr(conn: &Connection, id: &str, needed: bool) -> AppResult<()> {
    conn.execute(
        "UPDATE materials SET needs_ocr = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, i(needed), now_iso()],
    )?;
    Ok(())
}

pub struct IndexCounts {
    pub pages: i64,
    pub slides: i64,
    pub chars: i64,
}

pub fn set_index_counts(conn: &Connection, id: &str, counts: &IndexCounts) -> AppResult<()> {
    conn.execute(
        "UPDATE materials SET page_count = ?2, slide_count = ?3, char_count = ?4, updated_at = ?5
         WHERE id = ?1",
        params![id, counts.pages, counts.slides, counts.chars, now_iso()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

pub fn replace_pages(
    conn: &Connection,
    material_id: &str,
    pages: &[(i64, String, Option<f64>, Option<f64>, bool)],
) -> AppResult<()> {
    conn.execute("DELETE FROM document_pages WHERE material_id = ?1", [material_id])?;
    let now = now_iso();
    let mut stmt = conn.prepare(
        "INSERT INTO document_pages (id, material_id, page_index, label, width, height, text,
                                     char_count, has_image, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    for (index, text, width, height, has_image) in pages {
        stmt.execute(params![
            Uuid::new_v4().to_string(),
            material_id,
            index,
            format!("第 {} 页", index + 1),
            width,
            height,
            text,
            text.chars().count() as i64,
            i(*has_image),
            now
        ])?;
    }
    Ok(())
}

pub fn list_pages(conn: &Connection, material_id: &str) -> AppResult<Vec<DocumentPage>> {
    let mut stmt = conn.prepare(
        "SELECT id, material_id, page_index, label, width, height, text, char_count, has_image, created_at
         FROM document_pages WHERE material_id = ?1 ORDER BY page_index ASC",
    )?;
    let rows = stmt.query_map([material_id], |row| {
        Ok(DocumentPage {
            id: row.get(0)?,
            material_id: row.get(1)?,
            page_index: row.get(2)?,
            label: row.get(3)?,
            width: row.get(4)?,
            height: row.get(5)?,
            text: row.get(6)?,
            char_count: row.get(7)?,
            has_image: b(row.get::<_, i64>(8)?),
            created_at: row.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Chunks
// ---------------------------------------------------------------------------

pub struct ChunkSeed {
    pub page_index: Option<i64>,
    pub ordinal: i64,
    pub kind: String,
    pub section: Option<String>,
    pub content: String,
    pub context_prefix: Option<String>,
    pub token_estimate: i64,
    pub start_char: i64,
    pub end_char: i64,
}

/// Replace all chunks for a material and return the new chunk ids in order.
pub fn replace_chunks(
    conn: &Connection,
    material_id: &str,
    seeds: &[ChunkSeed],
) -> AppResult<Vec<String>> {
    conn.execute("DELETE FROM document_chunks WHERE material_id = ?1", [material_id])?;
    let now = now_iso();
    let mut stmt = conn.prepare(
        "INSERT INTO document_chunks (id, material_id, page_index, ordinal, kind, section, content,
                                      context_prefix, token_estimate, start_char, end_char, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;
    let mut ids = Vec::with_capacity(seeds.len());
    for seed in seeds {
        let id = Uuid::new_v4().to_string();
        stmt.execute(params![
            id,
            material_id,
            seed.page_index,
            seed.ordinal,
            seed.kind,
            seed.section,
            seed.content,
            seed.context_prefix,
            seed.token_estimate,
            seed.start_char,
            seed.end_char,
            now
        ])?;
        ids.push(id);
    }
    Ok(ids)
}

pub fn list_chunks(conn: &Connection, material_id: &str) -> AppResult<Vec<DocumentChunk>> {
    let mut stmt = conn.prepare(
        "SELECT id, material_id, page_index, ordinal, kind, section, content, context_prefix,
                token_estimate, start_char, end_char, created_at
         FROM document_chunks WHERE material_id = ?1 ORDER BY ordinal ASC",
    )?;
    let rows = stmt.query_map([material_id], |row| {
        Ok(DocumentChunk {
            id: row.get(0)?,
            material_id: row.get(1)?,
            page_index: row.get(2)?,
            ordinal: row.get(3)?,
            kind: row.get(4)?,
            section: row.get(5)?,
            content: row.get(6)?,
            context_prefix: row.get(7)?,
            token_estimate: row.get(8)?,
            start_char: row.get(9)?,
            end_char: row.get(10)?,
            created_at: row.get(11)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Chunks that still need an embedding for `model`.
pub fn chunks_missing_embedding(
    conn: &Connection,
    material_id: &str,
    model: &str,
) -> AppResult<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT ch.id, ch.content FROM document_chunks ch
          WHERE ch.material_id = ?1
            AND NOT EXISTS (
                SELECT 1 FROM chunk_embeddings e WHERE e.chunk_id = ch.id AND e.model = ?2
            )
          ORDER BY ch.ordinal ASC",
    )?;
    let rows = stmt.query_map(params![material_id, model], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Summaries
// ---------------------------------------------------------------------------

pub fn get_summary(
    conn: &Connection,
    material_id: &str,
    kind: &str,
    scope: Option<&str>,
) -> AppResult<Option<DocumentSummary>> {
    let sql = "SELECT s.id, s.material_id, m.title, s.kind, s.scope, s.content_md, s.provider_id,
                      s.model, s.source_sha256, s.stale, s.created_at, s.updated_at
               FROM document_summaries s JOIN materials m ON m.id = s.material_id
               WHERE s.material_id = ?1 AND s.kind = ?2 AND s.scope = ?3";
    conn.query_row(sql, params![material_id, kind, scope.unwrap_or("")], map_summary)
        .optional()
        .map_err(AppError::from)
}

fn map_summary(row: &Row<'_>) -> rusqlite::Result<DocumentSummary> {
    let scope: String = row.get(4)?;
    Ok(DocumentSummary {
        id: row.get(0)?,
        material_id: row.get(1)?,
        material_title: row.get(2)?,
        kind: row.get(3)?,
        scope: if scope.is_empty() { None } else { Some(scope) },
        content_md: row.get(5)?,
        provider_id: row.get(6)?,
        model: row.get(7)?,
        source_sha256: row.get(8)?,
        stale: b(row.get::<_, i64>(9)?),
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

pub fn list_summaries(conn: &Connection, material_id: &str) -> AppResult<Vec<DocumentSummary>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.material_id, m.title, s.kind, s.scope, s.content_md, s.provider_id,
                s.model, s.source_sha256, s.stale, s.created_at, s.updated_at
         FROM document_summaries s JOIN materials m ON m.id = s.material_id
         WHERE s.material_id = ?1 ORDER BY s.created_at DESC",
    )?;
    let rows = stmt.query_map([material_id], map_summary)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn upsert_summary(
    conn: &Connection,
    material_id: &str,
    kind: &str,
    scope: Option<&str>,
    content_md: &str,
    provider_id: Option<&str>,
    model: Option<&str>,
    source_sha256: Option<&str>,
) -> AppResult<DocumentSummary> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO document_summaries (id, material_id, kind, scope, content_md, provider_id,
                                         model, source_sha256, stale, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?9)
         ON CONFLICT(material_id, kind, scope) DO UPDATE SET
            content_md = excluded.content_md,
            provider_id = excluded.provider_id,
            model = excluded.model,
            source_sha256 = excluded.source_sha256,
            stale = 0,
            updated_at = excluded.updated_at",
        params![
            Uuid::new_v4().to_string(),
            material_id,
            kind,
            scope.unwrap_or(""),
            content_md,
            provider_id,
            model,
            source_sha256,
            now
        ],
    )?;
    get_summary(conn, material_id, kind, scope)?.ok_or_else(|| AppError::msg("保存总结失败"))
}

/// Mark cached summaries stale after the document bytes or index changed.
pub fn invalidate_summaries(conn: &Connection, material_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE document_summaries SET stale = 1, updated_at = ?2 WHERE material_id = ?1",
        params![material_id, now_iso()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Processing jobs
// ---------------------------------------------------------------------------

pub fn start_job(conn: &Connection, material_id: &str) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO document_processing_jobs (id, material_id, status, stage, progress, attempts, created_at, updated_at)
         VALUES (?1, ?2, 'running', 'queued', 0, 1, ?3, ?3)",
        params![id, material_id, now],
    )?;
    Ok(id)
}

pub fn update_job(
    conn: &Connection,
    job_id: &str,
    status: &str,
    stage: &str,
    progress: f64,
    error: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE document_processing_jobs SET status = ?2, stage = ?3, progress = ?4, error = ?5,
                updated_at = ?6 WHERE id = ?1",
        params![job_id, status, stage, progress.clamp(0.0, 1.0), error, now_iso()],
    )?;
    Ok(())
}

pub fn latest_job(conn: &Connection, material_id: &str) -> AppResult<Option<IndexProgress>> {
    conn.query_row(
        "SELECT id, material_id, stage, progress, error, created_at
         FROM document_processing_jobs WHERE material_id = ?1 ORDER BY created_at DESC LIMIT 1",
        [material_id],
        |row| {
            Ok(IndexProgress {
                job_id: row.get(0)?,
                material_id: row.get(1)?,
                stage: row.get::<_, String>(2)?,
                progress: row.get(3)?,
                message: row.get(4)?,
                pages: 0,
                chunks: 0,
            })
        },
    )
    .optional()
    .map_err(AppError::from)
}

// ---------------------------------------------------------------------------
// Aggregates
// ---------------------------------------------------------------------------

pub fn index_stats(conn: &Connection) -> AppResult<(i64, i64, i64)> {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM materials", [], |r| r.get(0))?;
    let indexed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM materials WHERE index_status IN ('indexed','partial')",
        [],
        |r| r.get(0),
    )?;
    let pages: i64 = conn.query_row("SELECT COUNT(*) FROM document_pages", [], |r| r.get(0))?;
    Ok((total, indexed, pages))
}

/// Total stored bytes for the cache manager.
pub fn storage_bytes(conn: &Connection) -> AppResult<i64> {
    Ok(conn.query_row("SELECT COALESCE(SUM(byte_size), 0) FROM materials", [], |r| r.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    fn seed_course(db: &Db) -> String {
        db.with(|c| {
            c.execute(
                "INSERT INTO courses (id, name, source, sync_status, archived, position, created_at, updated_at)
                 VALUES ('k1','测试课程','manual','local',0,1,'t','t')",
                [],
            )?;
            Ok("k1".to_string())
        })
        .unwrap()
    }

    fn seed_material(db: &Db, course: &str, sha: &str, title: &str) -> Material {
        db.with(|c| {
            create(
                c,
                &MaterialSeed {
                    course_id: Some(course.to_string()),
                    module_id: None,
                    external_id: None,
                    title: title.into(),
                    filename: format!("{title}.pdf"),
                    mime_type: Some("application/pdf".into()),
                    kind: "pdf".into(),
                    byte_size: 1024,
                    sha256: sha.into(),
                    storage_path: format!("/tmp/{sha}"),
                    source: "import".into(),
                    remote_url: None,
                },
            )
        })
        .unwrap()
    }

    #[test]
    fn material_crud_and_filtering() {
        let db = Db::open_in_memory().unwrap();
        let course = seed_course(&db);
        let m = seed_material(&db, &course, "abc", "第一讲");
        db.with(|c| {
            assert_eq!(get(c, &m.id)?.course_name.as_deref(), Some("测试课程"));

            let all = list(
                c,
                &MaterialFilter {
                    course_id: Some(course.clone()),
                    kind: None,
                    query: None,
                    favorites_only: false,
                    index_status: None,
                    limit: None,
                    offset: None,
                },
            )?;
            assert_eq!(all.len(), 1);

            update(
                c,
                &m.id,
                UpdateMaterialInput {
                    title: None,
                    course_id: None,
                    module_id: None,
                    is_favorite: Some(true),
                    tags: Some(vec!["exam".into()]),
                    ai_upload_allowed: Some(false),
                },
            )?;
            let favs = list(
                c,
                &MaterialFilter {
                    course_id: None,
                    kind: None,
                    query: None,
                    favorites_only: true,
                    index_status: None,
                    limit: None,
                    offset: None,
                },
            )?;
            assert_eq!(favs.len(), 1);
            assert_eq!(favs[0].tags, vec!["exam"]);
            assert!(!favs[0].ai_upload_allowed);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn dedupe_by_sha() {
        let db = Db::open_in_memory().unwrap();
        let course = seed_course(&db);
        let m = seed_material(&db, &course, "deadbeef", "讲义");
        db.with(|c| {
            let found = find_by_sha(c, "deadbeef")?.unwrap();
            assert_eq!(found.id, m.id);
            assert!(find_by_sha(c, "other")?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn pages_and_chunks_replace_atomically() {
        let db = Db::open_in_memory().unwrap();
        let course = seed_course(&db);
        let m = seed_material(&db, &course, "sha1", "讲义");
        db.with(|c| {
            replace_pages(
                c,
                &m.id,
                &[
                    (0, "第一页".to_string(), Some(595.0), Some(842.0), false),
                    (1, "第二页".to_string(), Some(595.0), Some(842.0), true),
                ],
            )?;
            let pages = list_pages(c, &m.id)?;
            assert_eq!(pages.len(), 2);
            assert!(pages[1].has_image);

            let ids = replace_chunks(
                c,
                &m.id,
                &[ChunkSeed {
                    page_index: Some(0),
                    ordinal: 0,
                    kind: "paragraph".into(),
                    section: Some("1.1".into()),
                    content: "内容".into(),
                    context_prefix: Some("第一章".into()),
                    token_estimate: 2,
                    start_char: 0,
                    end_char: 2,
                }],
            )?;
            assert_eq!(ids.len(), 1);
            let chunks = list_chunks(c, &m.id)?;
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0].section.as_deref(), Some("1.1"));

            // Replacing again must not accumulate rows.
            replace_chunks(c, &m.id, &[])?;
            assert!(list_chunks(c, &m.id)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn summaries_upsert_and_invalidate() {
        let db = Db::open_in_memory().unwrap();
        let course = seed_course(&db);
        let m = seed_material(&db, &course, "sha2", "讲义");
        db.with(|c| {
            upsert_summary(c, &m.id, "full", None, "# 摘要", None, Some("model"), Some("sha2"))?;
            let s = get_summary(c, &m.id, "full", None)?.unwrap();
            assert_eq!(s.content_md, "# 摘要");
            assert!(!s.stale);

            upsert_summary(c, &m.id, "full", None, "# 新摘要", None, Some("model"), Some("sha2"))?;
            assert_eq!(get_summary(c, &m.id, "full", None)?.unwrap().content_md, "# 新摘要");

            invalidate_summaries(c, &m.id)?;
            assert!(get_summary(c, &m.id, "full", None)?.unwrap().stale);
            Ok(())
        })
        .unwrap();
    }
}
