//! Constrain filesystem access to the application's own data directory.
//!
//! Tauri commands accept paths from the frontend. Even though the frontend is
//! trusted, the cost of validating every path is low and it prevents a logic bug
//! (or a future remote-content surface) from turning into arbitrary file access
//! (task specification §51 "不暴露文件系统的任意访问能力").

use std::path::{Component, Path, PathBuf};

use crate::error::{AppError, AppResult};

/// Normalise a path lexically: resolve `.` and `..` without touching the disk.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Ensure `candidate` (after normalisation) stays inside `root`.
pub fn ensure_within(root: &Path, candidate: &Path) -> AppResult<PathBuf> {
    let root_norm = normalize(root);
    let candidate_norm = if candidate.is_absolute() {
        normalize(candidate)
    } else {
        normalize(&root_norm.join(candidate))
    };

    if !candidate_norm.starts_with(&root_norm) {
        return Err(AppError::invalid("路径超出应用数据目录，已拒绝访问"));
    }
    Ok(candidate_norm)
}

/// Reject filenames that could escape a directory or break tooling.
pub fn sanitize_filename(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file")
        .trim();
    let cleaned: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\0' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim_matches(['.', ' ']).to_string();
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.chars().take(180).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_resolves_parent_segments() {
        assert_eq!(normalize(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize(Path::new("/a/./b")), PathBuf::from("/a/b"));
    }

    #[test]
    fn rejects_escaping_paths() {
        let root = PathBuf::from("C:/data/app");
        assert!(ensure_within(&root, Path::new("C:/data/app/materials/a.pdf")).is_ok());
        assert!(ensure_within(&root, Path::new("C:/data/app/../secret.txt")).is_err());
        assert!(ensure_within(&root, Path::new("C:/Windows/system32/x.dll")).is_err());
    }

    #[test]
    fn relative_paths_are_resolved_against_root() {
        let root = PathBuf::from("C:/data/app");
        let resolved = ensure_within(&root, Path::new("materials/a.pdf")).unwrap();
        assert_eq!(resolved, PathBuf::from("C:/data/app/materials/a.pdf"));
    }

    #[test]
    fn filenames_are_sanitised() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a:b*c?.pdf"), "a_b_c_.pdf");
        assert_eq!(sanitize_filename("   "), "file");
        assert_eq!(sanitize_filename(".."), "file");
        assert_eq!(sanitize_filename("正常文件名.pdf"), "正常文件名.pdf");
    }
}
