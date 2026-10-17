//! Canvas integration bookkeeping: connection state, capability matrix, sync runs
//! and import jobs.
//!
//! Nothing in this module stores session material. `user_ref` is a non-secret
//! display hint; cookies, CSRF tokens and SSO material are never persisted
//! (task specification §11 / §50).

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;
use uuid::Uuid;

use crate::domain::{
    Capability, CapabilityStatus, CanvasConnection, CanvasSessionState, SyncRun, SyncStats,
};
use crate::error::{AppError, AppResult};
use crate::time::now_iso;

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

fn map_connection(row: &Row<'_>, capabilities: Vec<Capability>) -> CanvasConnection {
    CanvasConnection {
        id: row.get::<_, String>(0).unwrap_or_default(),
        base_url: row.get::<_, String>(1).unwrap_or_default(),
        state: CanvasSessionState::from_str(&row.get::<_, String>(2).unwrap_or_default()),
        display_name: row.get(3).unwrap_or(None),
        user_ref: row.get(4).unwrap_or(None),
        last_checked_at: row.get(5).unwrap_or(None),
        last_verified_at: row.get(6).unwrap_or(None),
        capabilities,
        diagnostics: Vec::new(),
    }
}

/// Fetch the single Canvas connection row, creating it on first use.
pub fn ensure_connection(conn: &Connection, base_url: &str) -> AppResult<CanvasConnection> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM integration_connections WHERE provider = 'canvas' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;

    let id = match existing {
        Some(id) => {
            conn.execute(
                "UPDATE integration_connections SET base_url = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, base_url, now_iso()],
            )?;
            id
        }
        None => {
            let new_id = Uuid::new_v4().to_string();
            let now = now_iso();
            conn.execute(
                "INSERT INTO integration_connections (id, provider, base_url, state, created_at, updated_at)
                 VALUES (?1, 'canvas', ?2, 'not-connected', ?3, ?3)",
                params![new_id, base_url, now],
            )?;
            for key in crate::domain::CAPABILITY_KEYS {
                conn.execute(
                    "INSERT INTO integration_capabilities (id, connection_id, key, status, checked_at)
                     VALUES (?1, ?2, ?3, 'not-tested', NULL)",
                    params![Uuid::new_v4().to_string(), new_id, key],
                )?;
            }
            new_id
        }
    };
    get_connection(conn, &id)
}

pub fn get_connection(conn: &Connection, id: &str) -> AppResult<CanvasConnection> {
    let capabilities = list_capabilities(conn, id)?;
    conn.query_row(
        "SELECT id, base_url, state, display_name, user_ref, last_checked_at, last_verified_at
         FROM integration_connections WHERE id = ?1",
        [id],
        |row| Ok(map_connection(row, capabilities.clone())),
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("Canvas 连接尚未初始化"))
}

