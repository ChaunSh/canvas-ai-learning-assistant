//! AI orchestration: retrieval → prompt → streaming → persistence.
//!
//! This module owns the whole answer pipeline so the Tauri command layer stays
//! a thin adapter (task specification §28).

pub mod openai;
pub mod prompt;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::domain::{
    AiProvider, AskAccepted, ChatMessage, ChatScope, ChatStreamEvent, Citation, ProviderTestResult,
    SearchHit,
};
use crate::error::{AppError, AppResult};
use crate::retrieval;
use crate::retrieval::vector::VectorIndex;
use crate::security::SecretStore;
use crate::storage::{ai_repo, document_repo, Db};

use openai::{ChatMessageInput, ChatRequest, OpenAiClient};

pub const STREAM_EVENT: &str = "ai://stream";

/// Tracks in-flight generations so the UI can stop one without affecting others.
#[derive(Default)]
pub struct AiRuntime {
    active: Mutex<HashMap<String, CancellationToken>>,
}

impl AiRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, request_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.active.lock().insert(request_id.to_string(), token.clone());
        token
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        match self.active.lock().get(request_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    pub fn release(&self, request_id: &str) {
        self.active.lock().remove(request_id);
    }

    pub fn cancel_all(&self) {
        for (_, token) in self.active.lock().drain() {
            token.cancel();
        }
    }

    pub fn active_count(&self) -> usize {
        self.active.lock().len()
    }
}

/// Everything needed to talk to one provider.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub provider_id: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i64>,
    pub timeout_seconds: i64,
}

impl ClientConfig {
    pub fn client<'a>(&self, http: &'a reqwest::Client) -> OpenAiClient<'a> {
        OpenAiClient::new(http, &self.base_url, &self.api_key, self.timeout_seconds)
    }
}

/// Resolve a provider row plus its secret.
pub fn resolve_client(
    db: &Db,
    secrets: &dyn SecretStore,
    provider_id: Option<&str>,
) -> AppResult<(AiProvider, ClientConfig)> {
    let provider = match provider_id {
        Some(id) => ai_repo::get_provider(&db.lock(), id)?,
        None => ai_repo::default_provider(&db.lock())?
            .ok_or_else(|| AppError::invalid("尚未配置 AI 服务，请先在设置中添加 Provider"))?,
    };
    let key = secrets.get(&crate::security::secrets::provider_key(&provider.id))?;
    let api_key = key.ok_or_else(|| {
        AppError::invalid(format!(
            "AI 服务「{}」缺少 API Key，请重新保存配置",
            provider.name
        ))
    })?;
    let config = ClientConfig {
        provider_id: provider.id.clone(),
        base_url: provider.base_url.clone(),
        api_key,
        model: provider.chat_model.clone(),
        temperature: provider.temperature,
        max_tokens: provider.max_output_tokens,
        timeout_seconds: provider.timeout_seconds,
    };
    Ok((provider, config))
}

/// Optionally embed the query for semantic retrieval.
///
/// Returns `None` when no embedding model is configured or the call fails: the
/// answer still proceeds with keyword retrieval, and the response reports which
/// strategy was used rather than silently pretending semantics ran.
pub async fn embed_query(
    db: &Db,
    secrets: &dyn SecretStore,
    http: &reqwest::Client,
    provider_id: Option<&str>,
    query: &str,
) -> Option<(Vec<f32>, String)> {
    let provider = {
        let conn = db.lock();
        match provider_id {
            Some(id) => ai_repo::get_provider(&conn, id).ok(),
            None => ai_repo::default_provider(&conn).ok().flatten(),
        }
    }?;
    let model = provider.embedding_model.clone().filter(|m| !m.trim().is_empty())?;
    let key = secrets
        .get(&crate::security::secrets::provider_key(&provider.id))
        .ok()??;
    let client = OpenAiClient::new(http, &provider.base_url, &key, provider.timeout_seconds);
    match client.embed(&model, &[query.to_string()]).await {
        Ok(mut vectors) if !vectors.is_empty() => Some((vectors.remove(0), model)),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!(error = %e, "query embedding failed; falling back to keyword search");
            let _ = db.with(|c| {
                ai_repo::record_usage(
                    c,
                    Some(&provider.id),
                    Some(&model),
                    "embedding",
                    None,
                    None,
                    None,
                    false,
                    Some(&e.to_string()),
                )
            });
            None
        }
    }
}

