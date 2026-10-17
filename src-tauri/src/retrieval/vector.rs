//! Replaceable local vector index.
//!
//! Vectors are stored as little-endian `f32` blobs in `chunk_embeddings` and
//! searched exhaustively with cosine similarity. That is deliberate:
//!   * it is pure Rust, so it behaves identically on Windows and Android with no
//!     native extension to cross-compile;
//!   * student corpora are small (tens of thousands of chunks), where a linear
//!     scan costs a few milliseconds;
//!   * swapping in an ANN backend later only requires another `VectorIndex` impl.
//!
//! Vectors are L2-normalised on write, so cosine similarity is a dot product.

use rusqlite::{params, Connection};

use crate::error::{AppError, AppResult};
use crate::time::now_iso;

pub trait VectorIndex: Send + Sync {
    /// Insert or replace one chunk's embedding.
    fn upsert(
        &self,
        conn: &Connection,
        chunk_id: &str,
        material_id: &str,
        model: &str,
        vector: &[f32],
    ) -> AppResult<()>;

    /// Rank chunks by cosine similarity, restricted to the allowed materials.
    fn search(
        &self,
        conn: &Connection,
        model: &str,
        query: &[f32],
        allowed_materials: Option<&[String]>,
        limit: usize,
    ) -> AppResult<Vec<VectorHit>>;

    fn delete_material(&self, conn: &Connection, material_id: &str) -> AppResult<()>;

    fn count(&self, conn: &Connection, model: &str) -> AppResult<i64>;
}

#[derive(Debug, Clone)]
pub struct VectorHit {
    pub chunk_id: String,
    pub score: f32,
    pub rank: i64,
}

/// Brute-force cosine index backed by SQLite blobs.
#[derive(Default, Clone, Copy)]
pub struct SqliteBlobVectorIndex;

impl SqliteBlobVectorIndex {
    pub fn new() -> Self {
        Self
    }
}

impl VectorIndex for SqliteBlobVectorIndex {
    fn upsert(
        &self,
        conn: &Connection,
        chunk_id: &str,
        material_id: &str,
        model: &str,
        vector: &[f32],
    ) -> AppResult<()> {
        if vector.is_empty() {
            return Err(AppError::invalid("向量维度不能为 0"));
        }
        let normalized = normalize(vector);
        let blob = encode_f32(&normalized);
        conn.execute(
            "INSERT INTO chunk_embeddings (chunk_id, material_id, model, dim, vector, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(chunk_id) DO UPDATE SET
                material_id = excluded.material_id,
                model = excluded.model,
                dim = excluded.dim,
                vector = excluded.vector,
                created_at = excluded.created_at",
            params![
                chunk_id,
                material_id,
                model,
                normalized.len() as i64,
                blob,
                now_iso()
            ],
        )?;
        Ok(())
    }

    fn search(
        &self,
        conn: &Connection,
        model: &str,
        query: &[f32],
        allowed_materials: Option<&[String]>,
        limit: usize,
    ) -> AppResult<Vec<VectorHit>> {
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let query_norm = normalize(query);
        let mut stmt = conn.prepare(
            "SELECT chunk_id, material_id, dim, vector FROM chunk_embeddings WHERE model = ?1",
        )?;
        let rows = stmt.query_map([model], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?;

        let allowed: Option<std::collections::HashSet<&str>> = allowed_materials
            .map(|ids| ids.iter().map(|s| s.as_str()).collect());

        let mut scored: Vec<(String, f32)> = Vec::new();
        for row in rows {
            let (chunk_id, material_id, dim, blob) = row?;
            if let Some(allowed) = &allowed {
                if !allowed.contains(material_id.as_str()) {
                    continue;
                }
            }
            if dim as usize != query_norm.len() {
                // Model changed under us: skip rather than compare mismatched dims.
                continue;
            }
            let vector = decode_f32(&blob);
            if vector.len() != query_norm.len() {
                continue;
            }
            let score: f32 = vector
                .iter()
                .zip(query_norm.iter())
                .map(|(a, b)| a * b)
                .sum();
            scored.push((chunk_id, score));
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored
            .into_iter()
            .enumerate()
            .map(|(rank, (chunk_id, score))| VectorHit {
                chunk_id,
                score,
                rank: rank as i64,
            })
            .collect())
    }

    fn delete_material(&self, conn: &Connection, material_id: &str) -> AppResult<()> {
        conn.execute("DELETE FROM chunk_embeddings WHERE material_id = ?1", [material_id])?;
        Ok(())
    }

    fn count(&self, conn: &Connection, model: &str) -> AppResult<i64> {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM chunk_embeddings WHERE model = ?1",
            [model],
            |r| r.get(0),
        )?)
    }
}

