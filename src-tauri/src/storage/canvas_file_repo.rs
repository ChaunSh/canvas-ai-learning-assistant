//! Canvas course files: the local mirror plus the download state machine.
//!
//! Canvas owns the list; this module only records what a sync saw, tracks which
//! files have been downloaded, and links a downloaded file to the material it
//! produced. The download URL is kept server-side only — the frontend never
//! needs it, and it is a capability URL for the signed-in webview.

use rusqlite::{params, Connection, Row};
use serde_json::Value;

use crate::domain::CanvasFile;
use crate::error::{AppError, AppResult};
use crate::time::now_iso;

pub const STATE_PENDING: &str = "pending";
pub const STATE_DOWNLOADING: &str = "downloading";
pub const STATE_DOWNLOADED: &str = "downloaded";
pub const STATE_FAILED: &str = "failed";
pub const STATE_UNSUPPORTED: &str = "unsupported";

fn row_to_file(row: &Row<'_>) -> rusqlite::Result<CanvasFile> {
    Ok(CanvasFile {
        id: row.get(0)?,
        course_id: row.get(1)?,
        canvas_file_id: row.get(2)?,
        display_name: row.get(3)?,
        content_type: row.get(4)?,
        byte_size: row.get(5)?,
        download_url: row.get(6)?,
        state: row.get(7)?,
        material_id: row.get(8)?,
        error: row.get(9)?,
        remote_updated_at: row.get(10)?,
        updated_at: row.get(11)?,
        folder_path: row.get(12)?,
    })
}

const SELECT_COLUMNS: &str = "id, course_id, canvas_file_id, display_name, content_type, byte_size,
     download_url, state, material_id, error, remote_updated_at, updated_at, folder_path";

/// One Canvas file object, reduced to the fields we can rely on.
pub struct RemoteFile {
    pub canvas_id: String,
    pub display_name: String,
    pub content_type: Option<String>,
    pub byte_size: Option<i64>,
    pub download_url: String,
    pub remote_updated_at: Option<String>,
    pub folder_id: Option<String>,
}

/// Map Canvas folder ids to their full path ("Lecture Slides/Week 1").
pub fn parse_folders(folders: &[Value]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for folder in folders {
        let Some(id) = folder
            .get("id")
            .and_then(|v| v.as_i64().map(|n| n.to_string()).or_else(|| v.as_str().map(str::to_string)))
        else {
            continue;
        };
        let full = folder
            .get("full_name")
            .and_then(|v| v.as_str())
            .or_else(|| folder.get("name").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim_matches('/')
            .to_string();
        if !full.is_empty() {
            map.insert(id, full);
        }
    }
    map
}

/// Read one file object from the Canvas API.
///
/// Files without a usable download URL are skipped by the caller: a row we can
/// never fetch would just sit in the list and look broken.
pub fn parse_remote(value: &Value) -> Option<RemoteFile> {
    let canvas_id = value
        .get("id")
        .and_then(|v| v.as_i64().map(|n| n.to_string()).or_else(|| v.as_str().map(str::to_string)))?;
    let download_url = value
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|u| !u.trim().is_empty())?
        .to_string();
    let display_name = value
        .get("display_name")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("filename").and_then(|v| v.as_str()))
        .unwrap_or(&canvas_id)
        .to_string();
    Some(RemoteFile {
        canvas_id,
        display_name,
        content_type: value
            .get("content-type")
            .or_else(|| value.get("content_type"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        byte_size: value.get("size").and_then(|v| v.as_i64()),
        download_url,
        remote_updated_at: value
            .get("updated_at")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        folder_id: value
            .get("folder_id")
            .and_then(|v| v.as_i64().map(|n| n.to_string()).or_else(|| v.as_str().map(str::to_string))),
    })
}

/// Insert files seen by a sync, refreshing metadata but never local state.
///
/// Re-syncing must not resurrect a downloaded file as pending, nor wipe the
/// material link, so only descriptive columns are updated.
pub fn upsert_files(
    conn: &Connection,
    course_id: &str,
    files: &[Value],
    folders: &std::collections::HashMap<String, String>,
) -> AppResult<i64> {
    let now = now_iso();
    let mut stmt = conn.prepare(
        "INSERT INTO canvas_files (id, course_id, canvas_file_id, display_name, content_type,
                                   byte_size, download_url, remote_updated_at, state,
                                   folder_path, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?10, ?10)
         ON CONFLICT (course_id, canvas_file_id) DO UPDATE SET
             display_name = excluded.display_name,
             content_type = COALESCE(excluded.content_type, canvas_files.content_type),
             byte_size = COALESCE(excluded.byte_size, canvas_files.byte_size),
             download_url = excluded.download_url,
             remote_updated_at = COALESCE(excluded.remote_updated_at, canvas_files.remote_updated_at),
             folder_path = COALESCE(excluded.folder_path, canvas_files.folder_path),
             updated_at = excluded.updated_at",
    )?;
    let mut count = 0i64;
    for file in files.iter().filter_map(parse_remote) {
        let folder = file
            .folder_id
            .as_ref()
            .and_then(|id| folders.get(id))
            .cloned();
        stmt.execute(params![
            uuid::Uuid::new_v4().to_string(),
            course_id,
            file.canvas_id,
            file.display_name,
            file.content_type,
            file.byte_size,
            file.download_url,
            file.remote_updated_at,
            folder,
            now
        ])?;
        count += 1;
    }
    Ok(count)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<CanvasFile> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM canvas_files WHERE id = ?1"),
        [id],
        row_to_file,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::not_found("课件文件不存在"),
        other => AppError::from(other),
    })
}