/// What the command layer gets back before streaming starts.
pub struct PreparedAnswer {
    pub request_id: String,
    pub session: crate::domain::ChatSession,
    pub user_message: ChatMessage,
    pub assistant_message: ChatMessage,
    pub citations: Vec<Citation>,
    pub context_chunks: i64,
    pub strategy: String,
    pub config: ClientConfig,
    pub request: ChatRequest,
    pub hits: Vec<SearchHit>,
}

pub struct PrepareInput<'a> {
    pub question: &'a str,
    pub session_id: Option<&'a str>,
    pub scope: Option<ChatScope>,
    pub provider_id: Option<&'a str>,
    pub model: Option<&'a str>,
    pub top_k: u32,
    pub no_retrieval: bool,
    pub language: &'a str,
}

/// Retrieve evidence, persist the user turn and build the model request.
///
/// The caller registers the returned `request_id` with [`AiRuntime`] so the
/// generation can be cancelled; keeping registration out of this function makes
/// the cancellation token and the in-flight request the same object.
pub async fn prepare_answer(
    db: &Db,
    secrets: &dyn SecretStore,
    http: &reqwest::Client,
    index: &dyn VectorIndex,
    input: PrepareInput<'_>,
) -> AppResult<PreparedAnswer> {
    let question = input.question.trim();
    if question.is_empty() {
        return Err(AppError::invalid("问题不能为空"));
    }
    if question.chars().count() > 8000 {
        return Err(AppError::invalid("问题过长，请拆分为多个问题"));
    }

    let (provider, config) = resolve_client(db, secrets, input.provider_id)?;
    let model = input
        .model
        .map(|m| m.to_string())
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| config.model.clone());

    // --- session -----------------------------------------------------------
    let (session, scope) = {
        let conn = db.lock();
        match input.session_id {
            Some(id) => {
                let session = ai_repo::get_session(&conn, id)?;
                let scope = input
                    .scope
                    .clone()
                    .unwrap_or_else(|| session.scope.clone());
                (session, scope)
            }
            None => {
                let scope = input.scope.clone().unwrap_or_else(ChatScope::all);
                let session = ai_repo::create_session(
                    &conn,
                    &ai_repo::derive_title(question),
                    &scope,
                    Some(&provider.id),
                    Some(&model),
                )?;
                (session, scope)
            }
        }
    };

    // --- retrieval ---------------------------------------------------------
    let (hits, strategy, context_chunks) = if input.no_retrieval {
        (Vec::new(), "none".to_string(), 0)
    } else {
        let embedding = embed_query(
            db,
            secrets,
            http,
            Some(&provider.id),
            question,
        )
        .await;
        let material_ids = if scope.kind == "none" {
            vec!["__none__".to_string()]
        } else {
            scope.material_ids.clone()
        };
        let course_ids = if scope.kind == "all" || scope.kind == "none" {
            Vec::new()
        } else {
            scope.course_ids.clone()
        };
        let conn = db.lock();
        let outcome = retrieval::search(
            &conn,
            retrieval::RetrievalRequest {
                query: question,
                material_ids: &material_ids,
                course_ids: &course_ids,
                limit: input.top_k.clamp(1, 20) as usize,
                query_embedding: embedding.as_ref().map(|(v, _)| v.as_slice()),
                embedding_model: embedding.as_ref().map(|(_, m)| m.as_str()),
            },
            index,
        )?;
        let count = outcome.hits.len() as i64;
        (outcome.hits, outcome.strategy, count)
    };

    let (context, all_citations) = prompt::build_context(&hits, prompt::DEFAULT_CONTEXT_TOKENS);

    // --- messages ----------------------------------------------------------
    let (user_message, assistant_message) = {
        let conn = db.lock();
        let user = ai_repo::add_message(
            &conn,
            &session.id,
            "user",
            question,
            "complete",
            Some(question),
        )?;
        let assistant =
            ai_repo::add_message(&conn, &session.id, "assistant", "", "streaming", None)?;
        (user, assistant)
    };

    // --- request -----------------------------------------------------------
    let system_prompt = if input.language.starts_with("en") {
        prompt::SYSTEM_PROMPT_EN
    } else {
        prompt::SYSTEM_PROMPT_ZH
    };
    let mut messages = vec![ChatMessageInput {
        role: "system".into(),
        content: json!(system_prompt),
    }];
    // Carry a bounded amount of conversation history for follow-up questions.
    {
        let conn = db.lock();
        let history = ai_repo::recent_turns(&conn, &session.id, 6)?;
        for (role, content) in history {
            if role == "assistant" && content.trim().is_empty() {
                continue;
            }
            messages.push(ChatMessageInput {
                role,
                content: json!(content),
            });
        }
    }
    messages.push(ChatMessageInput {
        role: "user".into(),
        content: json!(prompt::build_user_turn(question, &context, input.language)),
    });

    let request_id = uuid::Uuid::new_v4().to_string();

    Ok(PreparedAnswer {
        request_id,
        session,
        user_message,
        assistant_message,
        citations: all_citations,
        context_chunks,
        strategy,
        request: ChatRequest {
            model,
            messages,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            stream: true,
        },
        config,
        hits,
    })
}

