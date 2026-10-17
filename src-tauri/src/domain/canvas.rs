use serde::{Deserialize, Serialize};

/// Connection state machine for the Canvas integration.
///
/// The critical distinction enforced throughout the code base: a successful
/// *web login* (`webAuthenticated`) is not the same as verified *data access*
/// (`dataAccessVerified`), and neither is an OAuth/API token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CanvasSessionState {
    NotConnected,
    LoginRequired,
    LoggingIn,
    WebAuthenticated,
    DataAccessVerified,
    SessionExpired,
    Unsupported,
    Error,
}

impl CanvasSessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            CanvasSessionState::NotConnected => "not-connected",
            CanvasSessionState::LoginRequired => "login-required",
            CanvasSessionState::LoggingIn => "logging-in",
            CanvasSessionState::WebAuthenticated => "web-authenticated",
            CanvasSessionState::DataAccessVerified => "data-access-verified",
            CanvasSessionState::SessionExpired => "session-expired",
            CanvasSessionState::Unsupported => "unsupported",
            CanvasSessionState::Error => "error",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "login-required" => CanvasSessionState::LoginRequired,
            "logging-in" => CanvasSessionState::LoggingIn,
            "web-authenticated" => CanvasSessionState::WebAuthenticated,
            "data-access-verified" => CanvasSessionState::DataAccessVerified,
            "session-expired" => CanvasSessionState::SessionExpired,
            "unsupported" => CanvasSessionState::Unsupported,
            "error" => CanvasSessionState::Error,
            _ => CanvasSessionState::NotConnected,
        }
    }
}

/// Per-dataset capability status (task specification §10.1).
///
/// `supported` is only ever set after a real request returned valid data or a
/// legitimate empty result. Anything untested stays `not-tested`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityStatus {
    Supported,
    Unsupported,
    PermissionDenied,
    LoginRequired,
    NotTested,
    TemporarilyUnavailable,
}

impl CapabilityStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CapabilityStatus::Supported => "supported",
            CapabilityStatus::Unsupported => "unsupported",
            CapabilityStatus::PermissionDenied => "permission-denied",
            CapabilityStatus::LoginRequired => "login-required",
            CapabilityStatus::NotTested => "not-tested",
            CapabilityStatus::TemporarilyUnavailable => "temporarily-unavailable",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "supported" => CapabilityStatus::Supported,
            "unsupported" => CapabilityStatus::Unsupported,
            "permission-denied" => CapabilityStatus::PermissionDenied,
            "login-required" => CapabilityStatus::LoginRequired,
            "temporarily-unavailable" => CapabilityStatus::TemporarilyUnavailable,
            _ => CapabilityStatus::NotTested,
        }
    }

    pub fn is_usable(&self) -> bool {
        matches!(self, CapabilityStatus::Supported)
    }
}

/// The datasets the specification asks us to evaluate independently.
pub const CAPABILITY_KEYS: [&str; 9] = [
    "courses",
    "modules",
    "pages",
    "files",
    "assignments",
    "announcements",
    "calendar",
    "grades",
    "discussions",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub key: String,
    pub status: CapabilityStatus,
    pub detail: Option<String>,
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasConnection {
    pub id: String,
    pub base_url: String,
    pub state: CanvasSessionState,
    pub display_name: Option<String>,
    pub user_ref: Option<String>,
    pub last_checked_at: Option<String>,
    pub last_verified_at: Option<String>,
    pub capabilities: Vec<Capability>,
    /// Human-readable explanation of how the state was determined.
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEntry {
    pub at: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStats {
    pub courses: i64,
    pub modules: i64,
    pub pages: i64,
    pub files: i64,
    pub assignments: i64,
    pub announcements: i64,
    pub events: i64,
    pub grades: i64,
    pub discussions: i64,
    pub materials_downloaded: i64,
    pub bytes_downloaded: i64,
    pub skipped: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRun {
    pub id: String,
    /// queued | running | completed | failed | cancelled | retry_pending
    pub status: String,
    pub scope: String,
    pub course_id: Option<String>,
    pub stats: SyncStats,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgressEvent {
    pub run_id: String,
    pub stage: String,
    pub resource: String,
    pub processed: i64,
    pub total: i64,
    pub message: Option<String>,
}

/// Result of one scripted probe inside the Canvas webview.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub ok: bool,
    pub http_status: Option<i64>,
    /// Parsed JSON body when the endpoint returned JSON.
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    pub error: Option<String>,
    pub url: String,
}

/// Canonical Canvas endpoints we are willing to call from the page context.
/// These are the same JSON endpoints the Canvas web app itself uses for the
/// logged-in user, restricted to GET/read-only operations.
pub fn api_base(path: &str) -> String {
    format!("/api/v1/{path}")
}

pub const CANVAS_BASE_URL: &str = "https://hkust-gz.instructure.com";

/// One Canvas course file and its local download state.
///
/// `download_url` is deliberately not serialised: it is a capability URL for the
/// signed-in webview, and the frontend never needs it — downloads are triggered
/// natively.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasFile {
    pub id: String,
    pub course_id: String,
    pub canvas_file_id: String,
    pub display_name: String,
    pub content_type: Option<String>,
    pub byte_size: Option<i64>,
    #[serde(default, skip_serializing)]
    pub download_url: String,
    /// pending | downloading | downloaded | failed | unsupported
    pub state: String,
    pub material_id: Option<String>,
    pub error: Option<String>,
    pub remote_updated_at: Option<String>,
    /// Canvas folder path, e.g. `Lecture Slides/Week 1`. `None` means the file
    /// sits in the course root (Canvas' own "Files" top level).
    pub folder_path: Option<String>,
    pub updated_at: String,
}

/// Course-grouped view of the Files tab: what is already here and what is not.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseFileGroup {
    pub course_id: String,
    pub course_name: String,
    pub downloaded: Vec<crate::domain::Material>,
    pub pending: Vec<CanvasFile>,
    /// material id -> Canvas folder path, so downloaded files stay inside their
    /// folder in the Files tab.
    pub folder_by_material: std::collections::HashMap<String, Option<String>>,
}

/// Outcome of a download run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadReport {
    pub downloaded: i64,
    pub failed: i64,
    pub skipped: i64,
    /// Human-readable reasons, capped by the caller.
    pub errors: Vec<String>,
}