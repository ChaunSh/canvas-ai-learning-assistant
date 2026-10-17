//! Canvas web-session integration.
//!
//! ## Why this design
//!
//! The Canvas web login is the *only* authentication entry point (task
//! specification §6). There is no PAT and no OAuth. To read data without
//! extracting cookies or tokens, the app hosts the university Canvas site in an
//! isolated webview and runs **same-origin `fetch` inside that page**, letting
//! the browser attach its own session. The result comes back to Rust through a
//! navigation event that Rust intercepts and cancels.
//!
//! What this deliberately does NOT do:
//!   * read, copy, persist or replay cookies / SSO / CSRF material;
//!   * send session state to a native HTTP client;
//!   * bypass CORS, CSRF or access control;
//!   * expose Tauri IPC to the remote origin (no remote capability is granted).
//!
//! The remote page can therefore never call a local command: application
//! commands additionally verify they came from the `main` window.

pub mod download;
pub mod parse;
pub mod probe;
pub mod sync;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::oneshot;

use crate::commands::guard::CANVAS_WINDOW;
use crate::domain::{
    CanvasConnection, CanvasSessionState, CapabilityStatus, ProbeResult, CANVAS_BASE_URL,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::sync_repo;

use self::sync::CanvasSync;

/// Path segment used for the native-side response channel. Requests to it are
/// intercepted before any network activity happens.
pub const BRIDGE_PATH: &str = "/__caa_bridge__/";

/// Emitted on the main window when the login watcher verifies a session, so the
/// UI can flip to "connected" without the user pressing anything.
pub const CONNECTION_EVENT: &str = "canvas://connection";

/// A probe awaiting its result, keyed by nonce.
#[derive(Clone, Default)]
pub struct CanvasProbeHub {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<probe::BridgePayload>>>>,
}

impl CanvasProbeHub {
    pub fn register(&self, nonce: &str) -> oneshot::Receiver<probe::BridgePayload> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(nonce.to_string(), tx);
        rx
    }

    /// Called from the navigation handler; returns true when the URL was a
    /// bridge response (and therefore must be blocked).
    pub fn try_resolve(&self, url: &tauri::Url) -> bool {
        let Some(payload) = probe::parse_bridge_url(url.as_str()) else {
            return false;
        };
        let mut pending = self.pending.lock();
        if let Some(sender) = pending.remove(&payload.nonce) {
            let _ = sender.send(payload);
        }
        true
    }

    pub fn cancel(&self, nonce: &str) {
        self.pending.lock().remove(nonce);
    }

    pub fn pending_count(&self) -> usize {
        self.pending.lock().len()
    }
}



/// Hosts the Canvas window may load.
///
/// Canvas routes login through its own service domains (`sso.canvaslms.com`
/// forwarding, `inst-fs` for files), so an allowlist limited to the university
/// domain stalls the SAML chain on a blank "Working..." page.
pub fn is_allowed_host(host: &str) -> bool {
    // Canvas serves course files from its own content/CDN domains; blocking
    // them cancels every download before it starts.
    const SUFFIXES: [&str; 5] = [
        ".instructure.com",
        ".canvaslms.com",
        ".canvas-user-content.com",
        ".inscloudgate.net",
        ".hkust-gz.edu.cn",
    ];
    const EXACT: [&str; 3] = ["instructure.com", "canvaslms.com", "hkust-gz.edu.cn"];
    EXACT.contains(&host) || SUFFIXES.iter().any(|suffix| host.ends_with(suffix))
}
/// Show and focus the Canvas window once it has something to paint.
fn reveal_canvas_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(CANVAS_WINDOW) {
        if let Err(error) = window.show() {
            tracing::warn!(%error, "could not show the Canvas window");
        }
        let _ = window.set_focus();
    }
}
/// Owns the isolated Canvas webview and serialises probe traffic.
pub struct CanvasRuntime {
    pub hub: CanvasProbeHub,
    /// Native download channel for course files.
    /// Serialises probes so only one script runs at a time.
    lock: tokio::sync::Mutex<()>,
    /// Base URL of the university instance.
    pub base_url: String,
}

impl CanvasRuntime {
    pub fn new() -> Self {
        Self {
            hub: CanvasProbeHub::default(),
                lock: tokio::sync::Mutex::new(()),
            base_url: CANVAS_BASE_URL.to_string(),
        }
    }

    /// True when a Canvas window currently exists.
    pub fn is_open(&self, app: &AppHandle) -> bool {
        app.get_webview_window(CANVAS_WINDOW).is_some()
    }

