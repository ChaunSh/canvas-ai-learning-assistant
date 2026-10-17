use serde::{Deserialize, Serialize};

/// Material file kinds we can classify locally.
pub const MATERIAL_KINDS: [&str; 11] = [
    "pdf", "pptx", "docx", "text", "markdown", "image", "ics", "csv", "json", "zip", "other",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    pub id: String,
    pub course_id: Option<String>,
    pub course_name: Option<String>,
    pub module_id: Option<String>,
    pub external_id: Option<String>,
    pub title: String,
    pub filename: String,
    pub mime_type: Option<String>,
    /// pdf | pptx | docx | text | markdown | image | ics | csv | json | zip | other
    pub kind: String,
    pub byte_size: i64,
    pub sha256: String,
    pub storage_path: String,
    /// canvas | manual | import | ai
    pub source: String,
    pub remote_url: Option<String>,
    /// pending | queued | processing | indexed | partial | failed | skipped
    pub index_status: String,
    pub index_error: Option<String>,
    pub page_count: i64,
    pub slide_count: i64,
    pub char_count: i64,
    pub needs_ocr: bool,
    pub is_favorite: bool,
    pub tags: Vec<String>,
    pub ai_upload_allowed: bool,
    pub last_opened_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Number of indexed chunks; joined for list views.
    #[serde(default)]
    pub chunk_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialVersion {
    pub id: String,
    pub material_id: String,
    pub sha256: String,
    pub byte_size: i64,
    pub storage_path: String,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPage {
    pub id: String,
    pub material_id: String,
    /// 0-based page (PDF) or slide (PPTX) index.
    pub page_index: i64,
    pub label: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub text: String,
    pub char_count: i64,
    pub has_image: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentChunk {
    pub id: String,
    pub material_id: String,
    pub page_index: Option<i64>,
    pub ordinal: i64,
    /// heading | paragraph | list | formula | definition | theorem | code | table | caption | image
    pub kind: String,
    pub section: Option<String>,
    pub content: String,
    pub context_prefix: Option<String>,
    pub token_estimate: i64,
    pub start_char: i64,
    pub end_char: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    pub id: String,
    pub material_id: String,
    pub material_title: Option<String>,
    /// full | section | concepts | definitions | formulas | theorems | pitfalls | review | self_test
    pub kind: String,
    pub scope: Option<String>,
    pub content_md: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub source_sha256: Option<String>,
    pub stale: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// One chunk returned by retrieval, with the metadata needed to render a
/// clickable, page-accurate citation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub chunk_id: String,
    pub material_id: String,
    pub material_title: String,
    pub filename: String,
    pub course_id: Option<String>,
    pub course_name: Option<String>,
    pub kind: String,
    /// 0-based index into pages/slides.
    pub page_index: Option<i64>,
    /// 1-based page shown to the user and used for jump links (`pageIndex + 1`).
    pub page: Option<i64>,
    /// Provenance of `page`: `native` (the file's own pagination), `slide` (a
    /// slide deck), or `generated` (the app paginated a format that has none).
    /// Kept separate so the UI can label a generated page honestly.
    pub page_kind: Option<String>,
    pub slide: Option<i64>,
    pub section: Option<String>,
    pub content: String,
    pub context_prefix: Option<String>,
    pub score: f64,
    /// "fts" | "vector" | "hybrid"
    pub matched_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportInput {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub course_id: Option<String>,
    #[serde(default)]
    pub module_id: Option<String>,
    /// Import a ZIP's supported members instead of storing it as one file.
    #[serde(default)]
    pub expand_archives: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub job_id: String,
    pub imported: Vec<Material>,
    pub skipped: Vec<ImportSkip>,
    pub failed: Vec<ImportFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSkip {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFailure {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialFilter {
    #[serde(default)]
    pub course_id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub favorites_only: bool,
    #[serde(default)]
    pub index_status: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMaterialInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "crate::domain::double_option")]
    pub course_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::domain::double_option")]
    pub module_id: Option<Option<String>>,
    #[serde(default)]
    pub is_favorite: Option<bool>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub ai_upload_allowed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub query: String,
    #[serde(default)]
    pub course_id: Option<String>,
    #[serde(default)]
    pub material_ids: Vec<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    /// "fts" when only keyword search ran, "hybrid" when vectors contributed.
    pub strategy: String,
    pub took_ms: u64,
    /// True when the query produced no usable tokens (e.g. punctuation only).
    pub empty_query: bool,
}

/// Progress events emitted while a document is parsed and indexed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexProgress {
    pub job_id: String,
    pub material_id: String,
    /// queued | extracting | chunking | indexing | embedding | completed | failed | cancelled
    pub stage: String,
    pub progress: f64,
    pub message: Option<String>,
    pub pages: i64,
    pub chunks: i64,
}
