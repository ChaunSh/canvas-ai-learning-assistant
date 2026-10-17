//! Secret storage for AI API keys.
//!
//! Requirements (task specification §52): keys must never live in the SQLite
//! database, in localStorage, in source, in logs or in a plain config file.
//!
//! * Windows  -> Windows Credential Manager, i.e. the OS credential vault, per
//!               user. Called through a small hand-written FFI declaration so the
//!               ABI does not drift with the `windows` crate's out-parameter
//!               conventions.
//! * Android  -> Android Keystore-backed AES-GCM (see `android_keystore`).
//! * Other    -> an explicitly-labelled restricted file. This path exists so the
//!               app still runs for development; the UI shows the weaker backing
//!               instead of pretending it is secure.

#[cfg(not(windows))]
use std::collections::HashMap;
use std::path::Path;
#[cfg(not(windows))]
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(windows)]
use crate::error::AppError;
use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretBacking {
    WindowsCredentialManager,
    AndroidKeystore,
    /// Not hardware-backed; surfaced in the UI as a limitation.
    RestrictedFile,
}

impl SecretBacking {
    pub fn as_str(&self) -> &'static str {
        match self {
            SecretBacking::WindowsCredentialManager => "windows-credential-manager",
            SecretBacking::AndroidKeystore => "android-keystore",
            SecretBacking::RestrictedFile => "restricted-file",
        }
    }

    /// Human-facing description used by the settings screen.
    pub fn label(&self) -> &'static str {
        match self {
            SecretBacking::WindowsCredentialManager => "Windows 凭据管理器",
            SecretBacking::AndroidKeystore => "Android Keystore（AES-GCM）",
            SecretBacking::RestrictedFile => "受限本地文件（非硬件加密，仅开发用途）",
        }
    }

    pub fn is_secure(&self) -> bool {
        !matches!(self, SecretBacking::RestrictedFile)
    }
}

pub trait SecretStore: Send + Sync {
    fn backing(&self) -> SecretBacking;
    fn set(&self, key: &str, value: &str) -> AppResult<()>;
    fn get(&self, key: &str) -> AppResult<Option<String>>;
    fn delete(&self, key: &str) -> AppResult<()>;
    /// Whether a secret exists, without materialising it.
    fn contains(&self, key: &str) -> AppResult<bool> {
        Ok(self.get(key)?.is_some())
    }
}

/// Credential target prefix. Keeping it stable preserves keys across upgrades.
pub const TARGET_PREFIX: &str = "CanvasAIAssistant/ai-provider/";

pub fn provider_key(provider_id: &str) -> String {
    format!("{TARGET_PREFIX}{provider_id}")
}

/// Build the platform-appropriate store.
pub fn create(data_dir: &Path) -> Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        let _ = data_dir;
        Arc::new(WindowsCredentialStore)
    }
    #[cfg(target_os = "android")]
    {
        Arc::new(android_keystore::AndroidKeystoreStore::new(
            data_dir.join("secrets.bin"),
        ))
    }
    #[cfg(all(not(windows), not(target_os = "android")))]
    {
        Arc::new(FileSecretStore::new(data_dir.join("secrets.json")))
    }
}

// ---------------------------------------------------------------------------
// Windows Credential Manager (advapi32)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod wincred {
    use std::ffi::c_void;

    pub const CRED_TYPE_GENERIC: u32 = 1;
    pub const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
    pub const ERROR_NOT_FOUND: i32 = 1168;

    #[repr(C)]
    #[derive(Default)]
    pub struct FileTime {
        pub low: u32,
        pub high: u32,
    }

    /// Mirror of the Win32 `CREDENTIALW` structure.
    #[repr(C)]
    pub struct CredentialW {
        pub flags: u32,
        pub kind: u32,
        pub target_name: *mut u16,
        pub comment: *mut u16,
        pub last_written: FileTime,
        pub credential_blob_size: u32,
        pub credential_blob: *mut u8,
        pub persist: u32,
        pub attribute_count: u32,
        pub attributes: *mut c_void,
        pub target_alias: *mut u16,
        pub user_name: *mut u16,
    }

    impl Default for CredentialW {
        fn default() -> Self {
            Self {
                flags: 0,
                kind: 0,
                target_name: std::ptr::null_mut(),
                comment: std::ptr::null_mut(),
                last_written: FileTime::default(),
                credential_blob_size: 0,
                credential_blob: std::ptr::null_mut(),
                persist: 0,
                attribute_count: 0,
                attributes: std::ptr::null_mut(),
                target_alias: std::ptr::null_mut(),
                user_name: std::ptr::null_mut(),
            }
        }
    }

    #[link(name = "advapi32")]
    extern "system" {
        pub fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
        pub fn CredReadW(
            target_name: *const u16,
            kind: u32,
            flags: u32,
            credential: *mut *mut CredentialW,
        ) -> i32;
        pub fn CredDeleteW(target_name: *const u16, kind: u32, flags: u32) -> i32;
        pub fn CredFree(buffer: *const c_void);
        pub fn GetLastError() -> u32;
    }

    /// Win32 BOOL is `int`; zero means failure.
    pub fn failed(result: i32) -> bool {
        result == 0
    }

    pub fn last_error() -> (u32, String) {
        let code = unsafe { GetLastError() };
        (code, std::io::Error::from_raw_os_error(code as i32).to_string())
    }
}

