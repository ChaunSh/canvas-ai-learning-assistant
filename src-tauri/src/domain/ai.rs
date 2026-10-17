use serde::{Deserialize, Serialize};

/// An OpenAI-compatible provider. The API key never appears here — it lives in
/// the platform secret store keyed by `id` (see `security::SecretStore`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub chat_model: String,
    pub vision_model: Option<String>,
    pub embedding_model: Option<String>,
    pub max_output_tokens: Option<i64>,
    pub temperature: Option<f64>,
    pub timeout_seconds: i64,
    pub is_default: bool,
    pub has_api_key: bool,
    pub last_tested_at: Option<String>,
    pub last_test_ok: Option<bool>,
    pub last_test_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    /// Absent for create, present for update.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub chat_model: String,
    #[serde(default)]
    pub vision_model: Option<String>,
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<i64>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub timeout_seconds: Option<i64>,
    /// When present the key is written to the secret store; when absent an
    /// existing key is kept.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub is_default: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub ok: bool,
    pub latency_ms: i64,
    pub chat_ok: bool,
    pub vision_ok: Option<bool>,
    pub embedding_ok: Option<bool>,
    pub models: Vec<ModelInfo>,
    pub error: Option<String>,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub owned_by: Option<String>,
}

/// Which documents the assistant may retrieve from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatScope {
    /// document | course | courses | all | none
    pub kind: String,
    #[serde(default)]
    pub course_ids: Vec<String>,
    #[serde(default)]
    pub material_ids: Vec<String>,
}

impl ChatScope {
    pub fn all() -> Self {
        ChatScope {
            kind: "all".into(),
            course_ids: vec![],
            material_ids: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub scope: ChatScope,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub id: String,
    pub message_id: String,
    pub document_id: String,
    pub chunk_id: String,
    pub course_id: Option<String>,
    pub filename: String,
    /// 1-based page number as presented in the answer.
    pub page: Option<i64>,
    /// Provenance of `page`: `native` | `slide` | `generated`, so the UI never
    /// presents an app-generated page as the document's own pagination.
    pub page_kind: Option<String>,
    /// 1-based slide number for PPTX sources.
    pub slide: Option<i64>,
    pub section: Option<String>,
    pub excerpt: String,
    pub source_type: String,
    pub score: Option<f64>,
    pub ordinal: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub session_id: String,
    /// system | user | assistant
    pub role: String,
    pub content: String,
    /// streaming | complete | stopped | error
    pub status: String,
    pub error: Option<String>,
    pub retrieval_query: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub citations: Vec<Citation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskInput {
    /// Existing session, or `None` to create one.
    #[serde(default)]
    pub session_id: Option<String>,
    pub question: String,
    #[serde(default)]
    pub scope: Option<ChatScope>,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub top_k: Option<u32>,
    /// When true, retrieval is skipped entirely (pure chat).
    #[serde(default)]
    pub no_retrieval: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAccepted {
    /// Identifier for this generation, used by `cancel_generation` and to
    /// correlate stream events.
    pub request_id: String,
    pub session_id: String,
    pub user_message: ChatMessage,
    pub assistant_message: ChatMessage,
    pub citations: Vec<Citation>,
    /// Number of retrieved chunks fed to the model.
    pub context_chunks: i64,
    pub strategy: String,
}

/// Streamed over the `ai://stream` event channel. `requestId` lets the UI
/// ignore events from a superseded generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatStreamEvent {
    #[serde(rename_all = "camelCase")]
    Started {
        request_id: String,
        session_id: String,
        message_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Delta {
        request_id: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Citations {
        request_id: String,
        citations: Vec<Citation>,
    },
    #[serde(rename_all = "camelCase")]
    Usage {
        request_id: String,
        prompt_tokens: Option<i64>,
        completion_tokens: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    Completed {
        request_id: String,
        message: ChatMessage,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        request_id: String,
        message: String,
        code: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryInput {
    pub material_id: String,
    /// full | concepts | definitions | formulas | theorems | pitfalls | review | self_test | section
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    /// zh | en
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub failures: i64,
}
