//! Safe ZIP handling.
//!
//! Course material zips are untrusted input. This module defends against
//! (task specification §54):
//!   * path traversal / drive-absolute paths / `..` segments (Zip Slip)
//!   * symlink entries escaping the destination
//!   * decompression bombs (per-file and total uncompressed byte caps)
//!   * entry-count exhaustion

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct ZipLimits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
    /// Entries beyond this nesting depth in the archive path are rejected.
    pub max_path_depth: usize,
}

impl Default for ZipLimits {
    fn default() -> Self {
        Self {
            max_entries: 2_000,
            // 512 MiB total, 128 MiB per file.
            max_total_bytes: 512 * 1024 * 1024,
            max_file_bytes: 128 * 1024 * 1024,
            max_path_depth: 16,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtractedEntry {
    pub path: PathBuf,
    pub bytes: u64,
}

/// Extract a ZIP archive into `dest`.
pub fn extract_zip(zip_path: &Path, dest: &Path, limits: &ZipLimits) -> AppResult<Vec<ExtractedEntry>> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::invalid(format!("ZIP 文件损坏或格式不支持：{e}")))?;

    if archive.len() > limits.max_entries {
        return Err(AppError::invalid(format!(
            "压缩包条目过多（{} > {}），已拒绝解压",
            archive.len(),
            limits.max_entries
        )));
    }

    std::fs::create_dir_all(dest)?;
    let canonical_dest = dest
        .canonicalize()
        .map_err(|e| AppError::Message(format!("无法解析目标目录：{e}")))?;

    let mut total: u64 = 0;
    let mut out = Vec::new();

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| AppError::invalid(format!("读取压缩包条目失败：{e}")))?;
        // zip 9 returns a Result here; an unreadable name is treated as unsafe.
        let entry_name = entry
            .name()
            .map(|name| name.to_string())
            .unwrap_or_else(|_| "<invalid>".to_string());

        // The `zip` crate sanitises absolute paths, drive letters, `..` and
        // backslash tricks here; `None` means the name is unsafe.
        let Some(relative) = entry.enclosed_name() else {
            tracing::warn!(name = %entry_name, "rejected unsafe zip entry path");
            continue;
        };

        // Defence in depth: `enclosed_name` is platform dependent, so reject any
        // prefix/root component explicitly as well.
        if relative
            .components()
            .any(|c| matches!(c, std::path::Component::Prefix(_) | std::path::Component::RootDir))
            || entry_name.starts_with('/')
            || entry_name.starts_with('\\')
            || entry_name.contains(":\\")
        {
            tracing::warn!(name = %entry_name, "rejected absolute zip entry path");
            continue;
        }

        if relative.components().count() > limits.max_path_depth {
            tracing::warn!(name = %entry_name, "rejected deeply nested zip entry");
            continue;
        }

        // Reject symlinks: their targets can escape the destination.
        if let Some(mode) = entry.unix_mode() {
            let file_type = mode & 0o170000;
            if file_type == 0o120000 {
                tracing::warn!(name = %entry_name, "rejected symlink zip entry");
                continue;
            }
        }

        if entry.is_dir() {
            std::fs::create_dir_all(dest.join(&relative))?;
            continue;
        }

        let declared = entry.size();
        if declared > limits.max_file_bytes {
            return Err(AppError::invalid(format!(
                "压缩包内文件过大（{} 字节），已拒绝解压",
                declared
            )));
        }
        if total + declared > limits.max_total_bytes {
            return Err(AppError::invalid("压缩包解压后总大小超出限制，已拒绝解压"));
        }

        let target = dest.join(&relative);
        // Defence in depth: the resolved parent must stay inside `dest`.
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
            let canonical_parent = parent
                .canonicalize()
                .map_err(|e| AppError::Message(format!("路径解析失败：{e}")))?;
            if !canonical_parent.starts_with(&canonical_dest) {
                tracing::warn!(name = %entry_name, "rejected escaping zip entry");
                continue;
            }
        }

        let written = copy_limited(&mut entry, &target, limits.max_file_bytes)?;
        total += written;
        out.push(ExtractedEntry {
            path: target,
            bytes: written,
        });
    }

    Ok(out)
}

/// Copy at most `limit` bytes, deleting the partial file if the cap is hit.
fn copy_limited<R: Read>(reader: &mut R, target: &Path, limit: u64) -> AppResult<u64> {
    let mut file = std::fs::File::create(target)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        written += read as u64;
        if written > limit {
            drop(file);
            let _ = std::fs::remove_file(target);
            return Err(AppError::invalid("压缩包条目实际大小超出限制"));
        }
        file.write_all(&buffer[..read])?;
    }
    file.flush()?;
    Ok(written)
}

/// Inspect a ZIP without extracting it (for the import preview).
pub fn list_entries(zip_path: &Path) -> AppResult<Vec<(String, u64)>> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::invalid(format!("ZIP 文件损坏或格式不支持：{e}")))?;
    let mut entries = Vec::new();
    for index in 0..archive.len().min(5_000) {
        if let Ok(entry) = archive.by_index(index) {
            if entry.is_dir() {
                continue;
            }
            let name = entry
                .name()
                .map(|name| name.to_string())
                .unwrap_or_default();
            entries.push((name, entry.size()));
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn extracts_normal_entries() {
        let dir = std::env::temp_dir().join("caa-zip-ok");
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("ok.zip");
        write_zip(&zip_path, &[("a.txt", b"hello"), ("sub/b.txt", b"world")]);

        let out_dir = dir.join("out");
        let _ = std::fs::remove_dir_all(&out_dir);
        let entries = extract_zip(&zip_path, &out_dir, &ZipLimits::default()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(std::fs::read_to_string(out_dir.join("a.txt")).unwrap(), "hello");
        assert_eq!(std::fs::read_to_string(out_dir.join("sub/b.txt")).unwrap(), "world");
    }

    #[test]
    fn rejects_traversal_entries() {
        let dir = std::env::temp_dir().join("caa-zip-slip");
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("slip.zip");
        write_zip(&zip_path, &[("../../escaped.txt", b"nope"), ("safe.txt", b"ok")]);

        let out_dir = dir.join("out");
        let _ = std::fs::remove_dir_all(&out_dir);
        let entries = extract_zip(&zip_path, &out_dir, &ZipLimits::default()).unwrap();
        // The traversal entry is dropped; the safe one survives.
        assert_eq!(entries.len(), 1);
        assert!(entries[0].path.ends_with("safe.txt"));
        assert!(!dir.join("escaped.txt").exists());
    }

    #[test]
    fn rejects_absolute_paths() {
        let dir = std::env::temp_dir().join("caa-zip-abs");
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("abs.zip");
        write_zip(&zip_path, &[("C:\\Windows\\evil.dll", b"nope")]);
        let out_dir = dir.join("out");
        let _ = std::fs::remove_dir_all(&out_dir);
        let entries = extract_zip(&zip_path, &out_dir, &ZipLimits::default()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn enforces_per_file_limit() {
        let dir = std::env::temp_dir().join("caa-zip-bomb");
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("bomb.zip");
        let payload = vec![b'x'; 200_000];
        write_zip(&zip_path, &[("big.bin", &payload)]);

        let out_dir = dir.join("out");
        let _ = std::fs::remove_dir_all(&out_dir);
        let limits = ZipLimits {
            max_file_bytes: 1024,
            ..ZipLimits::default()
        };
        let err = extract_zip(&zip_path, &out_dir, &limits).unwrap_err();
        assert_eq!(err.code(), "invalid_input");
    }
}
