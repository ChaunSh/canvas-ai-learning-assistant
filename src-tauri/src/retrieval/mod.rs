//! Retrieval: keyword (FTS5/BM25) + optional semantic (vector) with RRF fusion.
//!
//! The semantic half is entirely optional. When no embedding model is configured
//! the system degrades to keyword search and says so (`strategy = "fts"`),
//! instead of pretending semantic search happened.

pub mod fts;
pub mod fusion;
pub mod vector;

use std::collections::HashMap;
use std::time::Instant;

use rusqlite::Connection;

use crate::domain::{SearchHit, SearchResponse};
use crate::documents::extract::{has_real_pages, has_real_slides};
use crate::error::AppResult;
use vector::VectorIndex;

pub struct RetrievalRequest<'a> {
    pub query: &'a str,
    pub material_ids: &'a [String],
    pub course_ids: &'a [String],
    pub limit: usize,
    /// Present only when an embedding provider is configured and the query was
    /// embedded successfully.
    pub query_embedding: Option<&'a [f32]>,
    pub embedding_model: Option<&'a str>,
}

pub struct RetrievalOutcome {
    pub hits: Vec<SearchHit>,
    pub strategy: String,
    pub empty_query: bool,
    pub took_ms: u64,
}

impl RetrievalOutcome {
    pub fn into_response(self) -> SearchResponse {
        SearchResponse {
            hits: self.hits,
            strategy: self.strategy,
            took_ms: self.took_ms,
            empty_query: self.empty_query,
        }
    }
}

/// Run hybrid retrieval and hydrate full chunk/material metadata.
pub fn search(
    conn: &Connection,
    request: RetrievalRequest<'_>,
    index: &dyn VectorIndex,
) -> AppResult<RetrievalOutcome> {
    let started = Instant::now();
    let query = request.query.trim();
    if query.is_empty() {
        return Ok(RetrievalOutcome {
            hits: Vec::new(),
            strategy: "fts".into(),
            empty_query: true,
            took_ms: 0,
        });
    }

    // Fetch extra candidates so fusion has room to reorder.
    let candidate_limit = (request.limit * 4).clamp(20, 200);

    let (fts_hits, _used_or) = fts::search(
        conn,
        query,
        request.material_ids,
        request.course_ids,
        candidate_limit,
    )?;

    let mut strategy = "fts".to_string();
    let mut vector_hits: Vec<vector::VectorHit> = Vec::new();

    if let (Some(embedding), Some(model)) = (request.query_embedding, request.embedding_model) {
        let allowed: Option<Vec<String>> = if request.material_ids.is_empty() {
            None
        } else {
            Some(request.material_ids.to_vec())
        };
        match index.search(conn, model, embedding, allowed.as_deref(), candidate_limit) {
            Ok(hits) if !hits.is_empty() => {
                vector_hits = hits;
                strategy = if fts_hits.is_empty() {
                    "vector".to_string()
                } else {
                    "hybrid".to_string()
                };
            }
            Ok(_) => {
                tracing::debug!("vector index returned no candidates; using keyword only");
            }
            Err(e) => {
                // A vector failure must never take down keyword search.
                tracing::warn!(error = %e, "vector search failed, falling back to keyword");
            }
        }
    }

    let fts_order: Vec<String> = fts_hits.iter().map(|h| h.chunk_id.clone()).collect();
    let vector_order: Vec<String> = vector_hits.iter().map(|h| h.chunk_id.clone()).collect();

    let fused = match (fts_order.is_empty(), vector_order.is_empty()) {
        (true, true) => Vec::new(),
        (_, true) => fusion::normalize_scores(
            &fts_order.iter().cloned().map(|id| (id, 0.0)).collect::<Vec<_>>(),
        )
        .into_iter()
        .enumerate()
        .map(|(rank, (id, _))| (id, 1.0 / (rank as f64 + 1.0)))
        .collect(),
        (true, _) => vector_order
            .iter()
            .enumerate()
            .map(|(rank, id)| (id.clone(), 1.0 / (rank as f64 + 1.0)))
            .collect(),
        _ => fusion::reciprocal_rank_fusion(&[fts_order, vector_order], 60.0),
    };

    if fused.is_empty() {
        return Ok(RetrievalOutcome {
            hits: Vec::new(),
            strategy,
            empty_query: false,
            took_ms: started.elapsed().as_millis() as u64,
        });
    }

    let normalized = fusion::normalize_scores(&fused);
    let mut hits = hydrate(conn, &normalized, &strategy)?;

    // Diversify: cap per-material contributions so one long document cannot
    // fill the whole context window.
    let per_material_cap = (request.limit / 2).max(2);
    let mut counts: HashMap<String, usize> = HashMap::new();
    hits.retain(|hit| {
        let entry = counts.entry(hit.material_id.clone()).or_insert(0);
        if *entry >= per_material_cap {
            return false;
        }
        *entry += 1;
        true
    });
    hits.truncate(request.limit);

    Ok(RetrievalOutcome {
        hits,
        strategy,
        empty_query: false,
        took_ms: started.elapsed().as_millis() as u64,
    })
}