pub fn find_by_canvas_id(
    conn: &Connection,
    course_id: &str,
    canvas_file_id: &str,
) -> AppResult<Option<CanvasFile>> {
    conn.query_row(
        &format!(
            "SELECT {SELECT_COLUMNS} FROM canvas_files WHERE course_id = ?1 AND canvas_file_id = ?2"
        ),
        params![course_id, canvas_file_id],
        row_to_file,
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(AppError::from(other)),
    })
}

pub fn list_by_course(conn: &Connection, course_id: &str) -> AppResult<Vec<CanvasFile>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM canvas_files WHERE course_id = ?1
         ORDER BY state = 'downloaded', display_name COLLATE NOCASE"
    ))?;
    let rows = stmt.query_map([course_id], row_to_file)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Files a download run should fetch, oldest-first for stable ordering.
pub fn list_pending(conn: &Connection, course_id: Option<&str>) -> AppResult<Vec<CanvasFile>> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM canvas_files
         WHERE state IN ('pending', 'failed')
           AND (?1 IS NULL OR course_id = ?1)
         ORDER BY course_id, display_name COLLATE NOCASE"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![course_id], row_to_file)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn set_state(
    conn: &Connection,
    id: &str,
    state: &str,
    material_id: Option<&str>,
    error: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE canvas_files SET state = ?2, material_id = COALESCE(?3, material_id),
                error = ?4, updated_at = ?5 WHERE id = ?1",
        params![id, state, material_id, error, now_iso()],
    )?;
    Ok(())
}

pub fn counts(conn: &Connection, course_id: &str) -> AppResult<(i64, i64)> {
    let downloaded: i64 = conn.query_row(
        "SELECT COUNT(*) FROM canvas_files WHERE course_id = ?1 AND state = 'downloaded'",
        [course_id],
        |r| r.get(0),
    )?;
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM canvas_files WHERE course_id = ?1 AND state IN ('pending', 'failed')",
        [course_id],
        |r| r.get(0),
    )?;
    Ok((downloaded, pending))
}


