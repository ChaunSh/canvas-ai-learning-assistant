//! Canvas file downloads.
//!
//! The Canvas window owns the session; this module reads that session's cookies
//! and replays them from Rust. Doing the transfer in Rust removes both blockers
//! the browser put in the way: page scripts may not read a cross-origin file
//! body (CORS), and navigating to the URL is treated as a normal page load.
//!
//! Trade-off, recorded deliberately: cookies leave the webview. They are held in
//! memory for one request, never written to disk or logs, and are only read for
//! the school's Canvas host.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tauri::AppHandle;

use crate::domain::{CanvasFile, Material};
use crate::error::{AppError, AppResult};
use crate::paths::AppPaths;
use crate::storage::{canvas_file_repo, Db};

use super::CanvasRuntime;

/// Largest single file we will fetch automatically. Course videos are often
/// hundreds of megabytes and cannot be parsed anyway.
pub const MAX_BYTES: i64 = 100 * 1024 * 1024;

/// Extensions the parser can turn into an indexable material.
const SUPPORTED_EXTENSIONS: [&str; 12] = [
    "pdf", "pptx", "docx", "txt", "md", "markdown", "png", "jpg", "jpeg", "ics", "csv", "json",
];

/// Why a listed file will not be downloaded automatically.
pub fn check_downloadable(file: &CanvasFile) -> Result<(), String> {
    let name = file.display_name.to_ascii_lowercase();
    let extension = name.rsplit('.').next().unwrap_or_default();
    if !SUPPORTED_EXTENSIONS.contains(&extension) {
        return Err(format!("暂不支持的文件类型（.{extension}）"));
    }
    if let Some(size) = file.byte_size {
        if size > MAX_BYTES {
            return Err(format!(
                "文件过大（{:.0} MB，上限 {} MB）",
                size as f64 / 1024.0 / 1024.0,
                MAX_BYTES / 1024 / 1024
            ));
        }
    }
    Ok(())
}

/// A file URL we are willing to hand to the HTTP client.
pub fn is_canvas_file_url(url: &str) -> bool {
    let Ok(parsed) = tauri::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or_default();
    let trusted = host.ends_with("instructure.com") || host.ends_with("hkust-gz.edu.cn");
    trusted && parsed.path().contains("/files/")
}

/// Download one file and import it as a material.
pub async fn download_one(
    db: &Db,
    paths: &Arc<AppPaths>,
    runtime: &CanvasRuntime,
    app: &AppHandle,
    file: &CanvasFile,
) -> AppResult<Material> {
    if !is_canvas_file_url(&file.download_url) {
        canvas_file_repo::set_state(
            &db.lock(),
            &file.id,
            canvas_file_repo::STATE_UNSUPPORTED,
            None,
            Some("下载地址不在学校 Canvas 域名下"),
        )?;
        return Err(AppError::invalid("下载地址不在学校 Canvas 域名下"));
    }

    // Keep the original display name: the importer derives title and kind from
    // it, and a UUID file name would surface as an untitled "other" material.
    let dir = paths
        .cache_dir
        .join("canvas-downloads")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::Message(format!("无法创建下载临时目录：{e}")))?;
    let temp = dir.join(safe_file_name(&file.display_name));

    db.with(|c| {
        canvas_file_repo::set_state(c, &file.id, canvas_file_repo::STATE_DOWNLOADING, None, None)
    })?;

    let fetched = fetch_with_session(runtime, app, file, &temp).await;
    match fetched {
        Ok(()) => {
            let result = import_downloaded(db, paths, file, &temp);
            let _ = std::fs::remove_file(&temp);
            let _ = std::fs::remove_dir(dir);
            match result {
                Ok(material) => {
                    db.with(|c| {
                        canvas_file_repo::set_state(
                            c,
                            &file.id,
                            canvas_file_repo::STATE_DOWNLOADED,
                            Some(&material.id),
                            None,
                        )
                    })?;
                    Ok(material)
                }
                Err(error) => {
                    db.with(|c| {
                        canvas_file_repo::set_state(
                            c,
                            &file.id,
                            canvas_file_repo::STATE_FAILED,
                            None,
                            Some(&error.to_string()),
                        )
                    })?;
                    Err(error)
                }
            }
        }
        Err(message) => {
            let _ = std::fs::remove_file(&temp);
            let _ = std::fs::remove_dir(dir);
            db.with(|c| {
                canvas_file_repo::set_state(
                    c,
                    &file.id,
                    canvas_file_repo::STATE_FAILED,
                    None,
                    Some(&message.to_string()),
                )
            })?;
            Err(message)
        }
    }
}

