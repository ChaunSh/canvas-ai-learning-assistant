//! Canvas connection commands.
//!
//! These handlers never receive or return session material. They open the
//! isolated webview, ask it to verify state, and persist only the resulting
//! capability matrix.

use serde::Serialize;
use tauri::{AppHandle, State, WebviewWindow};

use crate::commands::guard::require_main_window;
use crate::domain::{
    CanvasConnection, CanvasSessionState, SyncRun, CANVAS_BASE_URL,
};
use crate::error::{AppError, AppResult};
use crate::integrations::canvas::sync::{summarize, CanvasSync};
use crate::state::AppState;
use crate::storage::sync_repo;

fn ensure_connection(state: &AppState) -> AppResult<CanvasConnection> {
    state
        .db
        .with(|c| sync_repo::ensure_connection(c, CANVAS_BASE_URL))
}

#[tauri::command]
pub async fn get_canvas_connection(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<CanvasConnection> {
    require_main_window(&window)?;
    let mut connection = ensure_connection(&state)?;
    connection.diagnostics.push(crate::domain::DiagnosticEntry {
        at: crate::time::now_iso(),
        level: "info".into(),
        message: crate::integrations::canvas::describe_state(connection.state).into(),
    });
    Ok(connection)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasLoginResult {
    pub base_url: String,
    pub state: CanvasSessionState,
    pub message: String,
}

/// Open the official Canvas login page in the isolated workspace window.
#[tauri::command]
pub async fn open_canvas_login(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<CanvasLoginResult> {
    require_main_window(&window)?;
    let connection = ensure_connection(&state)?;
    state.db.with(|c| {
        sync_repo::set_state(c, &connection.id, CanvasSessionState::LoggingIn)
    })?;

    let url = format!("{CANVAS_BASE_URL}/login");
    state.canvas.ensure_window(&app, &url)?;
    // The user finishes SSO/MFA in that window; watch it so the app notices the
    // login by itself instead of waiting for a manual "check connection".
    crate::integrations::canvas::spawn_login_watcher(app.clone());

    Ok(CanvasLoginResult {
        base_url: CANVAS_BASE_URL.to_string(),
        state: CanvasSessionState::LoggingIn,
        message: "已打开学校 Canvas 登录页面。完成登录（含 SSO / MFA）后会自动检测并更新状态，无需手动点击。"
            .into(),
    })
}

/// Open an arbitrary Canvas page inside the workspace window.
#[tauri::command]
pub async fn open_canvas_page(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    let parsed = tauri::Url::parse(&url)
        .map_err(|e| AppError::invalid(format!("页面地址无效：{e}")))?;
    if !state.canvas.is_allowed_origin(&parsed) {
        return Err(AppError::invalid(
            "只能打开学校 Canvas 域名下的页面",
        ));
    }
    state.canvas.ensure_window(&app, &url)?;
    Ok(())
}

/// Reset the connection state and clear the app's Canvas browsing data.
#[tauri::command]
pub async fn disconnect_canvas(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    require_main_window(&window)?;
    let connection = ensure_connection(&state)?;
    state.canvas.disconnect(&app).await?;
    state.db.with(|c| {
        // Capabilities become "not-tested": we can no longer attest to them.
        for key in crate::domain::CAPABILITY_KEYS {
            sync_repo::set_capability(
                c,
                &connection.id,
                key,
                crate::domain::CapabilityStatus::NotTested,
                Some("已断开连接，需重新验证"),
            )?;
        }
        sync_repo::set_identity(c, &connection.id, None, None)?;
        sync_repo::set_state(c, &connection.id, CanvasSessionState::NotConnected)
    })?;
    Ok(())
}

/// Verify login *and* data access, updating every capability we can test.
#[tauri::command]
pub async fn check_canvas_connection(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<CanvasConnection> {
    require_main_window(&window)?;
    let sync = CanvasSync::new(&state.db, state.canvas.as_ref(), &app);
    sync.check_connection().await
}

#[tauri::command]
pub async fn get_canvas_capabilities(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::domain::Capability>> {
    require_main_window(&window)?;
    let connection = ensure_connection(&state)?;
    state
        .db
        .with(|c| sync_repo::list_capabilities(c, &connection.id))
}

#[tauri::command]
pub async fn sync_all(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<SyncRun> {
    require_main_window(&window)?;
    let sync = CanvasSync::new(&state.db, state.canvas.as_ref(), &app);
    let run = sync.sync_all().await?;
    // Files listed by the sync can be fetched right away; this runs in the
    // background so the sync result reaches the UI immediately.
    crate::commands::canvas_files::spawn_auto_download(app.clone());
    Ok(run)
}

#[tauri::command]
pub async fn sync_course(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    course_id: String,
) -> AppResult<SyncRun> {
    require_main_window(&window)?;
    let sync = CanvasSync::new(&state.db, state.canvas.as_ref(), &app);
    sync.sync_course(&course_id).await
}

#[tauri::command]
pub async fn get_sync_history(
    window: WebviewWindow,
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> AppResult<Vec<SyncRun>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| sync_repo::list_sync_runs(c, limit.unwrap_or(30)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRunSummary {
    pub run: SyncRun,
    pub summary: String,
}

#[tauri::command]
pub async fn sync_history_detailed(
    window: WebviewWindow,
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> AppResult<Vec<SyncRunSummary>> {
    require_main_window(&window)?;
    let runs = state
        .db
        .with(|c| sync_repo::list_sync_runs(c, limit.unwrap_or(30)))?;
    Ok(runs
        .into_iter()
        .map(|run| SyncRunSummary {
            summary: summarize(&run),
            run,
        })
        .collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDiagnostics {
    pub errors: Vec<SyncErrorView>,
    pub log: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncErrorView {
    pub run_id: String,
    pub resource: Option<String>,
    pub message: String,
    pub retryable: bool,
}

#[tauri::command]
pub async fn get_sync_diagnostics(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<SyncDiagnostics> {
    require_main_window(&window)?;
    let errors = state
        .db
        .with(|c| sync_repo::recent_sync_errors(c, 100))?;
    let log = state.db.with(|c| sync_repo::sync_log_export(c))?;
    Ok(SyncDiagnostics {
        errors: errors
            .into_iter()
            .map(|(run_id, resource, message, retryable)| SyncErrorView {
                run_id,
                resource,
                message,
                retryable,
            })
            .collect(),
        log,
    })
}

/// Export the sync log to a user-chosen path.
#[tauri::command]
pub async fn export_sync_log(
    window: WebviewWindow,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    let log = state.db.with(|c| sync_repo::sync_log_export(c))?;
    let text = serde_json::to_string_pretty(&log)?;
    std::fs::write(&path, text)?;
    Ok(())
}

/// Static description of the integration for the diagnostics screen.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasIntegrationInfo {
    pub base_url: String,
    pub auth_model: String,
    pub file_download_supported: bool,
    pub notes: Vec<String>,
}

#[tauri::command]
pub async fn canvas_integration_info(
    window: WebviewWindow,
) -> AppResult<CanvasIntegrationInfo> {
    require_main_window(&window)?;
    Ok(CanvasIntegrationInfo {
        base_url: CANVAS_BASE_URL.to_string(),
        auth_model: "仅 Canvas 网页登录（无 PAT / 无 OAuth）".into(),
        file_download_supported: true,
        notes: vec![
            "所有数据请求都在应用内的 Canvas 页面中同源执行，浏览器自行携带会话。".into(),
            "应用不读取、不复制、不持久化任何 Cookie、SSO 或 CSRF 材料。".into(),
            "课件文件由 Canvas 页面自身发起下载，落盘后按普通课件解析与索引；应用不接触下载令牌。".into(),
            "每类数据的能力状态来自真实请求结果；未通过测试的能力保持「未测试」。".into(),
        ],
    })
}