fn normalize(vector: &[f32]) -> Vec<f32> {
    let norm: f32 = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return vector.to_vec();
    }
    vector.iter().map(|v| v / norm).collect()
}

fn encode_f32(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for v in values {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

fn decode_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    #[test]
    fn f32_roundtrip() {
        let original = vec![0.5f32, -0.25, 1.0, 0.0];
        let decoded = decode_f32(&encode_f32(&original));
        assert_eq!(original, decoded);
    }

    #[test]
    fn normalization_makes_unit_vectors() {
        let n = normalize(&[3.0, 4.0]);
        assert!((n[0] - 0.6).abs() < 1e-6);
        assert!((n[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn ranks_by_similarity_and_scopes() {
        let db = Db::open_in_memory().unwrap();
        let index = SqliteBlobVectorIndex::new();
        db.with(|c| {
            // Two materials, two chunks each, plus a chunk row so FK holds.
            c.execute_batch(
                "INSERT INTO courses (id, name, source, sync_status, archived, position, created_at, updated_at)
                   VALUES ('k1','C1','manual','local',0,1,'t','t');
                 INSERT INTO materials (id, course_id, title, filename, kind, sha256, storage_path, source, created_at, updated_at)
                   VALUES ('m1','k1','M1','m1.pdf','pdf','h1','p1','manual','t','t'),
                          ('m2','k1','M2','m2.pdf','pdf','h2','p2','manual','t','t');",
            )?;
            for (ordinal, (chunk, material)) in
                [("a", "m1"), ("b", "m1"), ("c", "m2")].iter().enumerate()
            {
                c.execute(
                    "INSERT INTO document_chunks (id, material_id, page_index, ordinal, kind, content, token_estimate, start_char, end_char, created_at)
                     VALUES (?1, ?2, 0, ?3, 'paragraph', 'x', 1, 0, 1, 't')",
                    params![chunk, material, ordinal as i64],
                )?;
            }
            index.upsert(c, "a", "m1", "emb", &[1.0, 0.0])?;
            index.upsert(c, "b", "m1", "emb", &[0.0, 1.0])?;
            index.upsert(c, "c", "m2", "emb", &[0.9, 0.1])?;

            let hits = index.search(c, "emb", &[1.0, 0.0], None, 10)?;
            assert_eq!(hits[0].chunk_id, "a");
            assert!((hits[0].score - 1.0).abs() < 1e-6);

            let hits = index.search(c, "emb", &[1.0, 0.0], Some(&["m2".to_string()]), 10)?;
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].chunk_id, "c");

            assert_eq!(index.count(c, "emb")?, 3);
            index.delete_material(c, "m1")?;
            assert_eq!(index.count(c, "emb")?, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn mismatched_dimensions_are_skipped() {
        let db = Db::open_in_memory().unwrap();
        let index = SqliteBlobVectorIndex::new();
        db.with(|c| {
            c.execute_batch(
                "INSERT INTO materials (id, title, filename, kind, sha256, storage_path, source, created_at, updated_at)
                   VALUES ('m1','M1','m1.pdf','pdf','h1','p1','manual','t','t');",
            )?;
            c.execute(
                "INSERT INTO document_chunks (id, material_id, page_index, ordinal, kind, content, token_estimate, start_char, end_char, created_at)
                 VALUES ('a','m1',0,0,'paragraph','x',1,0,1,'t')",
                [],
            )?;
            index.upsert(c, "a", "m1", "emb", &[1.0, 0.0, 0.0])?;
            let hits = index.search(c, "emb", &[1.0, 0.0], None, 10)?;
            assert!(hits.is_empty());
            Ok(())
        })
        .unwrap();
    }
}
