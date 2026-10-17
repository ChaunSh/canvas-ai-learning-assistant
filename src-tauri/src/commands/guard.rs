use tauri::WebviewWindow;

use crate::error::{AppError, AppResult};

/// Label of the trusted application window created from `tauri.conf.json`.
pub const MAIN_WINDOW: &str = "main";

/// Label of the isolated window that hosts the university Canvas login and the
/// read-only page probes. It must never be able to reach local commands.
pub const CANVAS_WINDOW: &str = "canvas-workspace";

/// Reject command invocations that did not originate from the trusted app window.
///
/// Tauri injects its IPC bridge into webviews it owns, therefore a label check is
/// required in addition to capability configuration: the Canvas workspace loads
/// a remote origin and must not be able to read the local database.
pub fn require_main_window(window: &WebviewWindow) -> AppResult<()> {
    let label = window.label();
    if label == MAIN_WINDOW {
        Ok(())
    } else {
        Err(AppError::Message(format!(
            "拒绝来自非受信任窗口（{label}）的本地接口调用"
        )))
    }
}
