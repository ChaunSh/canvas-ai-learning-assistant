//! Material import: hashing, de-duplication, secure storage and ZIP expansion.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::documents::{archive, detect_kind};
use crate::error::{AppError, AppResult};
use crate::paths::AppPaths;
use crate::security::paths_guard;
use crate::storage::{document_repo, sync_repo, Db};
use crate::domain::Material;

/// Largest single file we accept. Guardrail against exhausting memory/disk on a
/// mobile device (§54).
pub const MAX_MATERIAL_BYTES: u64 = 300 * 1024 * 1024;

#[derive(Debug)]
pub struct ImportedFile {
    pub material: Material,
    pub duplicate: bool,
}

/// Copy `source` into the content-addressed store and create the material row.
pub fn import_file(
    db: &Db,
    paths: &AppPaths,
    source: &Path,
    course_id: Option<&str>,
    module_id: Option<&str>,
    origin: &str,
    external_id: Option<&str>,
    remote_url: Option<&str>,
) -> AppResult<ImportedFile> {
    let metadata = std::fs::metadata(source)
        .map_err(|e| AppError::invalid(format!("无法读取文件 {}：{e}", source.display())))?;
    if !metadata.is_file() {
        return Err(AppError::invalid(format!(
            "{} 不是常规文件",
            source.display()
        )));
    }
    if metadata.len() > MAX_MATERIAL_BYTES {
        return Err(AppError::invalid(format!(
            "文件过大（{} MB），上限为 {} MB",
            metadata.len() / 1024 / 1024,
            MAX_MATERIAL_BYTES / 1024 / 1024
        )));
    }
    if metadata.len() == 0 {
        return Err(AppError::invalid("文件为空"));
    }

    let filename = paths_guard::sanitize_filename(
        source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file"),
    );
    let kind = detect_kind(&filename, None).to_string();
    let mime = crate::documents::extract::guess_mime(&filename);

    // Hash the source first: an existing identical file is reused without copying.
    let sha256 = hash_file(source)?;
    let existing = db.with(|c| document_repo::find_by_sha(c, &sha256))?;
    if let Some(material) = existing {
        return Ok(ImportedFile {
            material,
            duplicate: true,
        });
    }

    let extension = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let destination = paths.material_blob_path(&sha256, &extension);
    if !destination.exists() {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        copy_atomic(source, &destination)?;
    }
    let byte_size = std::fs::metadata(&destination)?.len() as i64;

    let title = strip_extension(&filename);
    let material = db.with(|c| {
        document_repo::create(
            c,
            &document_repo::MaterialSeed {
                course_id: course_id.map(|s| s.to_string()),
                module_id: module_id.map(|s| s.to_string()),
                external_id: external_id.map(|s| s.to_string()),
                title,
                filename: filename.clone(),
                mime_type: mime,
                kind,
                byte_size,
                sha256: sha256.clone(),
                storage_path: destination.to_string_lossy().to_string(),
                source: origin.to_string(),
                remote_url: remote_url.map(|s| s.to_string()),
            },
        )
    })?;

    if let (Some(course), true) = (course_id, true) {
        db.with(|c| crate::storage::course_repo::refresh_module_counts(c, course))?;
    }
    Ok(ImportedFile {
        material,
        duplicate: false,
    })
}

