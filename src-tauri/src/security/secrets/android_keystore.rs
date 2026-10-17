//! Android secret storage backed by the system Keystore.
//!
//! AES-256-GCM keys are generated inside the `AndroidKeyStore` provider and never
//! leave it; only the ciphertext + nonce are kept in the app's private
//! `SharedPreferences`. The JNI surface is intentionally tiny
//! (`put`/`get`/`delete` on `com.hkustgz.canvasai.SecureStore`), so the crypto
//! lives in Kotlin where the Keystore APIs are first-class and this module only
//! marshals strings.
//!
//! Failure policy: if the Keystore is unavailable this backend returns an error
//! rather than silently persisting plaintext. The UI reports the backing store, so
//! a degraded platform is visible instead of hidden.

use std::path::PathBuf;

use jni::objects::{GlobalRef, JObject, JString, JValue};
use jni::JavaVM;
use once_cell::sync::OnceCell;

use crate::error::{AppError, AppResult};
use crate::security::secrets::{SecretBacking, SecretStore};

const HELPER_CLASS: &str = "com/hkustgz/canvasai/SecureStore";

static VM: OnceCell<JavaVM> = OnceCell::new();
static CONTEXT: OnceCell<GlobalRef> = OnceCell::new();

pub struct AndroidKeystoreStore {
    /// Kept for diagnostics; the Keystore backend stores ciphertext in
    /// SharedPreferences rather than a file this module manages.
    path: PathBuf,
}

impl AndroidKeystoreStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl SecretStore for AndroidKeystoreStore {
    fn backing(&self) -> SecretBacking {
        SecretBacking::AndroidKeystore
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        let stored = with_env(|env| {
            let key = env
                .new_string(key)
                .map_err(|e| format!("new_string(key): {e}"))?;
            let value = env
                .new_string(value)
                .map_err(|e| format!("new_string(value): {e}"))?;
            let result = env.call_static_method(
                HELPER_CLASS,
                "put",
                "(Ljava/lang/String;Ljava/lang/String;)Z",
                &[
                    JValue::Object(JObject::from(key)),
                    JValue::Object(JObject::from(value)),
                ],
            );
            as_bool(env, result, "put")
        })?;

        if stored {
            Ok(())
        } else {
            Err(AppError::msg(format!(
                "Android Keystore 拒绝保存密钥（{}）",
                self.path.display()
            )))
        }
    }

    fn get(&self, key: &str) -> AppResult<Option<String>> {
        with_env(|env| {
            let key = env
                .new_string(key)
                .map_err(|e| format!("new_string(key): {e}"))?;
            let result = env.call_static_method(
                HELPER_CLASS,
                "get",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(JObject::from(key))],
            );
            match result {
                Ok(JValue::Object(obj)) if !obj.is_null() => {
                    let text = env
                        .get_string(JString::from(obj))
                        .map_err(|e| format!("get_string: {e}"))?;
                    Ok(Some(text.to_string_lossy().into_owned()))
                }
                Ok(_) => Ok(None),
                Err(error) => Err(describe(env, &error.to_string())),
            }
        })
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        let _ = with_env(|env| {
            let key = env
                .new_string(key)
                .map_err(|e| format!("new_string(key): {e}"))?;
            let result = env.call_static_method(
                HELPER_CLASS,
                "delete",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(JObject::from(key))],
            );
            as_bool(env, result, "delete")
        })?;
        Ok(())
    }
}

/// Run `body` on an attached JNI thread.
fn with_env<T>(body: impl FnOnce(&jni::JNIEnv<'_>) -> Result<T, String>) -> AppResult<T> {
    let vm = ensure_initialised()?;
    let env = vm
        .attach_current_thread()
        .map_err(|e| AppError::msg(format!("无法连接 JVM：{e}")))?;
    body(&env).map_err(AppError::msg)
}

/// Initialise the Kotlin helper exactly once and cache the VM + context.
///
/// The attach guard borrows the `JavaVM`, so it is scoped and dropped before the
/// VM is moved into the cell.
fn ensure_initialised() -> AppResult<&'static JavaVM> {
    if let Some(vm) = VM.get() {
        return Ok(vm);
    }

    let android = ndk_context::android_context();
    let vm_ptr = android.vm() as *mut jni::sys::JavaVM;
    if vm_ptr.is_null() {
        return Err(AppError::msg("Android JVM 句柄不可用"));
    }
    let vm = unsafe { JavaVM::from_raw(vm_ptr) }
        .map_err(|e| AppError::msg(format!("无法获取 JVM：{e}")))?;

    {
        let env = vm
            .attach_current_thread()
            .map_err(|e| AppError::msg(format!("无法连接 JVM：{e}")))?;

        let raw_context = android.context() as jni::sys::jobject;
        if raw_context.is_null() {
            return Err(AppError::msg("Android Context 不可用"));
        }

        env.call_static_method(
            HELPER_CLASS,
            "init",
            "(Landroid/content/Context;)V",
            &[JValue::Object(JObject::from(raw_context))],
        )
        .map_err(|e| AppError::msg(format!("SecureStore.init 失败：{e}")))?;

        let global = env
            .new_global_ref(JObject::from(raw_context))
            .map_err(|e| AppError::msg(format!("无法保存 Context 引用：{e}")))?;
        let _ = CONTEXT.set(global);
    }

    let _ = VM.set(vm);
    VM.get()
        .ok_or_else(|| AppError::msg("Android 密钥存储初始化未完成"))
}

/// Convert a JNI call result into a boolean, clearing any pending Java exception
/// so the thread stays usable.
fn as_bool(
    env: &jni::JNIEnv<'_>,
    result: jni::errors::Result<JValue<'_>>,
    operation: &str,
) -> Result<bool, String> {
    match result {
        // `jboolean` is a `u8`; JNI uses 0/1 rather than Rust's `bool`.
        Ok(JValue::Bool(value)) => Ok(value != 0),
        Ok(_) => Ok(false),
        Err(error) => Err(describe(env, &format!("{operation}: {error}"))),
    }
}

/// Report a pending Java exception, then clear it.
fn describe(env: &jni::JNIEnv<'_>, fallback: &str) -> String {
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
        // The exception text is surfaced by the Kotlin side in logcat; keeping the
        // Rust message short avoids relying on further JNI calls here.
        return format!("{fallback}（Java 异常，详见设备日志）");
    }
    fallback.to_string()
}
