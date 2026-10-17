//! Full-text search over document chunks using SQLite FTS5 (BM25).
//!
//! The FTS table stores a pre-tokenized `index_text` (see `documents::tokenize`)
//! because `unicode61` cannot segment Chinese. Queries are tokenized with the
//! same function.

use rusqlite::{params, Connection};

use crate::documents::tokenize::{match_expression, MatchMode};
use crate::error::AppResult;

/// Metadata needed to insert one chunk into the FTS index.
pub struct FtsChunk<'a> {
    pub chunk_id: &'a str,
    pub material_id: &'a str,
    pub course_id: Option<&'a str>,
    pub page_index: Option<i64>,
    pub index_text: String,
}

/// Replace the index entries for one material. Called inside the indexing
/// transaction so a failed re-index never leaves stale rows behind.
pub fn replace_material(conn: &Connection, material_id: &str, chunks: &[FtsChunk<'_>]) -> AppResult<()> {
    conn.execute("DELETE FROM chunk_fts WHERE material_id = ?1", [material_id])?;
    let mut stmt = conn.prepare(
        "INSERT INTO chunk_fts (chunk_id, material_id, course_id, page_index, index_text)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for chunk in chunks {
        stmt.execute(params![
            chunk.chunk_id,
            chunk.material_id,
            chunk.course_id,
            chunk.page_index,
            chunk.index_text
        ])?;
    }
    Ok(())
}

pub fn delete_material(conn: &Connection, material_id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM chunk_fts WHERE material_id = ?1", [material_id])?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct FtsHit {
    pub chunk_id: String,
    pub score: f64,
    pub rank: i64,
}

/// BM25 search with optional material/course scoping.
///
/// Returns `(hits, used_or_fallback)`. AND is tried first for precision; if that
/// yields nothing, OR is retried for recall.
pub fn search(
    conn: &Connection,
    query: &str,
    material_ids: &[String],
    course_ids: &[String],
    limit: usize,
) -> AppResult<(Vec<FtsHit>, bool)> {
    let Some(and_expr) = match_expression(query, MatchMode::And) else {
        return Ok((Vec::new(), false));
    };

    let hits = run_match(conn, &and_expr, material_ids, course_ids, limit)?;
    if !hits.is_empty() {
        return Ok((hits, false));
    }
    let Some(or_expr) = match_expression(query, MatchMode::Or) else {
        return Ok((Vec::new(), false));
    };
    let hits = run_match(conn, &or_expr, material_ids, course_ids, limit)?;
    Ok((hits, true))
}

fn run_match(
    conn: &Connection,
    expression: &str,
    material_ids: &[String],
    course_ids: &[String],
    limit: usize,
) -> AppResult<Vec<FtsHit>> {
    let mut sql = String::from(
        "SELECT chunk_id, bm25(chunk_fts) AS score FROM chunk_fts WHERE chunk_fts MATCH ?1",
    );
    let mut args: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Text(expression.to_string())];

    if !material_ids.is_empty() {
        let placeholders = placeholders(material_ids.len(), args.len() + 1);
        sql.push_str(&format!(" AND material_id IN ({placeholders})"));
        for id in material_ids {
            args.push(rusqlite::types::Value::Text(id.clone()));
        }
    }
    if !course_ids.is_empty() {
        let placeholders = placeholders(course_ids.len(), args.len() + 1);
        sql.push_str(&format!(" AND course_id IN ({placeholders})"));
        for id in course_ids {
            args.push(rusqlite::types::Value::Text(id.clone()));
        }
    }
    sql.push_str(" ORDER BY score ASC LIMIT ?");
    args.push(rusqlite::types::Value::Integer(limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, f64>(1)?,
        ))
    })?;

    let mut hits = Vec::new();
    for (rank, row) in rows.enumerate() {
        let (chunk_id, bm25) = row?;
        // FTS5's bm25() is negative and smaller-is-better; flip for a
        // higher-is-better relevance score.
        hits.push(FtsHit {
            chunk_id,
            score: -bm25,
            rank: rank as i64,
        });
    }
    Ok(hits)
}

/// `?n, ?n+1, ...` for an IN clause starting at parameter `start`.
fn placeholders(count: usize, start: usize) -> String {
    (start..start + count)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    fn seed(db: &Db) {
        db.with(|c| {
            let chunks = vec![
                FtsChunk {
                    chunk_id: "c1",
                    material_id: "m1",
                    course_id: Some("k1"),
                    page_index: Some(0),
                    index_text: "导数 描述 函数 的 变化 化率".into(),
                },
                FtsChunk {
                    chunk_id: "c2",
                    material_id: "m1",
                    course_id: Some("k1"),
                    page_index: Some(1),
                    index_text: "积分 上限 下限".into(),
                },
                FtsChunk {
                    chunk_id: "c3",
                    material_id: "m2",
                    course_id: Some("k2"),
                    page_index: Some(0),
                    index_text: "derivative definition limit".into(),
                },
            ];
            replace_material(c, "m1", &chunks[0..2])?;
            replace_material(c, "m2", &chunks[2..3])?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn finds_chinese_by_bigram() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        db.with(|c| {
            let (hits, _) = search(c, "导数", &[], &[], 10)?;
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].chunk_id, "c1");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn scopes_by_material() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        db.with(|c| {
            let (hits, _) = search(c, "derivative", &["m1".to_string()], &[], 10)?;
            assert!(hits.is_empty(), "m1 has no English content");
            let (hits, _) = search(c, "derivative", &["m2".to_string()], &[], 10)?;
            assert_eq!(hits.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn scopes_by_course() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        db.with(|c| {
            let (hits, _) = search(c, "积分", &[], &["k2".to_string()], 10)?;
            assert!(hits.is_empty());
            let (hits, _) = search(c, "积分", &[], &["k1".to_string()], 10)?;
            assert_eq!(hits.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn falls_back_to_or_for_extra_terms() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        db.with(|c| {
            // No chunk has both terms: AND yields nothing, OR recovers results.
            let (hits, fallback) = search(c, "导数 积分", &[], &[], 10)?;
            assert!(!hits.is_empty());
            assert!(fallback, "expected OR fallback to be reported");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reindexing_replaces_old_rows() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        db.with(|c| {
            let replacement = vec![FtsChunk {
                chunk_id: "c9",
                material_id: "m1",
                course_id: Some("k1"),
                page_index: Some(0),
                index_text: "全新 内容".into(),
            }];
            replace_material(c, "m1", &replacement)?;
            let (hits, _) = search(c, "导数", &[], &[], 10)?;
            assert!(hits.is_empty(), "old rows must be gone");
            let (hits, _) = search(c, "全新", &[], &[], 10)?;
            assert_eq!(hits.len(), 1);
            Ok(())
        })
        .unwrap();
    }
}