/// Load chunk + material + course metadata for fused chunk ids, preserving rank.
fn hydrate(
    conn: &Connection,
    fused: &[(String, f64)],
    strategy: &str,
) -> AppResult<Vec<SearchHit>> {
    let mut stmt = conn.prepare(
        "SELECT ch.id, ch.material_id, m.title, m.filename, m.course_id, c.name, m.kind,
                ch.page_index, ch.kind, ch.section, ch.content, ch.context_prefix
           FROM document_chunks ch
           JOIN materials m ON m.id = ch.material_id
           LEFT JOIN courses c ON c.id = m.course_id
          WHERE ch.id = ?1",
    )?;

    let mut out = Vec::new();
    for (chunk_id, score) in fused {
        let row = stmt.query_row([chunk_id], |row| {
            let material_id: String = row.get(1)?;
            let material_title: String = row.get(2)?;
            let filename: String = row.get(3)?;
            let course_id: Option<String> = row.get(4)?;
            let course_name: Option<String> = row.get(5)?;
            let kind: String = row.get(6)?;
            let page_index: Option<i64> = row.get(7)?;
            let chunk_kind: String = row.get(8)?;
            let section: Option<String> = row.get(9)?;
            let content: String = row.get(10)?;
            let context_prefix: Option<String> = row.get(11)?;
            Ok((
                material_id,
                material_title,
                filename,
                course_id,
                course_name,
                kind,
                page_index,
                chunk_kind,
                section,
                content,
                context_prefix,
            ))
        });
        let Ok((
            material_id,
            material_title,
            filename,
            course_id,
            course_name,
            kind,
            page_index,
            chunk_kind,
            section,
            content,
            context_prefix,
        )) = row
        else {
            continue;
        };

        // Every chunk carries a 1-based page so a citation can always jump
        // somewhere. `page_kind` is what keeps us honest: a real PDF page, a
        // slide, or a page the app itself laid out for an unpaginated file.
        let page = page_index.map(|i| i + 1);
        let page_kind = page.map(|_| {
            if has_real_pages(&kind) {
                "native"
            } else if has_real_slides(&kind) {
                "slide"
            } else {
                "generated"
            }
        });
        let slide = if has_real_slides(&kind) {
            page_index.map(|i| i + 1)
        } else {
            None
        };

        out.push(SearchHit {
            chunk_id: chunk_id.clone(),
            material_id,
            material_title,
            filename,
            course_id,
            course_name,
            kind: chunk_kind,
            page_index,
            page,
            page_kind: page_kind.map(|k| k.to_string()),
            slide,
            section,
            content,
            context_prefix,
            score: *score,
            matched_by: strategy.to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    fn seed_corpus(db: &Db) {
        db.with(|c| {
            c.execute_batch(
                "INSERT INTO courses (id, name, source, sync_status, archived, position, created_at, updated_at)
                   VALUES ('k1','微积分','manual','local',0,1,'t','t');
                 INSERT INTO materials (id, course_id, title, filename, kind, sha256, storage_path, source, index_status, created_at, updated_at)
                   VALUES ('m1','k1','第一讲 导数','lecture1.pdf','pdf','h1','p1','import','indexed','t','t'),
                          ('m2','k1','讲义笔记','notes.docx','docx','h2','p2','import','indexed','t','t');",
            )?;
            c.execute(
                "INSERT INTO document_chunks (id, material_id, page_index, ordinal, kind, section, content, token_estimate, start_char, end_char, created_at)
                 VALUES ('c1','m1',2,0,'paragraph','1.1 定义','导数是函数的变化率。',10,0,20,'t'),
                        ('c2','m1',4,1,'paragraph','1.2 性质','可导必连续。',8,20,40,'t'),
                        ('c3','m2',0,0,'paragraph','笔记','导数笔记：极限定义。',9,0,20,'t')",
                [],
            )?;
            crate::retrieval::fts::replace_material(
                c,
                "m1",
                &[
                    crate::retrieval::fts::FtsChunk {
                        chunk_id: "c1",
                        material_id: "m1",
                        course_id: Some("k1"),
                        page_index: Some(2),
                        index_text: "导数 是 函数 的 变化 化率".into(),
                    },
                    crate::retrieval::fts::FtsChunk {
                        chunk_id: "c2",
                        material_id: "m1",
                        course_id: Some("k1"),
                        page_index: Some(4),
                        index_text: "可导 必 连续".into(),
                    },
                ],
            )?;
            crate::retrieval::fts::replace_material(
                c,
                "m2",
                &[crate::retrieval::fts::FtsChunk {
                    chunk_id: "c3",
                    material_id: "m2",
                    course_id: Some("k1"),
                    page_index: Some(0),
                    index_text: "导数 笔记 极限 定义".into(),
                }],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn every_hit_carries_a_jumpable_page_with_honest_provenance() {
        let db = Db::open_in_memory().unwrap();
        seed_corpus(&db);
        let index = vector::SqliteBlobVectorIndex::new();
        db.with(|c| {
            let outcome = search(
                c,
                RetrievalRequest {
                    query: "导数",
                    material_ids: &[],
                    course_ids: &[],
                    limit: 10,
                    query_embedding: None,
                    embedding_model: None,
                },
                &index,
            )?;
            assert_eq!(outcome.strategy, "fts");

            // PDF: the document's own pagination, so the page is "native".
            let pdf_hit = outcome.hits.iter().find(|h| h.material_id == "m1").unwrap();
            assert_eq!(pdf_hit.page, Some(3), "page index 2 -> page 3");
            assert_eq!(pdf_hit.page_kind.as_deref(), Some("native"));

            // DOCX: still jumpable, but the page is one the app laid out, and the
            // provenance says so instead of pretending it is the file's page.
            let docx_hit = outcome.hits.iter().find(|h| h.material_id == "m2").unwrap();
            assert_eq!(docx_hit.page, Some(1), "page index 0 -> page 1");
            assert_eq!(docx_hit.page_kind.as_deref(), Some("generated"));
            assert!(docx_hit.slide.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn empty_query_is_reported() {
        let db = Db::open_in_memory().unwrap();
        let index = vector::SqliteBlobVectorIndex::new();
        db.with(|c| {
            let outcome = search(
                c,
                RetrievalRequest {
                    query: "   ",
                    material_ids: &[],
                    course_ids: &[],
                    limit: 5,
                    query_embedding: None,
                    embedding_model: None,
                },
                &index,
            )?;
            assert!(outcome.empty_query);
            assert!(outcome.hits.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn material_scope_is_respected() {
        let db = Db::open_in_memory().unwrap();
        seed_corpus(&db);
        let index = vector::SqliteBlobVectorIndex::new();
        db.with(|c| {
            let outcome = search(
                c,
                RetrievalRequest {
                    query: "导数",
                    material_ids: &["m2".to_string()],
                    course_ids: &[],
                    limit: 10,
                    query_embedding: None,
                    embedding_model: None,
                },
                &index,
            )?;
            assert!(outcome.hits.iter().all(|h| h.material_id == "m2"));
            Ok(())
        })
        .unwrap();
    }
}