    /// Create (or focus) the isolated login window.
    pub fn ensure_window(&self, app: &AppHandle, url: &str) -> AppResult<WebviewWindow> {
        if let Some(existing) = app.get_webview_window(CANVAS_WINDOW) {
            // A window whose first navigation never happened sits on about:blank
            // and stays white forever; focusing it would just show that blank
            // surface again, so navigate it to the intended page.
            let blank = existing
                .url()
                .map(|current| matches!(current.scheme(), "about" | "data"))
                .unwrap_or(true);
            if blank {
                if let Ok(target) = tauri::Url::parse(url) {
                    let _ = existing.navigate(target);
                }
            }
            let _ = existing.show();
            let _ = existing.set_focus();
            return Ok(existing);
        }

        let target = tauri::Url::parse(url)
            .map_err(|e| AppError::invalid(format!("Canvas 地址无效：{e}")))?;
        if !self.is_allowed_origin(&target) {
            return Err(AppError::invalid(format!(
                "拒绝打开非学校 Canvas 域名：{}",
                target.host_str().unwrap_or("?")
            )));
        }

        let hub = self.hub.clone();
        let app_for_load = app.clone();
        let window = WebviewWindowBuilder::new(app, CANVAS_WINDOW, WebviewUrl::External(target))
            .title("Canvas 登录 / 工作区")
            .inner_size(1100.0, 820.0)
            .min_inner_size(420.0, 520.0)
            .on_navigation(move |url| {
                // Bridge responses are consumed natively and never loaded.
                if hub.try_resolve(url) {
                    return false;
                }
                // Never block the webview's own plumbing pages: blocking
                // `about:blank` on the very first navigation leaves the window
                // stuck on a blank surface until it is recreated.
                if matches!(url.scheme(), "about" | "data" | "blob") {
                    return true;
                }
                // Canvas-owned domains only (the university site plus Canvas'
                // own login and file-service domains).
                let host = url.host_str().unwrap_or("");
                let allowed = is_allowed_host(host);
                if !allowed {
                    tracing::warn!(host, "blocked navigation to a non-Canvas origin");
                }
                allowed
            })
            // Create it hidden and reveal it once the first page is painted.
            // Showing an empty WebView2 before its first navigation is what
            // produced the white window that only worked after reopening.
            .visible(false)
            .on_page_load(move |_webview, payload| {
                if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                    reveal_canvas_window(&app_for_load);
                }
            })
            .build()
            .map_err(|e| AppError::Message(format!("无法创建 Canvas 窗口：{e}")))?;