/// Fetch the file with the Canvas window's session cookies.
async fn fetch_with_session(
    runtime: &CanvasRuntime,
    app: &AppHandle,
    file: &CanvasFile,
    destination: &Path,
) -> AppResult<()> {
    let url = tauri::Url::parse(&file.download_url)
        .map_err(|e| AppError::invalid(format!("下载地址无效：{e}")))?;
    let window = runtime.ensure_window(app, &format!("{}/", runtime.base_url))?;

    // The window is created lazily; its cookie store is only populated once the
    // page has actually loaded, so poll briefly instead of failing on race.
    let mut cookies = Vec::new();
    for _ in 0..20 {
        cookies = window.cookies_for_url(url.clone()).unwrap_or_default();
        if !cookies.is_empty() {
            break;
        }
        if !runtime.wait_for_canvas_page(&window, 2).await {
            // Not on Canvas yet: nudge it there and keep waiting.
            if let Ok(base) = tauri::Url::parse(&format!("{}/", runtime.base_url)) {
                let _ = window.navigate(base);
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    if cookies.is_empty() {
        return Err(AppError::Message(
            "没有找到 Canvas 会话 Cookie，请先在 Canvas 窗口完成登录".into(),
        ));
    }
    let cookie_header = cookies
        .iter()
        .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
        .collect::<Vec<_>>()
        .join("; ");

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| AppError::Message(format!("无法创建下载客户端：{e}")))?;

    let response = client
        .get(url)
        .header(reqwest::header::COOKIE, cookie_header)
        .header(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) CanvasAIAssistant",
        )
        .send()
        .await
        .map_err(|e| AppError::Message(format!("下载请求失败：{e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(AppError::Message(format!(
            "Canvas 返回 {status}（会话可能已过期，请重新登录）"
        )));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.starts_with("text/html") {
        return Err(AppError::Message(
            "Canvas 返回的是网页而不是文件（多半是登录已失效）".into(),
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::Message(format!("读取下载内容失败：{e}")))?;
    if bytes.is_empty() {
        return Err(AppError::Message("下载到的文件是空的".into()));
    }
    std::fs::write(destination, &bytes)
        .map_err(|e| AppError::Message(format!("无法写入临时文件：{e}")))?;
    Ok(())
}

/// Canvas names are user data: strip separators and control characters so the
/// temp file stays inside its own directory.
fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "canvas-file".to_string()
    } else if trimmed.chars().count() > 120 {
        let cut: String = trimmed.chars().take(120).collect();
        cut
    } else {
        trimmed
    }
}
/// Import a finished download into the content-addressed store.
fn import_downloaded(
    db: &Db,
    paths: &AppPaths,
    file: &CanvasFile,
    downloaded: &Path,
) -> AppResult<Material> {
    let metadata = std::fs::metadata(downloaded)
        .map_err(|e| AppError::Message(format!("下载文件不可读：{e}")))?;
    if metadata.len() == 0 {
        return Err(AppError::Message("下载到的文件是空的".into()));
    }
    let imported = crate::documents::import::import_file(
        db,
        paths,
        downloaded,
        Some(&file.course_id),
        None,
        "canvas",
        Some(&file.canvas_file_id),
        Some(&file.download_url),
    )?;
    Ok(imported.material)
}

/// Result of a batch run: the imported materials plus a per-file report.
pub struct BatchResult {
    pub materials: Vec<Material>,
    pub report: crate::domain::DownloadReport,
}

/// Download every pending file for `course_id` (or all courses), sequentially.
///
/// Sequential keeps memory flat, makes progress meaningful, and avoids hammering
/// the school's file service.
pub async fn download_pending(
    db: &Db,
    paths: &Arc<AppPaths>,
    runtime: &CanvasRuntime,
    app: &AppHandle,
    course_id: Option<&str>,
    limit: i64,
) -> BatchResult {
    let mut result = BatchResult {
        materials: Vec::new(),
        report: crate::domain::DownloadReport::default(),
    };

    // A run killed mid-download would otherwise leave rows stuck "downloading".
    let _ = db.with(|c| canvas_file_repo::reset_stale_downloads(c));

    let files = match db.with(|c| canvas_file_repo::list_pending(c, course_id)) {
        Ok(files) => files,
        Err(error) => {
            result.report.errors.push(error.to_string());
            return result;
        }
    };

    for file in files.into_iter().take(limit.max(1) as usize) {
        // Videos and huge archives would waste bandwidth and can never be
        // indexed, so they are marked unsupported instead of retried forever.
        if let Err(reason) = check_downloadable(&file) {
            let _ = db.with(|c| {
                canvas_file_repo::set_state(
                    c,
                    &file.id,
                    canvas_file_repo::STATE_UNSUPPORTED,
                    None,
                    Some(&reason),
                )
            });
            result.report.skipped += 1;
            continue;
        }
        match download_one(db, paths, runtime, app, &file).await {
            Ok(material) => {
                result.report.downloaded += 1;
                result.materials.push(material);
            }
            Err(error) => {
                result.report.failed += 1;
                if result.report.errors.len() < 10 {
                    result
                        .report
                        .errors
                        .push(format!("{}：{error}", file.display_name));
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: i64) -> CanvasFile {
        CanvasFile {
            id: "f".into(),
            course_id: "c".into(),
            canvas_file_id: "1".into(),
            display_name: name.into(),
            content_type: Some("application/pdf".into()),
            byte_size: Some(size),
            download_url: "https://hkust-gz.instructure.com/files/1/download".into(),
            state: "pending".into(),
            material_id: None,
            error: None,
            remote_updated_at: None,
            folder_path: None,
            updated_at: "t".into(),
        }
    }

    #[test]
    fn oversized_and_unparseable_files_are_refused() {
        assert!(check_downloadable(&file("lecture.pdf", 1024)).is_ok());
        assert!(check_downloadable(&file("lecture.pdf", MAX_BYTES + 1))
            .unwrap_err()
            .contains("文件过大"));
        assert!(check_downloadable(&file("lecture.mp4", 1024))
            .unwrap_err()
            .contains("暂不支持"));
        assert!(check_downloadable(&file("no-extension", 1024)).is_err());
    }

    #[test]
    fn only_school_canvas_file_urls_are_allowed() {
        assert!(is_canvas_file_url(
            "https://hkust-gz.instructure.com/files/1/download"
        ));
        assert!(is_canvas_file_url(
            "https://hkust-gz.instructure.com/courses/1/files/2/download"
        ));
        assert!(!is_canvas_file_url(
            "https://evil.example.com/files/1/download"
        ));
        assert!(!is_canvas_file_url(
            "https://hkust-gz.instructure.com/courses/1/pages/home"
        ));
        assert!(!is_canvas_file_url("javascript:alert(1)"));
    }
}