/// Execute a prepared answer, emitting stream events and persisting the result.
pub async fn execute_generation(
    app: &tauri::AppHandle,
    db: Db,
    http: reqwest::Client,
    runtime: Arc<AiRuntime>,
    prepared: PreparedAnswer,
    cancel: CancellationToken,
) -> AppResult<ChatMessage> {
    use tauri::Emitter;

    let request_id = prepared.request_id.clone();
    let session_id = prepared.session.id.clone();
    let message_id = prepared.assistant_message.id.clone();
    let citations = prepared.citations.clone();
    let provided = citations.len();

    let _ = app.emit(
        STREAM_EVENT,
        ChatStreamEvent::Started {
            request_id: request_id.clone(),
            session_id: session_id.clone(),
            message_id: message_id.clone(),
        },
    );

    if !citations.is_empty() {
        let _ = app.emit(
            STREAM_EVENT,
            ChatStreamEvent::Citations {
                request_id: request_id.clone(),
                citations: citations.clone(),
            },
        );
    }

    let client = prepared.config.client(&http);
    let started = Instant::now();
    let app_for_delta = app.clone();
    let request_for_delta = request_id.clone();
    let mut emitted = String::new();

    let stream_result = client
        .chat_stream(&prepared.request, |text| {
            if cancel.is_cancelled() {
                return false;
            }
            emitted.push_str(text);
            let _ = app_for_delta.emit(
                STREAM_EVENT,
                ChatStreamEvent::Delta {
                    request_id: request_for_delta.clone(),
                    text: text.to_string(),
                },
            );
            true
        })
        .await;

    let latency = started.elapsed().as_millis() as i64;

    let outcome = match stream_result {
        Ok(result) if cancel.is_cancelled() || !emitted.is_empty() && result.content != emitted => {
            // Cancelled mid-flight: keep what the user already saw.
            let content = if emitted.is_empty() { result.content } else { emitted.clone() };
            Ok((content, result.prompt_tokens, result.completion_tokens, "stopped"))
        }
        Ok(result) => Ok((
            result.content.clone(),
            result.prompt_tokens,
            result.completion_tokens,
            "complete",
        )),
        Err(_e) if cancel.is_cancelled() => Ok((emitted.clone(), None, None, "stopped")),
        Err(e) => Err(e),
    };

    match outcome {
        Ok((content, prompt_tokens, completion_tokens, status)) => {
            let used = prompt::citations_used(&content, &citations);
            let fabricated = prompt::fabricated_labels(&content, provided);
            if !fabricated.is_empty() {
                tracing::warn!(
                    ?fabricated,
                    "model cited labels that were never provided; not persisted as sources"
                );
            }

            let final_message = {
                let conn = db.lock();
                ai_repo::update_message(&conn, &message_id, &content, status, None)?;
                ai_repo::set_message_usage(&conn, &message_id, prompt_tokens, completion_tokens)?;
                if !used.is_empty() {
                    ai_repo::add_citations(&conn, &message_id, &used)?;
                }
                ai_repo::record_usage(
                    &conn,
                    Some(&prepared.config.provider_id),
                    Some(&prepared.request.model),
                    "chat",
                    prompt_tokens,
                    completion_tokens,
                    Some(latency),
                    true,
                    None,
                )?;
                ai_repo::get_message(&conn, &message_id)?
            };

            let _ = app.emit(
                STREAM_EVENT,
                ChatStreamEvent::Usage {
                    request_id: request_id.clone(),
                    prompt_tokens,
                    completion_tokens,
                },
            );
            let _ = app.emit(
                STREAM_EVENT,
                ChatStreamEvent::Completed {
                    request_id: request_id.clone(),
                    message: final_message.clone(),
                },
            );
            runtime.release(&request_id);
            Ok(final_message)
        }
        Err(error) => {
            let message = error.to_string();
            {
                let conn = db.lock();
                let partial = if emitted.is_empty() { "" } else { &emitted };
                let _ = ai_repo::update_message(&conn, &message_id, partial, "error", Some(&message));
                let _ = ai_repo::record_usage(
                    &conn,
                    Some(&prepared.config.provider_id),
                    Some(&prepared.request.model),
                    "chat",
                    None,
                    None,
                    Some(latency),
                    false,
                    Some(&message),
                );
            }
            let _ = app.emit(
                STREAM_EVENT,
                ChatStreamEvent::Failed {
                    request_id: request_id.clone(),
                    message: message.clone(),
                    code: error.code().to_string(),
                },
            );
            runtime.release(&request_id);
            Err(error)
        }
    }
}

