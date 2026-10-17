use serde::{Serialize, Serializer};

/// Every failure surfaced to the frontend carries a stable machine-readable
/// `code` plus a human message, so the UI can localise and branch without
/// string-matching English text.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    InvalidInput(String),

    #[error("{0}")]
    Conflict(String),

    /// The operation exists but this platform / build cannot perform it.
    #[error("{0}")]
    Unsupported(String),

    #[error("{0}")]
    Cancelled(String),

    /// Upstream AI provider failure (network, HTTP status, protocol).
    #[error("{0}")]
    Ai(String),

    /// Upstream Canvas failure or unsupported page shape.
    #[error("{0}")]
    Canvas(String),

    #[error("数据库错误：{0}")]
    Db(#[from] rusqlite::Error),

    #[error("文件读写错误：{0}")]
    Io(#[from] std::io::Error),

    #[error("数据序列化错误：{0}")]
    Json(#[from] serde_json::Error),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Message(_) => "internal",
            AppError::NotFound(_) => "not_found",
            AppError::InvalidInput(_) => "invalid_input",
            AppError::Conflict(_) => "conflict",
            AppError::Unsupported(_) => "unsupported",
            AppError::Cancelled(_) => "cancelled",
            AppError::Ai(_) => "ai_error",
            AppError::Canvas(_) => "canvas_error",
            AppError::Db(_) => "db_error",
            AppError::Io(_) => "io_error",
            AppError::Json(_) => "serialization_error",
        }
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        AppError::InvalidInput(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        AppError::NotFound(msg.into())
    }

    pub fn msg(msg: impl Into<String>) -> Self {
        AppError::Message(msg.into())
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

impl From<tauri::Error> for AppError {
    fn from(value: tauri::Error) -> Self {
        AppError::Message(format!("应用接口错误：{value}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Convenience so command bodies can `ok_or_not_found(...)`.
pub trait OptionExt<T> {
    fn or_not_found(self, msg: impl Into<String>) -> AppResult<T>;
}

impl<T> OptionExt<T> for Option<T> {
    fn or_not_found(self, msg: impl Into<String>) -> AppResult<T> {
        self.ok_or_else(|| AppError::NotFound(msg.into()))
    }
}
