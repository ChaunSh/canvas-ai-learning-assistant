use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

/// Filesystem layout owned by the app. Everything lives under the platform's
/// private application-data directory, so uninstalling the app removes it and
/// no other process (or user) can read it on Android.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    /// Original bytes of imported/downloaded materials, content-addressed by sha256.
    pub materials_dir: PathBuf,
    /// Derived artifacts (page images, thumbnails). Safe to delete.
    pub cache_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub exports_dir: PathBuf,
}

impl AppPaths {
    /// Resolve the layout, creating directories as needed.
    pub fn resolve(app: &AppHandle) -> AppResult<Self> {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|e| AppError::Message(format!("无法定位应用数据目录：{e}")))?;
        Self::from_data_dir(data_dir)
    }

    pub fn from_data_dir(data_dir: PathBuf) -> AppResult<Self> {
        let materials_dir = data_dir.join("materials");
        let cache_dir = data_dir.join("cache");
        let backups_dir = data_dir.join("backups");
        let exports_dir = data_dir.join("exports");
        for dir in [&data_dir, &materials_dir, &cache_dir, &backups_dir, &exports_dir] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(Self {
            db_path: data_dir.join("canvas-ai.db"),
            data_dir,
            materials_dir,
            cache_dir,
            backups_dir,
            exports_dir,
        })
    }

    /// Store a material blob under `materials/<first two hex>/<sha256>.<ext>`.
    pub fn material_blob_path(&self, sha256: &str, extension: &str) -> PathBuf {
        let prefix = &sha256[..sha256.len().min(2)];
        let ext = extension.trim_start_matches('.');
        let filename = if ext.is_empty() {
            sha256.to_string()
        } else {
            format!("{sha256}.{ext}")
        };
        self.materials_dir.join(prefix).join(filename)
    }
}