/// Build the `AskAccepted` payload the UI uses to render immediately.
pub fn accepted_from(prepared: &PreparedAnswer) -> AskAccepted {
    AskAccepted {
        request_id: prepared.request_id.clone(),
        session_id: prepared.session.id.clone(),
        user_message: prepared.user_message.clone(),
        assistant_message: prepared.assistant_message.clone(),
        citations: prepared.citations.clone(),
        context_chunks: prepared.context_chunks,
        strategy: prepared.strategy.clone(),
    }
}

/// Verify a provider end to end: model listing, then a tiny chat completion.
pub async fn test_provider(
    http: &reqwest::Client,
    config: &ClientConfig,
    vision_model: Option<&str>,
    embedding_model: Option<&str>,
) -> AppResult<ProviderTestResult> {
    let client = config.client(http);
    let started = Instant::now();

    let models = client.list_models().await.unwrap_or_default();
    let models_ok = !models.is_empty();

    let probe = ChatRequest {
        model: config.model.clone(),
        messages: vec![ChatMessageInput {
            role: "user".into(),
            content: json!("Reply with the single word: ok"),
        }],
        temperature: Some(0.0),
        max_tokens: Some(16),
        stream: false,
    };
    let chat_result = client.chat(&probe).await;
    let latency_ms = started.elapsed().as_millis() as i64;

    let (chat_ok, error) = match &chat_result {
        Ok(_) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };

    // Capability probes are best effort; a failure only marks that capability.
    let vision_ok = match vision_model.filter(|m| !m.trim().is_empty()) {
        Some(model) => Some(
            client
                .chat(&ChatRequest {
                    model: model.to_string(),
                    messages: vec![ChatMessageInput {
                        role: "user".into(),
                        content: json!("ping"),
                    }],
                    temperature: Some(0.0),
                    max_tokens: Some(8),
                    stream: false,
                })
                .await
                .is_ok(),
        ),
        None => None,
    };
    let embedding_ok = match embedding_model.filter(|m| !m.trim().is_empty()) {
        Some(model) => client.embed(model, &["ping".to_string()]).await.ok().map(|v| !v.is_empty()),
        None => None,
    };

    Ok(ProviderTestResult {
        ok: chat_ok,
        latency_ms,
        chat_ok,
        vision_ok,
        embedding_ok,
        models: if models_ok { models } else { Vec::new() },
        error,
        checked_at: crate::time::now_iso(),
    })
}