/// Expand a ZIP into a temporary directory and import every supported member.
pub fn import_zip(
    db: &Db,
    paths: &AppPaths,
    source: &Path,
    course_id: Option<&str>,
    module_id: Option<&str>,
    origin: &str,
) -> AppResult<(Vec<Material>, Vec<String>)> {
    let temp = tempfile::Builder::new()
        .prefix("caa-zip-")
        .tempdir()
        .map_err(|e| AppError::Message(format!("无法创建临时目录：{e}")))?;
    let target = temp.path().join("contents");
    archive::extract_zip(source, &target, &archive::ZipLimits::default())?;

    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    let mut stack: Vec<PathBuf> = vec![target];
    let mut budget = 500usize;

    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if budget == 0 {
                skipped.push("压缩包内条目过多，部分内容已跳过".to_string());
                break;
            }
            budget -= 1;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let kind = detect_kind(name, None);
            // Nested archives are not expanded recursively (bomb protection).
            if matches!(kind, "zip" | "other" | "ics") {
                skipped.push(format!("{name}：类型不支持自动导入"));
                continue;
            }
            match import_file(db, paths, &path, course_id, module_id, origin, None, None) {
                Ok(result) => {
                    if result.duplicate {
                        skipped.push(format!("{name}：已存在相同内容的课件"));
                    } else {
                        imported.push(result.material);
                    }
                }
                Err(e) => skipped.push(format!("{name}：{e}")),
            }
        }
    }

    temp.close().ok();
    Ok((imported, skipped))
}

fn hash_file(path: &Path) -> AppResult<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn copy_atomic(source: &Path, destination: &Path) -> AppResult<()> {
    let partial = destination.with_extension("partial");
    {
        let mut input = std::fs::File::open(source)?;
        let mut output = std::fs::File::create(&partial)?;
        let mut buffer = [0u8; 128 * 1024];
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
        }
        output.flush()?;
    }
    std::fs::rename(&partial, destination)?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn strip_extension(filename: &str) -> String {
    match filename.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => filename.to_string(),
    }
}

/// Record the import job lifecycle for the sync log.
pub fn record_import(
    db: &Db,
    kind: &str,
    source: Option<&str>,
    processed: i64,
    total: i64,
    message: Option<&str>,
) -> AppResult<()> {
    db.with(|c| {
        let job = sync_repo::create_import_job(c, kind, source)?;
        sync_repo::update_import_job(c, &job, "completed", processed, total, message)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    fn temp_paths(name: &str) -> AppPaths {
        let dir = std::env::temp_dir().join(format!("caa-import-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        AppPaths::from_data_dir(dir).unwrap()
    }

    fn write(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn import_copies_and_dedupes() {
        let paths = temp_paths("dedupe");
        let db = Db::open_in_memory().unwrap();
        let source = paths.data_dir.join("incoming/lecture1.pdf");
        write(&source, b"%PDF-1.4 fake content");

        let first = import_file(&db, &paths, &source, None, None, "import", None, None).unwrap();
        assert!(!first.duplicate);
        assert_eq!(first.material.kind, "pdf");
        assert_eq!(first.material.title, "lecture1");
        assert!(std::path::Path::new(&first.material.storage_path).exists());

        let second = import_file(&db, &paths, &source, None, None, "import", None, None).unwrap();
        assert!(second.duplicate);
        assert_eq!(second.material.id, first.material.id);
    }

    #[test]
    fn import_rejects_empty_and_missing() {
        let paths = temp_paths("invalid");
        let db = Db::open_in_memory().unwrap();
        let empty = paths.data_dir.join("empty.pdf");
        write(&empty, b"");
        assert_eq!(
            import_file(&db, &paths, &empty, None, None, "import", None, None)
                .unwrap_err()
                .code(),
            "invalid_input"
        );
        assert!(import_file(
            &db,
            &paths,
            &paths.data_dir.join("nope.pdf"),
            None,
            None,
            "import",
            None,
            None
        )
        .is_err());
    }

    #[test]
    fn sanitises_dangerous_filenames() {
        let paths = temp_paths("names");
        let db = Db::open_in_memory().unwrap();
        let source = paths.data_dir.join("incoming/a:b*c?.md");
        write(&source, b"# hi");
        let result = import_file(&db, &paths, &source, None, None, "import", None, None).unwrap();
        assert_eq!(result.material.filename, "a_b_c_.md");
        assert_eq!(result.material.kind, "markdown");
    }

    #[test]
    fn title_strips_extension() {
        assert_eq!(strip_extension("lecture.pdf"), "lecture");
        assert_eq!(strip_extension("no_ext"), "no_ext");
        assert_eq!(strip_extension(".hidden"), ".hidden");
    }
}
