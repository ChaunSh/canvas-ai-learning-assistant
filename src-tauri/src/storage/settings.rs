use std::collections::HashMap;

use rusqlite::Connection;

use crate::error::AppResult;
use crate::time::now_iso;

/// Read a setting value (raw string).
pub fn get(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row("SELECT value FROM app_settings WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?)
}

/// Write a setting value.
pub fn set(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        rusqlite::params![key, value, now_iso()],
    )?;
    Ok(())
}

pub fn remove(conn: &Connection, key: &str) -> AppResult<()> {
    conn.execute("DELETE FROM app_settings WHERE key = ?1", [key])?;
    Ok(())
}

/// Read a JSON-encoded setting with a fallback.
pub fn get_json<T: serde::de::DeserializeOwned + Default>(
    conn: &Connection,
    key: &str,
) -> AppResult<T> {
    match get(conn, key)? {
        Some(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        None => Ok(T::default()),
    }
}

/// Write a JSON-encoded setting.
pub fn set_json<T: serde::Serialize>(conn: &Connection, key: &str, value: &T) -> AppResult<()> {
    let raw = serde_json::to_string(value)?;
    set(conn, key, &raw)
}

/// All settings, for the diagnostics screen.
pub fn all(conn: &Connection) -> AppResult<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT key, value FROM app_settings ORDER BY key")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut map = HashMap::new();
    for row in rows {
        let (k, v) = row?;
        map.insert(k, v);
    }
    Ok(map)
}

pub const KEY_THEME: &str = "ui.theme";
pub const KEY_LOCALE: &str = "ui.locale";
pub const KEY_ONBOARDED: &str = "app.onboarded";
pub const KEY_AI_PRIVACY: &str = "ai.privacy";
pub const KEY_SYNC_POLICY: &str = "sync.policy";
pub const KEY_LAST_SYNC: &str = "sync.lastAt";