/// Generate (or return a cached) summary for a material.
pub async fn generate_summary(
    db: &Db,
    secrets: &dyn SecretStore,
    http: &reqwest::Client,
    material_id: &str,
    kind: &str,
    scope: Option<&str>,
    language: &str,
    force: bool,
) -> AppResult<crate::domain::DocumentSummary> {
    let material = db.with(|c| document_repo::get(c, material_id))?;

    if !force {
        let cached = db.with(|c| document_repo::get_summary(c, material_id, kind, scope))?;
        if let Some(summary) = cached {
            if !summary.stale && summary.source_sha256.as_deref() == Some(material.sha256.as_str()) {
                return Ok(summary);
            }
        }
    }

    let chunks = db.with(|c| document_repo::list_chunks(c, material_id))?;
    if chunks.is_empty() {
        return Err(AppError::invalid(
            "该课件尚未建立索引，请先完成解析后再生成总结",
        ));
    }

    let (provider, config) = resolve_client(db, secrets, None)?;
    let client = config.client(http);

    // Compose the source text within a token budget, respecting the section scope.
    let source = compose_summary_source(&chunks, scope, 9000);
    if source.trim().is_empty() {
        return Err(AppError::invalid("指定范围内没有可总结的内容"));
    }

    let instruction = summary_instruction(kind, language);
    let request = ChatRequest {
        model: config.model.clone(),
        messages: vec![
            ChatMessageInput {
                role: "system".into(),
                content: json!(if language.starts_with("en") {
                    prompt::SYSTEM_PROMPT_EN
                } else {
                    prompt::SYSTEM_PROMPT_ZH
                }),
            },
            ChatMessageInput {
                role: "user".into(),
                content: json!(format!(
                    "<课件证据>\n文件名：{}\n\n{source}\n</课件证据>\n\n{instruction}",
                    material.filename
                )),
            },
        ],
        temperature: Some(0.2),
        max_tokens: Some(3000),
        stream: false,
    };

    let started = Instant::now();
    let result = client.chat(&request).await;
    let latency = started.elapsed().as_millis() as i64;
    match result {
        Ok(completion) => {
            let summary = db.with(|c| {
                ai_repo::record_usage(
                    c,
                    Some(&provider.id),
                    Some(&config.model),
                    "summary",
                    completion.prompt_tokens,
                    completion.completion_tokens,
                    Some(latency),
                    true,
                    None,
                )?;
                document_repo::upsert_summary(
                    c,
                    material_id,
                    kind,
                    scope,
                    &completion.content,
                    Some(&provider.id),
                    Some(&config.model),
                    Some(&material.sha256),
                )
            })?;
            Ok(summary)
        }
        Err(error) => {
            let _ = db.with(|c| {
                ai_repo::record_usage(
                    c,
                    Some(&provider.id),
                    Some(&config.model),
                    "summary",
                    None,
                    None,
                    Some(latency),
                    false,
                    Some(&error.to_string()),
                )
            });
            Err(error)
        }
    }
}

/// Assemble the text that will be summarised, honouring an optional section
/// scope and a token budget.
///
/// Pure on purpose: the budget cut-off and scope filter are the two places where
/// a summary can silently end up describing the wrong material, so they are
/// unit-tested rather than only exercised through a live model call.
pub fn compose_summary_source(
    chunks: &[crate::domain::DocumentChunk],
    scope: Option<&str>,
    budget_tokens: i64,
) -> String {
    let mut source = String::new();
    let mut budget = budget_tokens.max(0);
    for chunk in chunks {
        if let Some(scope) = scope {
            if chunk.section.as_deref() != Some(scope) {
                continue;
            }
        }
        // `break`, not `continue`: chunks arrive in reading order, so once one
        // does not fit the rest are noise for the summary anyway.
        if chunk.token_estimate > budget {
            break;
        }
        budget -= chunk.token_estimate;
        if let Some(prefix) = &chunk.context_prefix {
            source.push_str(&format!("【{prefix}】\n"));
        }
        if let Some(section) = &chunk.section {
            source.push_str(&format!("## {section}\n"));
        }
        source.push_str(&chunk.content);
        source.push_str("\n\n");
    }
    source
}

