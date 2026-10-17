use serde::{Deserialize, Serialize};
use tauri::{State, WebviewWindow};

use crate::commands::guard::require_main_window;
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::{settings, Db};
use crate::time::now_iso;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub tauri_version: String,
    pub platform: String,
    pub arch: String,
    pub data_dir: String,
    pub database_path: String,
    pub database_version: i64,
    pub database_size: u64,
    pub started_at: String,
}

#[tauri::command]
pub async fn app_info(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<AppInfo> {
    require_main_window(&window)?;
    let paths = state.paths.clone();
    let db = state.db.clone();
    let version = db.with(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))?;
    Ok(AppInfo {
        name: "Canvas AI 学习助手".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        tauri_version: tauri::VERSION.into(),
        platform: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        data_dir: paths.data_dir.to_string_lossy().to_string(),
        database_path: paths.db_path.to_string_lossy().to_string(),
        database_version: version,
        database_size: db.file_size(),
        started_at: now_iso(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingEntry {
    pub key: String,
    pub value: String,
    pub updated_at: Option<String>,
}

#[tauri::command]
pub async fn settings_get(
    window: WebviewWindow,
    state: State<'_, AppState>,
    key: String,
) -> AppResult<Option<String>> {
    require_main_window(&window)?;
    reuse(&state.db, |c| settings::get(c, &key))
}

#[tauri::command]
pub async fn settings_set(
    window: WebviewWindow,
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    reuse(&state.db, |c| settings::set(c, &key, &value))
}

#[tauri::command]
pub async fn settings_all(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<Vec<SettingEntry>> {
    require_main_window(&window)?;
    reuse(&state.db, |c| {
        let map = settings::all(c)?;
        Ok(map
            .into_iter()
            .map(|(key, value)| SettingEntry {
                key,
                value,
                updated_at: None,
            })
            .collect())
    })
}

/// Bridge a synchronous repository closure into an async command without
/// holding the database lock across an await point.
fn reuse<T>(db: &Db, f: impl FnOnce(&rusqlite::Connection) -> AppResult<T>) -> AppResult<T> {
    db.with(f)
}