#[cfg(windows)]
pub struct WindowsCredentialStore;

#[cfg(windows)]
impl SecretStore for WindowsCredentialStore {
    fn backing(&self) -> SecretBacking {
        SecretBacking::WindowsCredentialManager
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        use wincred::*;

        let mut target: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
        let mut blob: Vec<u8> = value.as_bytes().to_vec();
        let credential = CredentialW {
            kind: CRED_TYPE_GENERIC,
            target_name: target.as_mut_ptr(),
            credential_blob_size: blob.len() as u32,
            credential_blob: blob.as_mut_ptr(),
            persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };

        let result = unsafe { CredWriteW(&credential, 0) };
        // Do not leave the plaintext copy in this process longer than necessary.
        blob.iter_mut().for_each(|b| *b = 0);
        if failed(result) {
            let (code, message) = last_error();
            return Err(AppError::Message(format!(
                "写入 Windows 凭据管理器失败（错误 {code}）：{message}"
            )));
        }
        Ok(())
    }

    fn get(&self, key: &str) -> AppResult<Option<String>> {
        use wincred::*;

        let target: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
        let mut pointer: *mut CredentialW = std::ptr::null_mut();
        let result = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) };
        if failed(result) {
            let (code, _) = last_error();
            if code as i32 == ERROR_NOT_FOUND {
                return Ok(None);
            }
            // Any other failure is reported as "no secret" but logged, so a
            // broken vault never blocks the whole settings screen.
            tracing::warn!(code, key, "CredReadW failed");
            return Ok(None);
        }
        if pointer.is_null() {
            return Ok(None);
        }

        let value = unsafe {
            let credential = &*pointer;
            let bytes = if credential.credential_blob.is_null() {
                &[][..]
            } else {
                std::slice::from_raw_parts(
                    credential.credential_blob,
                    credential.credential_blob_size as usize,
                )
            };
            let text = String::from_utf8_lossy(bytes).into_owned();
            CredFree(pointer as *const core::ffi::c_void);
            text
        };
        Ok(Some(value))
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        use wincred::*;
        let target: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
        let result = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if failed(result) {
            let (code, _) = last_error();
            if code as i32 != ERROR_NOT_FOUND {
                tracing::debug!(code, key, "CredDeleteW failed");
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Restricted file store (development / unsupported platforms)
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
pub struct FileSecretStore {
    path: PathBuf,
}

#[cfg(not(windows))]
impl FileSecretStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> AppResult<HashMap<String, String>> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let raw = std::fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }

    fn save(&self, map: &HashMap<String, String>) -> AppResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string(map)?;
        std::fs::write(&self.path, json)?;
        restrict_permissions(&self.path)?;
        Ok(())
    }
}

#[cfg(not(windows))]
impl SecretStore for FileSecretStore {
    fn backing(&self) -> SecretBacking {
        SecretBacking::RestrictedFile
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        let mut map = self.load()?;
        map.insert(key.to_string(), value.to_string());
        self.save(&map)
    }

    fn get(&self, key: &str) -> AppResult<Option<String>> {
        Ok(self.load()?.get(key).cloned())
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        let mut map = self.load()?;
        map.remove(key);
        self.save(&map)
    }
}

#[cfg(all(not(windows), unix))]
fn restrict_permissions(path: &Path) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(all(not(windows), not(unix)))]
fn restrict_permissions(_path: &Path) -> AppResult<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Android Keystore
// ---------------------------------------------------------------------------

#[cfg(target_os = "android")]
pub mod android_keystore;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_key_is_stable_and_namespaced() {
        assert_eq!(provider_key("abc"), "CanvasAIAssistant/ai-provider/abc");
    }

    #[test]
    fn backing_labels_are_explicit() {
        assert!(SecretBacking::WindowsCredentialManager.is_secure());
        assert!(SecretBacking::AndroidKeystore.is_secure());
        assert!(!SecretBacking::RestrictedFile.is_secure());
        assert!(SecretBacking::RestrictedFile.label().contains("开发"));
    }

    #[cfg(not(windows))]
    #[test]
    fn file_store_roundtrip() {
        let dir = std::env::temp_dir().join("caa-secret-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = FileSecretStore::new(dir.join("secrets.json"));
        assert!(store.get("k").unwrap().is_none());
        store.set("k", "v").unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some("v"));
        store.set("k", "v2").unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some("v2"));
        store.delete("k").unwrap();
        assert!(store.get("k").unwrap().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_credential_roundtrip() {
        let store = WindowsCredentialStore;
        let key = provider_key("caa-selftest");
        store.set(&key, "secret-value").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("secret-value"));
        store.delete(&key).unwrap();
        assert!(store.get(&key).unwrap().is_none());
    }
}
