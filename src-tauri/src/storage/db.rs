use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::{Mutex, MutexGuard};
use rusqlite::Connection;

use crate::error::{AppError, AppResult};

use super::migrations;

/// Owned handle to the application database.
#[derive(Clone)]
pub struct Db {
    inner: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl Db {
    /// Open (creating if needed) the database at `path` and bring the schema up
    /// to date.
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)
            .map_err(|e| AppError::Db(e))?;
        configure(&conn, true)?;
        migrations::run(&mut conn)?;
        tracing::info!(path = %path.display(), "database ready");
        Ok(Self {
            inner: Arc::new(Mutex::new(conn)),
            path: path.to_path_buf(),
        })
    }

    /// Ephemeral database used by unit and integration tests.
    pub fn open_in_memory() -> AppResult<Self> {
        let mut conn = Connection::open_in_memory()?;
        configure(&conn, false)?;
        migrations::run(&mut conn)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(conn)),
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.inner.lock()
    }

    /// Run `f` with exclusive access to the connection.
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
        let guard = self.inner.lock();
        f(&guard)
    }

    /// Run `f` inside an immediate transaction, committing on success.
    pub fn transaction<T>(&self, f: impl FnOnce(&rusqlite::Transaction<'_>) -> AppResult<T>) -> AppResult<T> {
        let mut guard = self.inner.lock();
        let tx = guard.transaction()?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// Write a consistent snapshot of the database to `target`.
    ///
    /// `VACUUM INTO` produces a single self-contained file, which is what the
    /// data-export feature ships to the user.
    pub fn backup_to(&self, target: &Path) -> AppResult<u64> {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if target.exists() {
            std::fs::remove_file(target)?;
        }
        let guard = self.inner.lock();
        let escaped = target.to_string_lossy().replace('\'', "''");
        guard.execute_batch(&format!("VACUUM INTO '{escaped}'"))?;
        drop(guard);
        let size = std::fs::metadata(target)?.len();
        Ok(size)
    }

    /// Database file size in bytes (0 for in-memory databases).
    pub fn file_size(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }
}

fn configure(conn: &Connection, on_disk: bool) -> AppResult<()> {
    // Fail loudly instead of silently dropping foreign-key violations.
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    if on_disk {
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
    }
    conn.execute_batch("PRAGMA busy_timeout = 5000;")?;
    conn.execute_batch("PRAGMA temp_store = MEMORY;")?;
    Ok(())
}

/// Convert a `bool` column to Rust `bool`.
pub fn b(value: i64) -> bool {
    value != 0
}

/// Convert a Rust `bool` to a SQLite integer.
pub fn i(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

/// Parse a JSON array column, tolerating corrupt/legacy values.
pub fn json_vec(value: &str) -> Vec<String> {
    serde_json::from_str(value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_migrations_apply_cleanly() {
        let db = Db::open_in_memory().expect("in-memory db");
        let version: i64 = db
            .with(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(version, migrations::LATEST_VERSION);

        // Every table required by the specification must exist.
        let expected = [
            "courses",
            "course_modules",
            "materials",
            "material_versions",
            "assignments",
            "announcements",
            "calendar_events",
            "grade_records",
            "discussion_topics",
            "discussion_entries",
            "document_processing_jobs",
            "document_pages",
            "document_chunks",
            "chunk_embeddings",
            "document_summaries",
            "ai_providers",
            "ai_model_configs",
            "chat_sessions",
            "chat_messages",
            "chat_citations",
            "ai_usage_records",
            "study_plans",
            "study_tasks",
            "study_preferences",
            "study_progress",
            "study_sessions",
            "integration_connections",
            "integration_capabilities",
            "sync_runs",
            "sync_checkpoints",
            "sync_errors",
            "resource_mappings",
            "import_jobs",
        ];
        for table in expected {
            let count: i64 = db
                .with(|c| {
                    Ok(c.query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                        [table],
                        |r| r.get(0),
                    )?)
                })
                .unwrap();
            assert_eq!(count, 1, "missing table {table}");
        }
    }

    #[test]
    fn fts5_is_available() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            c.execute(
                "INSERT INTO chunk_fts (chunk_id, material_id, course_id, page_index, index_text)
                 VALUES ('c1','m1','k1',0,'hello world')",
                [],
            )?;
            let hits: i64 = c.query_row(
                "SELECT COUNT(*) FROM chunk_fts WHERE chunk_fts MATCH 'hello'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(hits, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn migrations_are_idempotent() {
        let db = Db::open_in_memory().unwrap();
        let mut guard = db.lock();
        migrations::run(&mut guard).unwrap();
        migrations::run(&mut guard).unwrap();
    }
}