fn summary_instruction(kind: &str, language: &str) -> String {
    let zh = !language.starts_with("en");
    match (kind, zh) {
        ("concepts", true) => "请只整理核心概念：每个概念给出名称、定义、与其他概念的关系，并标注来自哪个章节。".into(),
        ("definitions", true) => "请整理本课件的所有定义，使用「定义 / 符号 / 适用条件」的结构，并标注来源。".into(),
        ("formulas", true) => "请整理本课件的所有公式，使用 LaTeX 表示，说明每个符号含义与适用条件。".into(),
        ("theorems", true) => "请整理本课件的定理、引理与推论，说明条件、结论与证明要点。".into(),
        ("pitfalls", true) => "请列出本课件中最容易出错的知识点，说明错误原因与正确做法。".into(),
        ("review", true) => "请给出一份复习建议：按重要性与依赖关系排序，并估算每部分所需时间。".into(),
        ("self_test", true) => "请生成 8 道自测题（含答案要点），覆盖本课件的重点，难度递进。".into(),
        ("section", true) => "请针对指定章节生成结构化总结，包含要点、公式与易错点。".into(),
        ("full", true) | (_, true) => "请生成完整总结：\n1. 一句话概述\n2. 知识点结构（分章节）\n3. 关键公式与定理\n4. 易错点\n5. 复习建议\n所有结论都要使用 [S1] 形式标注来源。".into(),
        ("concepts", false) => "List only the core concepts: name, definition, relation to other concepts, and the section each came from.".into(),
        ("definitions", false) => "List every definition as Definition / Notation / Conditions, each with its source section.".into(),
        ("formulas", false) => "List every formula in LaTeX with the meaning of each symbol and its conditions of use.".into(),
        ("theorems", false) => "List theorems, lemmas and corollaries with hypotheses, conclusions and proof sketches.".into(),
        ("pitfalls", false) => "List the most common mistakes for this material with the reason and the correct approach.".into(),
        ("review", false) => "Produce a revision plan ordered by importance and dependency, with time estimates.".into(),
        ("self_test", false) => "Write 8 self-test questions with answer keys, increasing in difficulty.".into(),
        ("section", false) => "Summarise the selected section with key points, formulas and pitfalls.".into(),
        _ => "Produce a complete summary: one-line overview, structured key points by section, key formulas and theorems, pitfalls, and revision advice. Cite sources as [S1].".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DocumentChunk;

    fn chunk(section: Option<&str>, tokens: i64, content: &str) -> DocumentChunk {
        DocumentChunk {
            id: format!("c-{content}"),
            material_id: "m1".into(),
            page_index: Some(0),
            ordinal: 0,
            kind: "paragraph".into(),
            section: section.map(|s| s.to_string()),
            content: content.into(),
            context_prefix: Some("第一章".into()),
            token_estimate: tokens,
            start_char: 0,
            end_char: 0,
            created_at: "2026-10-18T00:00:00Z".into(),
        }
    }

    #[test]
    fn summary_source_includes_context_and_section_headers() {
        let chunks = vec![chunk(Some("1.1 极限"), 10, "极限的定义。")];
        let source = compose_summary_source(&chunks, None, 100);
        assert!(source.contains("【第一章】"), "context prefix missing: {source}");
        assert!(source.contains("## 1.1 极限"), "section header missing: {source}");
        assert!(source.contains("极限的定义。"));
    }

    #[test]
    fn summary_source_honours_the_section_scope() {
        let chunks = vec![
            chunk(Some("第一章"), 10, "第一节内容"),
            chunk(Some("第二章"), 10, "第二节内容"),
        ];
        let scoped = compose_summary_source(&chunks, Some("第二章"), 100);
        assert!(scoped.contains("第二节内容"));
        assert!(!scoped.contains("第一节内容"), "scope leaked: {scoped}");

        let unmatched = compose_summary_source(&chunks, Some("第九章"), 100);
        assert!(unmatched.trim().is_empty(), "unknown scope must yield nothing");
    }

    #[test]
    fn summary_source_stops_at_the_token_budget() {
        let chunks = vec![
            chunk(Some("A"), 60, "第一段"),
            chunk(Some("B"), 50, "第二段放不下"),
            chunk(Some("C"), 10, "第三段"),
        ];
        let source = compose_summary_source(&chunks, None, 70);
        assert!(source.contains("第一段"));
        assert!(!source.contains("第二段放不下"), "budget overrun: {source}");
        // Once a chunk does not fit, later (smaller) chunks are not cherry-picked.
        assert!(!source.contains("第三段"));
    }

    #[test]
    fn summary_source_with_zero_budget_is_empty() {
        let chunks = vec![chunk(Some("A"), 1, "内容")];
        assert!(compose_summary_source(&chunks, None, 0).is_empty());
        assert!(compose_summary_source(&[], None, 100).is_empty());
    }

    #[test]
    fn summary_instructions_differ_per_kind_and_language() {
        let kinds = [
            "full",
            "concepts",
            "definitions",
            "formulas",
            "theorems",
            "pitfalls",
            "review",
            "self_test",
            "section",
        ];
        let mut seen = std::collections::HashSet::new();
        for kind in kinds {
            let zh = summary_instruction(kind, "zh");
            let en = summary_instruction(kind, "en");
            assert!(!zh.is_empty() && !en.is_empty(), "{kind} produced an empty prompt");
            assert_ne!(zh, en, "{kind} ignored the language");
            assert!(seen.insert(zh), "{kind} duplicated another kind's prompt");
        }
        // Unknown kinds must still produce a usable full summary prompt.
        let unknown = summary_instruction("not-a-kind", "zh");
        assert_eq!(unknown, summary_instruction("full", "zh"));
    }
}