pub fn set_state(conn: &Connection, id: &str, state: CanvasSessionState) -> AppResult<()> {
    let now = now_iso();
    conn.execute(
        "UPDATE integration_connections SET state = ?2, last_checked_at = ?3, updated_at = ?3
         WHERE id = ?1",
        params![id, state.as_str(), now],
    )?;
    // Verification time only advances on a genuinely verified state.
    if matches!(state, CanvasSessionState::DataAccessVerified | CanvasSessionState::WebAuthenticated) {
        conn.execute(
            "UPDATE integration_connections SET last_verified_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
    }
    Ok(())
}

pub fn set_identity(
    conn: &Connection,
    id: &str,
    display_name: Option<&str>,
    user_ref: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE integration_connections SET display_name = ?2, user_ref = ?3, updated_at = ?4
         WHERE id = ?1",
        params![id, display_name, user_ref, now_iso()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

pub fn list_capabilities(conn: &Connection, connection_id: &str) -> AppResult<Vec<Capability>> {
    let mut stmt = conn.prepare(
        "SELECT key, status, detail, checked_at FROM integration_capabilities
         WHERE connection_id = ?1 ORDER BY rowid ASC",
    )?;
    let rows = stmt.query_map([connection_id], |row| {
        Ok(Capability {
            key: row.get(0)?,
            status: CapabilityStatus::from_str(&row.get::<_, String>(1)?),
            detail: row.get(2)?,
            checked_at: row.get(3)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn set_capability(
    conn: &Connection,
    connection_id: &str,
    key: &str,
    status: CapabilityStatus,
    detail: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO integration_capabilities (id, connection_id, key, status, detail, checked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(connection_id, key) DO UPDATE SET
            status = excluded.status, detail = excluded.detail, checked_at = excluded.checked_at",
        params![
            Uuid::new_v4().to_string(),
            connection_id,
            key,
            status.as_str(),
            detail,
            now_iso()
        ],
    )?;
    Ok(())
}

pub fn capabilities_map(
    conn: &Connection,
    connection_id: &str,
) -> AppResult<std::collections::HashMap<String, CapabilityStatus>> {
    Ok(list_capabilities(conn, connection_id)?
        .into_iter()
        .map(|c| (c.key, c.status))
        .collect())
}

// ---------------------------------------------------------------------------
// Sync runs
// ---------------------------------------------------------------------------

fn map_run(row: &Row<'_>) -> rusqlite::Result<SyncRun> {
    let stats_json: String = row.get(4)?;
    Ok(SyncRun {
        id: row.get(0)?,
        status: row.get(1)?,
        scope: row.get(2)?,
        course_id: row.get(3)?,
        stats: serde_json::from_str(&stats_json).unwrap_or_default(),
        error: row.get(5)?,
        started_at: row.get(6)?,
        finished_at: row.get(7)?,
        created_at: row.get(8)?,
    })
}

const RUN_SELECT: &str = "SELECT id, status, scope, course_id, stats, error, started_at, finished_at, created_at FROM sync_runs";

pub fn create_sync_run(
    conn: &Connection,
    connection_id: &str,
    scope: &str,
    course_id: Option<&str>,
) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO sync_runs (id, connection_id, scope, course_id, status, stats, started_at, created_at)
         VALUES (?1, ?2, ?3, ?4, 'running', '{}', ?5, ?5)",
        params![id, connection_id, scope, course_id, now],
    )?;
    Ok(id)
}

pub fn finish_sync_run(
    conn: &Connection,
    id: &str,
    status: &str,
    stats: &SyncStats,
    error: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE sync_runs SET status = ?2, stats = ?3, error = ?4, finished_at = ?5 WHERE id = ?1",
        params![
            id,
            status,
            serde_json::to_string(stats)?,
            error,
            now_iso()
        ],
    )?;
    Ok(())
}

pub fn get_sync_run(conn: &Connection, id: &str) -> AppResult<SyncRun> {
    let sql = format!("{RUN_SELECT} WHERE id = ?1");
    conn.query_row(&sql, [id], map_run)
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("同步记录不存在：{id}")))
}

pub fn list_sync_runs(conn: &Connection, limit: i64) -> AppResult<Vec<SyncRun>> {
    let sql = format!("{RUN_SELECT} ORDER BY created_at DESC LIMIT ?1");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit], map_run)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn add_sync_error(
    conn: &Connection,
    run_id: &str,
    resource: Option<&str>,
    message: &str,
    retryable: bool,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO sync_errors (id, sync_run_id, resource, message, retryable, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            Uuid::new_v4().to_string(),
            run_id,
            resource,
            message,
            crate::storage::db::i(retryable),
            now_iso()
        ],
    )?;
    Ok(())
}

pub fn set_checkpoint(conn: &Connection, run_id: &str, resource: &str, cursor: Option<&str>) -> AppResult<()> {
    conn.execute(
        "INSERT INTO sync_checkpoints (id, sync_run_id, resource, cursor, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(sync_run_id, resource) DO UPDATE SET cursor = excluded.cursor, updated_at = excluded.updated_at",
        params![Uuid::new_v4().to_string(), run_id, resource, cursor, now_iso()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Resource mappings
// ---------------------------------------------------------------------------

/// Remember which local row corresponds to a remote resource.
pub fn upsert_mapping(
    conn: &Connection,
    local_type: &str,
    local_id: &str,
    remote_type: &str,
    remote_id: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO resource_mappings (id, local_type, local_id, remote_type, remote_id, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(local_type, local_id, remote_type, remote_id) DO UPDATE SET updated_at = excluded.updated_at",
        params![
            Uuid::new_v4().to_string(),
            local_type,
            local_id,
            remote_type,
            remote_id,
            now_iso()
        ],
    )?;
    Ok(())
}

/// Whether a remote file has already been downloaded.
pub fn find_material_by_remote(
    conn: &Connection,
    remote_type: &str,
    remote_id: &str,
) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT local_id FROM resource_mappings
             WHERE remote_type = ?1 AND remote_id = ?2 AND local_type = 'material' LIMIT 1",
            params![remote_type, remote_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?)
}

// ---------------------------------------------------------------------------
// Import jobs
// ---------------------------------------------------------------------------

pub fn create_import_job(conn: &Connection, kind: &str, source_path: Option<&str>) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO import_jobs (id, kind, status, source_path, progress, total, processed, created_at, updated_at)
         VALUES (?1, ?2, 'running', ?3, 0, 0, 0, ?4, ?4)",
        params![id, kind, source_path, now],
    )?;
    Ok(id)
}

pub fn update_import_job(
    conn: &Connection,
    id: &str,
    status: &str,
    processed: i64,
    total: i64,
    message: Option<&str>,
) -> AppResult<()> {
    let progress = if total > 0 {
        (processed as f64 / total as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    conn.execute(
        "UPDATE import_jobs SET status = ?2, processed = ?3, total = ?4, progress = ?5,
                message = ?6, updated_at = ?7 WHERE id = ?1",
        params![id, status, processed, total, progress, message, now_iso()],
    )?;
    Ok(())
}

pub fn list_import_jobs(conn: &Connection, limit: i64) -> AppResult<Vec<(String, String, i64, i64, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT id, status, processed, total, message FROM import_jobs ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Recent sync errors for the diagnostics screen.
pub fn recent_sync_errors(conn: &Connection, limit: i64) -> AppResult<Vec<(String, Option<String>, String, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT sync_run_id, resource, message, retryable FROM sync_errors ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            crate::storage::db::b(row.get::<_, i64>(3)?),
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Structured JSON snapshot for the "导出同步日志" action.
pub fn sync_log_export(conn: &Connection) -> AppResult<Value> {
    let runs = list_sync_runs(conn, 100)?;
    let errors = recent_sync_errors(conn, 200)?;
    Ok(serde_json::json!({
        "exportedAt": now_iso(),
        "runs": runs,
        "errors": errors.into_iter().map(|(run, resource, message, retryable)| serde_json::json!({
            "syncRunId": run,
            "resource": resource,
            "message": message,
            "retryable": retryable,
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    #[test]
    fn connection_initialises_all_capabilities_as_not_tested() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let conn = ensure_connection(c, "https://hkust-gz.instructure.com")?;
            assert_eq!(conn.state, CanvasSessionState::NotConnected);
            assert_eq!(conn.capabilities.len(), crate::domain::CAPABILITY_KEYS.len());
            assert!(conn
                .capabilities
                .iter()
                .all(|cap| cap.status == CapabilityStatus::NotTested));

            // Re-running must not duplicate rows.
            let again = ensure_connection(c, "https://hkust-gz.instructure.com")?;
            assert_eq!(again.id, conn.id);
            assert_eq!(again.capabilities.len(), crate::domain::CAPABILITY_KEYS.len());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn capability_updates_are_upserts() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let conn = ensure_connection(c, "https://example.test")?;
            set_capability(c, &conn.id, "courses", CapabilityStatus::Supported, Some("42 门课程"))?;
            set_capability(c, &conn.id, "grades", CapabilityStatus::PermissionDenied, Some("403"))?;
            let map = capabilities_map(c, &conn.id)?;
            assert_eq!(map.get("courses"), Some(&CapabilityStatus::Supported));
            assert_eq!(map.get("grades"), Some(&CapabilityStatus::PermissionDenied));
            assert_eq!(map.get("files"), Some(&CapabilityStatus::NotTested));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn verified_state_records_timestamp() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let conn = ensure_connection(c, "https://example.test")?;
            set_state(c, &conn.id, CanvasSessionState::WebAuthenticated)?;
            let after_web = get_connection(c, &conn.id)?;
            assert!(after_web.last_verified_at.is_some());
            assert!(!after_web.last_verified_at.unwrap().is_empty());

            set_state(c, &conn.id, CanvasSessionState::LoginRequired)?;
            let after_login = get_connection(c, &conn.id)?;
            assert_eq!(after_login.state, CanvasSessionState::LoginRequired);
            // A non-verified state does not clear the last verified instant.
            assert!(after_login.last_verified_at.is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn sync_run_lifecycle() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let conn = ensure_connection(c, "https://example.test")?;
            let run = create_sync_run(c, &conn.id, "all", None)?;
            let stats = SyncStats {
                courses: 3,
                assignments: 10,
                ..SyncStats::default()
            };
            finish_sync_run(c, &run, "completed", &stats, None)?;
            let stored = get_sync_run(c, &run)?;
            assert_eq!(stored.status, "completed");
            assert_eq!(stored.stats.courses, 3);
            assert_eq!(stored.stats.assignments, 10);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn remote_mapping_prevents_duplicate_downloads() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            assert!(find_material_by_remote(c, "canvas_file", "77")?.is_none());
            upsert_mapping(c, "material", "local-1", "canvas_file", "77")?;
            assert_eq!(
                find_material_by_remote(c, "canvas_file", "77")?.as_deref(),
                Some("local-1")
            );
            // Idempotent.
            upsert_mapping(c, "material", "local-1", "canvas_file", "77")?;
            let count: i64 =
                c.query_row("SELECT COUNT(*) FROM resource_mappings", [], |r| r.get(0))?;
            assert_eq!(count, 1);
            Ok(())
        })
        .unwrap();
    }
}