        // Safety net: if the first page never finishes loading (offline, blocked
        // redirect), still show the window so the user is not left with nothing.
        {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                if let Some(window) = app.get_webview_window(CANVAS_WINDOW) {
                    if !window.is_visible().unwrap_or(true) {
                        tracing::warn!("Canvas window did not finish loading; revealing it anyway");
                        reveal_canvas_window(&app);
                    }
                }
            });
        }

        Ok(window)
    }


    /// Wait until the webview reports a Canvas URL, or give up after `attempts`.
    pub(crate) async fn wait_for_canvas_page(&self, window: &WebviewWindow, attempts: u32) -> bool {
        for _ in 0..attempts {
            if let Ok(current) = window.url() {
                let host = current.host_str().unwrap_or_default();
                let on_canvas = current.scheme() == "https" && is_allowed_host(host);
                if on_canvas {
                    return true;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
        false
    }
    /// Run one same-origin fetch inside the Canvas page and await its payload.
    pub async fn probe(
        &self,
        app: &AppHandle,
        path: &str,
        purpose: &str,
    ) -> AppResult<ProbeResult> {
        let _guard = self.lock.lock().await;

        if !path.starts_with('/') {
            return Err(AppError::invalid("Canvas 探测路径必须以 / 开头"));
        }
        let window = self.ensure_window(app, &format!("{}/", self.base_url))?;

        // A freshly created window is still loading; evaluating a fetch there
        // fails with a network error that looks like "session expired". Wait
        // until the webview actually reports a Canvas URL before probing.
        if !self.wait_for_canvas_page(&window, 40).await {
            // Still not there: force a navigation once and give it more time.
            if let Ok(url) = tauri::Url::parse(&format!("{}/", self.base_url)) {
                let _ = window.navigate(url);
            }
            if !self.wait_for_canvas_page(&window, 40).await {
                return Ok(ProbeResult {
                    ok: false,
                    http_status: None,
                    body: None,
                    error: Some("Canvas 页面尚未加载完成，请稍后重试".to_string()),
                    url: path.to_string(),
                });
            }
        }
        // Small settle so the document (and its JS context) exists.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let nonce = uuid::Uuid::new_v4().to_string();
        let receiver = self.hub.register(&nonce);
        let script = probe::build_fetch_script(&nonce, path, BRIDGE_PATH);
        window
            .eval(&script)
            .map_err(|e| AppError::Message(format!("无法在 Canvas 页面中执行探测：{e}")))?;

        let payload = match tokio::time::timeout(std::time::Duration::from_secs(45), receiver).await {
            Ok(Ok(payload)) => payload,
            Ok(Err(_)) => {
                return Err(AppError::Canvas("Canvas 探测通道已关闭".into()));
            }
            Err(_) => {
                self.hub.cancel(&nonce);
                return Err(AppError::Canvas(format!(
                    "Canvas 探测超时（{purpose}）。请确认已登录并处于 Canvas 页面。"
                )));
            }
        };

        let http_status = payload.status;
        if let Some(error) = payload.error {
            return Ok(ProbeResult {
                ok: false,
                http_status,
                body: None,
                error: Some(error),
                url: path.to_string(),
            });
        }

        let body_text = payload.body.unwrap_or_default();
        let parsed = serde_json::from_str::<serde_json::Value>(&body_text).ok();

        // 401/403 are meaningful capability answers, not crashes.
        let ok = matches!(http_status, Some(code) if (200..300).contains(&code));
        Ok(ProbeResult {
            ok,
            http_status,
            body: parsed,
            error: if ok {
                None
            } else {
                Some(match http_status {
                    Some(401) => "未登录或会话已过期（401）".to_string(),
                    Some(403) => "当前账号无权访问该数据（403）".to_string(),
                    Some(404) => "接口不存在（404）".to_string(),
                    Some(code) => format!("Canvas 返回 HTTP {code}"),
                    None => "未收到有效的 HTTP 状态".to_string(),
                })
            },
            url: path.to_string(),
        })
    }

    pub fn is_allowed_origin(&self, url: &tauri::Url) -> bool {
        if url.scheme() != "https" {
            return false;
        }
        let base_host = tauri::Url::parse(&self.base_url)
            .ok()
            .and_then(|base| base.host_str().map(|host| host.to_string()))
            .unwrap_or_default();
        // Canvas assets and SSO forwarding live on sibling service domains, so
        // sharing the same predicate keeps the window usable end to end.
        url.host_str()
            .map(|host| host == base_host || is_allowed_host(host))
            .unwrap_or(false)
    }

    /// Close the Canvas window and clear its browsing data.
    pub async fn disconnect(&self, app: &AppHandle) -> AppResult<()> {
        if let Some(window) = app.get_webview_window(CANVAS_WINDOW) {
            // Clearing before close ensures the session is not left on disk.
            let _ = window.clear_all_browsing_data();
            let _ = window.close();
        }
        Ok(())
    }
}

impl Default for CanvasRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Map a probe outcome onto the capability matrix.
pub fn capability_from_probe(probe: &ProbeResult) -> CapabilityStatus {
    match (probe.http_status, probe.ok) {
        (Some(401), _) => CapabilityStatus::LoginRequired,
        (Some(403), _) => CapabilityStatus::PermissionDenied,
        (Some(404), _) => CapabilityStatus::Unsupported,
        (Some(code), _) if (500..=599).contains(&code) => CapabilityStatus::TemporarilyUnavailable,
        (_, true) => CapabilityStatus::Supported,
        _ => CapabilityStatus::TemporarilyUnavailable,
    }
}

/// Translate a session state into a user-facing sentence.
pub fn describe_state(state: CanvasSessionState) -> &'static str {
    match state {
        CanvasSessionState::NotConnected => "尚未连接 Canvas",
        CanvasSessionState::LoginRequired => "需要在 Canvas 页面完成登录",
        CanvasSessionState::LoggingIn => "正在等待登录完成",
        CanvasSessionState::WebAuthenticated => "网页已登录（尚未验证数据读取权限）",
        CanvasSessionState::DataAccessVerified => "网页已登录，且课程数据可读",
        CanvasSessionState::SessionExpired => "会话已过期，请重新登录",
        CanvasSessionState::Unsupported => "当前环境不支持嵌入式 Canvas 登录",
        CanvasSessionState::Error => "连接发生错误",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_mapping_distinguishes_failures() {
        let probe = |status: Option<i64>, ok: bool| ProbeResult {
            ok,
            http_status: status,
            body: None,
            error: None,
            url: "/api/v1/courses".into(),
        };
        assert_eq!(
            capability_from_probe(&probe(Some(200), true)),
            CapabilityStatus::Supported
        );
        assert_eq!(
            capability_from_probe(&probe(Some(401), false)),
            CapabilityStatus::LoginRequired
        );
        assert_eq!(
            capability_from_probe(&probe(Some(403), false)),
            CapabilityStatus::PermissionDenied
        );
        assert_eq!(
            capability_from_probe(&probe(Some(404), false)),
            CapabilityStatus::Unsupported
        );
        assert_eq!(
            capability_from_probe(&probe(Some(503), false)),
            CapabilityStatus::TemporarilyUnavailable
        );
    }

    #[test]
    fn probe_hub_resolves_once() {
        let hub = CanvasProbeHub::default();
        let mut rx = hub.register("abc");
        let url = tauri::Url::parse(
            "https://hkust-gz.instructure.com/__caa_bridge__/abc?status=200&body=%7B%7D",
        )
        .unwrap();
        assert!(hub.try_resolve(&url), "bridge url must be intercepted");
        assert_eq!(hub.pending_count(), 0);
        let payload = rx.try_recv().expect("payload delivered");
        assert_eq!(payload.status, Some(200));
        // A second resolution for the same nonce is still blocked but has no receiver.
        assert!(hub.try_resolve(&url));
    }

    #[test]
    fn non_bridge_urls_are_not_intercepted() {
        let hub = CanvasProbeHub::default();
        let url = tauri::Url::parse("https://hkust-gz.instructure.com/courses").unwrap();
        assert!(!hub.try_resolve(&url));
    }

    #[test]
    fn state_descriptions_exist() {
        for state in [
            CanvasSessionState::NotConnected,
            CanvasSessionState::LoginRequired,
            CanvasSessionState::LoggingIn,
            CanvasSessionState::WebAuthenticated,
            CanvasSessionState::DataAccessVerified,
            CanvasSessionState::SessionExpired,
            CanvasSessionState::Unsupported,
            CanvasSessionState::Error,
        ] {
            assert!(!describe_state(state).is_empty());
        }
    }
}

/// True when a URL is a logged-in Canvas page rather than an auth hop.
///
/// SSO/MFA pages live on the same registrable domains, so a host check alone
/// would keep probing while the user is still typing a password.
fn is_canvas_app_page(url: &tauri::Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    let on_canvas = host.ends_with("instructure.com") || host.ends_with("hkust-gz.edu.cn");
    if !on_canvas {
        return false;
    }
    let path = url.path().to_ascii_lowercase();
    const AUTH_HINTS: [&str; 7] = ["login", "saml", "oauth", "auth", "mfa", "idp", "signin"];
    !AUTH_HINTS.iter().any(|hint| path.contains(hint))
}

/// Verify the session automatically once the user reaches a logged-in page.
///
/// The user completes SSO/MFA inside the Canvas window; nothing tells the app
/// when that finished. Without this the UI sat on "logging in" until the user
/// manually pressed 「检查连接」 (the reported bug). The watcher polls the
/// window URL, runs the same verification the button runs, and emits
/// [`CONNECTION_EVENT`] so the main window updates itself.
///
/// It gives up after ~10 minutes or when the window is closed, and stops early
/// once data access is verified.
pub fn spawn_login_watcher(app: AppHandle) {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        for attempt in 0..120u32 {
            let wait = if attempt == 0 { 2 } else { 5 };
            tokio::time::sleep(Duration::from_secs(wait)).await;

            let Some(window) = app.get_webview_window(CANVAS_WINDOW) else {
                break;
            };
            let Ok(url) = window.url() else { continue };
            if !is_canvas_app_page(&url) {
                continue;
            }

            let state = app.state::<AppState>();
            let Ok(current) = state
                .db
                .with(|c| sync_repo::ensure_connection(c, CANVAS_BASE_URL))
            else {
                continue;
            };
            if matches!(current.state, CanvasSessionState::DataAccessVerified) {
                break;
            }

            let sync = CanvasSync::new(&state.db, state.canvas.as_ref(), &app);
            match sync.check_connection().await {
                Ok(connection) => {
                    if matches!(
                        connection.state,
                        CanvasSessionState::DataAccessVerified | CanvasSessionState::WebAuthenticated
                    ) {
                        announce(&app, &connection);
                    }
                    if matches!(connection.state, CanvasSessionState::DataAccessVerified) {
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!(%error, "canvas login watcher: verification failed");
                }
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// Push a connection update to the UI (and log it) after an automatic check.
fn announce(app: &AppHandle, connection: &CanvasConnection) {
    tracing::info!(
        state = ?connection.state,
        "canvas session verified automatically"
    );
    if let Err(error) = app.emit(CONNECTION_EVENT, connection) {
        tracing::warn!(%error, "could not emit canvas connection event");
    }
}