/// Rows left in `downloading` by a crashed or killed run are returned to the
/// queue, otherwise they would never be retried.
pub fn reset_stale_downloads(conn: &Connection) -> AppResult<()> {
    conn.execute(
        "UPDATE canvas_files SET state = 'pending', error = '上次下载被中断', updated_at = ?1
         WHERE state = 'downloading'",
        [now_iso()],
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;
    use serde_json::json;

    fn seed_course(db: &Db) -> String {
        db.with(|c| {
            c.execute(
                "INSERT INTO courses (id, name, source, sync_status, archived, position, created_at, updated_at)
                 VALUES ('k1','操作系统','canvas','synced',0,1,'t','t')",
                [],
            )?;
            // A material row so linking a downloaded file can satisfy its FK.
            c.execute(
                "INSERT INTO materials (id, course_id, title, filename, sha256, storage_path, created_at, updated_at)
                 VALUES ('m1','k1','第一讲','第一讲.pdf','sha1','p1','t','t')",
                [],
            )?;
            Ok::<(), crate::error::AppError>(())
        })
        .unwrap();
        "k1".to_string()
    }

    fn file(id: i64, name: &str) -> Value {
        json!({
            "id": id,
            "display_name": name,
            "content-type": "application/pdf",
            "size": 1024,
            "url": format!("https://hkust-gz.instructure.com/files/{id}/download?verifier=x"),
            "updated_at": "2026-10-01T00:00:00Z"
        })
    }

    #[test]
    fn upsert_keeps_local_download_state() {
        let db = Db::open_in_memory().unwrap();
        let course = seed_course(&db);
        db.with(|c| {
            let n = upsert_files(c, &course, &[file(1, "第一讲.pdf"), file(2, "第二讲.pdf")], &Default::default())?;
            assert_eq!(n, 2);

            let rows = list_by_course(c, &course)?;
            assert_eq!(rows.len(), 2);
            let first = rows.iter().find(|f| f.canvas_file_id == "1").unwrap();
            assert_eq!(first.state, STATE_PENDING);
            assert_eq!(first.display_name, "第一讲.pdf");

            set_state(c, &first.id, STATE_DOWNLOADED, Some("m1"), None)?;
            // A re-sync refreshes metadata but must not reset the state.
            upsert_files(c, &course, &[file(1, "第一讲-改名.pdf"), file(2, "第二讲.pdf")], &Default::default())?;
            let after = get(c, &first.id)?;
            assert_eq!(after.state, STATE_DOWNLOADED);
            assert_eq!(after.material_id.as_deref(), Some("m1"));
            assert_eq!(after.display_name, "第一讲-改名.pdf");
            Ok::<(), crate::error::AppError>(())
        })
        .unwrap();
    }

    #[test]
    fn files_without_a_download_url_are_ignored() {
        let db = Db::open_in_memory().unwrap();
        let course = seed_course(&db);
        db.with(|c| {
            let broken = json!({ "id": 9, "display_name": "坏链接.pdf", "url": "" });
            let n = upsert_files(c, &course, &[file(1, "a.pdf"), broken], &Default::default())?;
            assert_eq!(n, 1, "only the usable file is stored");
            Ok::<(), crate::error::AppError>(())
        })
        .unwrap();
    }

    #[test]
    fn pending_list_includes_failed_for_retry() {
        let db = Db::open_in_memory().unwrap();
        let course = seed_course(&db);
        db.with(|c| {
            upsert_files(c, &course, &[file(1, "a.pdf"), file(2, "b.pdf"), file(3, "c.pdf")], &Default::default())?;
            let rows = list_by_course(c, &course)?;
            let by_name = |name: &str| rows.iter().find(|f| f.display_name == name).unwrap().id.clone();
            set_state(c, &by_name("a.pdf"), STATE_DOWNLOADED, Some("m1"), None)?;
            set_state(c, &by_name("b.pdf"), STATE_FAILED, None, Some("网络错误"))?;
            set_state(c, &by_name("c.pdf"), STATE_DOWNLOADING, None, None)?;

            let pending = list_pending(c, Some(&course))?;
            let names: Vec<&str> = pending.iter().map(|f| f.display_name.as_str()).collect();
            assert_eq!(names, vec!["b.pdf"], "failed retries, downloaded/in-flight skipped");

            let (downloaded, remaining) = counts(c, &course)?;
            assert_eq!(downloaded, 1);
            assert_eq!(remaining, 1);
            Ok::<(), crate::error::AppError>(())
        })
        .unwrap();
    }

    #[test]
    fn parsing_falls_back_to_the_raw_filename() {
        let parsed = parse_remote(&json!({
            "id": "42",
            "filename": "week-3.pdf",
            "url": "https://x/files/42/download"
        }))
        .expect("parsed");
        assert_eq!(parsed.canvas_id, "42");
        assert_eq!(parsed.display_name, "week-3.pdf");
        assert_eq!(parsed.byte_size, None);
    }
}
