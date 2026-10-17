//! AI providers, chat sessions/messages/citations and usage accounting.

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::domain::{
    AiProvider, ChatMessage, ChatScope, ChatSession, ChatStreamEvent, Citation, UsageStats,
};
use crate::error::{AppError, AppResult};
use crate::storage::db::{b, i};
use crate::time::now_iso;

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

const PROVIDER_SELECT: &str = r#"
SELECT id, name, base_url, chat_model, vision_model, embedding_model, max_output_tokens,
       temperature, timeout_seconds, is_default, last_tested_at, last_test_ok, last_test_error,
       created_at, updated_at
FROM ai_providers
"#;

fn map_provider(row: &Row<'_>) -> rusqlite::Result<AiProvider> {
    Ok(AiProvider {
        id: row.get(0)?,
        name: row.get(1)?,
        base_url: row.get(2)?,
        chat_model: row.get(3)?,
        vision_model: row.get(4)?,
        embedding_model: row.get(5)?,
        max_output_tokens: row.get(6)?,
        temperature: row.get(7)?,
        timeout_seconds: row.get(8)?,
        is_default: b(row.get::<_, i64>(9)?),
        // Filled in by the command layer from the platform secret store.
        has_api_key: false,
        last_tested_at: row.get(10)?,
        last_test_ok: row.get::<_, Option<i64>>(11)?.map(b),
        last_test_error: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

pub fn list_providers(conn: &Connection) -> AppResult<Vec<AiProvider>> {
    let sql = format!("{PROVIDER_SELECT} ORDER BY is_default DESC, name ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], map_provider)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn get_provider(conn: &Connection, id: &str) -> AppResult<AiProvider> {
    let sql = format!("{PROVIDER_SELECT} WHERE id = ?1");
    conn.query_row(&sql, [id], map_provider)
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("AI 服务配置不存在：{id}")))
}

/// The provider to use when the caller did not choose one.
pub fn default_provider(conn: &Connection) -> AppResult<Option<AiProvider>> {
    let sql = format!("{PROVIDER_SELECT} WHERE is_default = 1 LIMIT 1");
    let found = conn
        .query_row(&sql, [], map_provider)
        .optional()?;
    if found.is_some() {
        return Ok(found);
    }
    // Fall back to the first configured provider.
    let sql = format!("{PROVIDER_SELECT} ORDER BY created_at ASC LIMIT 1");
    Ok(conn.query_row(&sql, [], map_provider).optional()?)
}

pub struct ProviderUpsert<'a> {
    pub id: Option<&'a str>,
    pub name: &'a str,
    pub base_url: &'a str,
    pub chat_model: &'a str,
    pub vision_model: Option<&'a str>,
    pub embedding_model: Option<&'a str>,
    pub max_output_tokens: Option<i64>,
    pub temperature: Option<f64>,
    pub timeout_seconds: i64,
    pub is_default: bool,
}

pub fn save_provider(conn: &Connection, seed: &ProviderUpsert<'_>) -> AppResult<AiProvider> {
    if seed.name.trim().is_empty() {
        return Err(AppError::invalid("服务名称不能为空"));
    }
    if !(seed.base_url.starts_with("http://") || seed.base_url.starts_with("https://")) {
        return Err(AppError::invalid("Base URL 必须以 http:// 或 https:// 开头"));
    }
    if seed.chat_model.trim().is_empty() {
        return Err(AppError::invalid("聊天模型不能为空"));
    }
    if let Some(t) = seed.temperature {
        if !(0.0..=2.0).contains(&t) {
            return Err(AppError::invalid("温度必须在 0 到 2 之间"));
        }
    }
    if seed.timeout_seconds <= 0 {
        return Err(AppError::invalid("超时时间必须大于 0"));
    }

    let base_url = seed.base_url.trim_end_matches('/').to_string();
    let now = now_iso();
    let id = match seed.id {
        Some(existing) => {
            get_provider(conn, existing)?;
            conn.execute(
                "UPDATE ai_providers SET name = ?2, base_url = ?3, chat_model = ?4, vision_model = ?5,
                        embedding_model = ?6, max_output_tokens = ?7, temperature = ?8,
                        timeout_seconds = ?9, updated_at = ?10
                 WHERE id = ?1",
                params![
                    existing,
                    seed.name.trim(),
                    base_url,
                    seed.chat_model.trim(),
                    seed.vision_model.filter(|v| !v.trim().is_empty()),
                    seed.embedding_model.filter(|v| !v.trim().is_empty()),
                    seed.max_output_tokens,
                    seed.temperature,
                    seed.timeout_seconds,
                    now
                ],
            )?;
            existing.to_string()
        }
        None => {
            let new_id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO ai_providers (id, name, base_url, chat_model, vision_model,
                                           embedding_model, max_output_tokens, temperature,
                                           timeout_seconds, is_default, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?10)",
                params![
                    new_id,
                    seed.name.trim(),
                    base_url,
                    seed.chat_model.trim(),
                    seed.vision_model.filter(|v| !v.trim().is_empty()),
                    seed.embedding_model.filter(|v| !v.trim().is_empty()),
                    seed.max_output_tokens,
                    seed.temperature,
                    seed.timeout_seconds,
                    now
                ],
            )?;
            new_id
        }
    };

    if seed.is_default {
        set_default_provider(conn, &id)?;
    }
    get_provider(conn, &id)
}

pub fn set_default_provider(conn: &Connection, id: &str) -> AppResult<()> {
    get_provider(conn, id)?;
    let now = now_iso();
    conn.execute("UPDATE ai_providers SET is_default = 0 WHERE id <> ?1", [id])?;
    conn.execute(
        "UPDATE ai_providers SET is_default = 1, updated_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

pub fn delete_provider(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM ai_providers WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AppError::not_found(format!("AI 服务配置不存在：{id}")));
    }
    Ok(())
}

pub fn mark_tested(
    conn: &Connection,
    id: &str,
    ok: bool,
    error: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE ai_providers SET last_tested_at = ?2, last_test_ok = ?3, last_test_error = ?4,
                updated_at = ?2 WHERE id = ?1",
        params![id, now_iso(), i(ok), error],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

fn map_session(row: &Row<'_>) -> rusqlite::Result<ChatSession> {
    let course_ids: String = row.get(6)?;
    let material_ids: String = row.get(7)?;
    let scope_kind: String = row.get(5)?;
    Ok(ChatSession {
        id: row.get(0)?,
        title: row.get(1)?,
        provider_id: row.get(2)?,
        model: row.get(3)?,
        archived: b(row.get::<_, i64>(4)?),
        scope: ChatScope {
            kind: scope_kind,
            course_ids: serde_json::from_str(&course_ids).unwrap_or_default(),
            material_ids: serde_json::from_str(&material_ids).unwrap_or_default(),
        },
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        message_count: row.get(10)?,
    })
}

const SESSION_SELECT: &str = r#"
SELECT s.id, s.title, s.provider_id, s.model, s.archived, s.scope_kind,
       s.scope_course_ids, s.scope_material_ids, s.created_at, s.updated_at,
       (SELECT COUNT(*) FROM chat_messages m WHERE m.session_id = s.id) AS message_count
FROM chat_sessions s
"#;

pub fn create_session(
    conn: &Connection,
    title: &str,
    scope: &ChatScope,
    provider_id: Option<&str>,
    model: Option<&str>,
) -> AppResult<ChatSession> {
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO chat_sessions (id, title, provider_id, model, scope_kind, scope_course_ids,
                                    scope_material_ids, archived, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?8)",
        params![
            id,
            title,
            provider_id,
            model,
            scope.kind,
            serde_json::to_string(&scope.course_ids)?,
            serde_json::to_string(&scope.material_ids)?,
            now
        ],
    )?;
    get_session(conn, &id)
}

pub fn get_session(conn: &Connection, id: &str) -> AppResult<ChatSession> {
    let sql = format!("{SESSION_SELECT} WHERE s.id = ?1");
    conn.query_row(&sql, [id], map_session)
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("对话不存在：{id}")))
}

pub fn list_sessions(conn: &Connection, limit: i64) -> AppResult<Vec<ChatSession>> {
    let sql = format!("{SESSION_SELECT} WHERE s.archived = 0 ORDER BY s.updated_at DESC LIMIT ?1");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit], map_session)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn update_session_scope(conn: &Connection, id: &str, scope: &ChatScope) -> AppResult<()> {
    conn.execute(
        "UPDATE chat_sessions SET scope_kind = ?2, scope_course_ids = ?3, scope_material_ids = ?4,
                updated_at = ?5 WHERE id = ?1",
        params![
            id,
            scope.kind,
            serde_json::to_string(&scope.course_ids)?,
            serde_json::to_string(&scope.material_ids)?,
            now_iso()
        ],
    )?;
    Ok(())
}

pub fn rename_session(conn: &Connection, id: &str, title: &str) -> AppResult<()> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid("对话标题不能为空"));
    }
    conn.execute(
        "UPDATE chat_sessions SET title = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, trimmed, now_iso()],
    )?;
    Ok(())
}

pub fn delete_session(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM chat_sessions WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AppError::not_found(format!("对话不存在：{id}")));
    }
    Ok(())
}

fn touch_session(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE chat_sessions SET updated_at = ?2 WHERE id = ?1",
        params![id, now_iso()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

pub fn add_message(
    conn: &Connection,
    session_id: &str,
    role: &str,
    content: &str,
    status: &str,
    retrieval_query: Option<&str>,
) -> AppResult<ChatMessage> {
    let id = Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO chat_messages (id, session_id, role, content, status, retrieval_query,
                                    created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![id, session_id, role, content, status, retrieval_query, now],
    )?;
    touch_session(conn, session_id)?;
    get_message(conn, &id)
}

pub fn get_message(conn: &Connection, id: &str) -> AppResult<ChatMessage> {
    let mut message = conn
        .query_row(
            "SELECT id, session_id, role, content, status, error, retrieval_query,
                    prompt_tokens, completion_tokens, created_at, updated_at
             FROM chat_messages WHERE id = ?1",
            [id],
            |row| {
                Ok(ChatMessage {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    status: row.get(4)?,
                    error: row.get(5)?,
                    retrieval_query: row.get(6)?,
                    prompt_tokens: row.get(7)?,
                    completion_tokens: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                    citations: Vec::new(),
                })
            },
        )
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("消息不存在：{id}")))?;
    message.citations = list_citations(conn, &id)?;
    Ok(message)
}

pub fn update_message(
    conn: &Connection,
    id: &str,
    content: &str,
    status: &str,
    error: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE chat_messages SET content = ?2, status = ?3, error = ?4, updated_at = ?5
         WHERE id = ?1",
        params![id, content, status, error, now_iso()],
    )?;
    Ok(())
}

pub fn set_message_usage(
    conn: &Connection,
    id: &str,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE chat_messages SET prompt_tokens = ?2, completion_tokens = ?3 WHERE id = ?1",
        params![id, prompt_tokens, completion_tokens],
    )?;
    Ok(())
}

pub fn list_messages(conn: &Connection, session_id: &str) -> AppResult<Vec<ChatMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, role, content, status, error, retrieval_query, prompt_tokens,
                completion_tokens, created_at, updated_at
         FROM chat_messages WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
    )?;
    let rows = stmt.query_map([session_id], |row| {
        Ok(ChatMessage {
            id: row.get(0)?,
            session_id: row.get(1)?,
            role: row.get(2)?,
            content: row.get(3)?,
            status: row.get(4)?,
            error: row.get(5)?,
            retrieval_query: row.get(6)?,
            prompt_tokens: row.get(7)?,
            completion_tokens: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
            citations: Vec::new(),
        })
    })?;
    let mut out: Vec<ChatMessage> = Vec::new();
    for row in rows {
        out.push(row?);
    }
    for message in out.iter_mut() {
        message.citations = list_citations(conn, &message.id)?;
    }
    Ok(out)
}

pub fn delete_message(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM chat_messages WHERE id = ?1", [id])?;
    Ok(())
}

/// Remove the assistant+user pair after `message_id` so a regeneration starts
/// from a clean state.
pub fn truncate_after(conn: &Connection, session_id: &str, created_at: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM chat_messages WHERE session_id = ?1 AND created_at > ?2",
        params![session_id, created_at],
    )?;
    Ok(())
}

/// Prior turns for multi-turn context, oldest first.
pub fn recent_turns(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> AppResult<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT role, content FROM (
            SELECT role, content, created_at, rowid AS rid FROM chat_messages
             WHERE session_id = ?1 AND status IN ('complete', 'stopped') AND role IN ('user','assistant')
             ORDER BY created_at DESC, rid DESC LIMIT ?2
         ) ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![session_id, limit as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Citations
// ---------------------------------------------------------------------------

pub fn add_citations(
    conn: &Connection,
    message_id: &str,
    citations: &[Citation],
) -> AppResult<()> {
    conn.execute("DELETE FROM chat_citations WHERE message_id = ?1", [message_id])?;
    let now = now_iso();
    let mut stmt = conn.prepare(
        "INSERT INTO chat_citations (id, message_id, document_id, chunk_id, course_id, filename,
                                     page, page_kind, slide, section, excerpt, source_type, score,
                                     ordinal, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )?;
    for (index, citation) in citations.iter().enumerate() {
        // The UI shows these as S1, S2, … matching the labels in the answer, so
        // the stored ordinal must be the 1-based label. Enumerating from 0 here
        // produced a card labelled "S0" that no [S1] marker could point at.
        let ordinal = if citation.ordinal > 0 {
            citation.ordinal
        } else {
            index as i64 + 1
        };
        stmt.execute(params![
            Uuid::new_v4().to_string(),
            message_id,
            citation.document_id,
            citation.chunk_id,
            citation.course_id,
            citation.filename,
            citation.page,
            citation.page_kind,
            citation.slide,
            citation.section,
            citation.excerpt,
            citation.source_type,
            citation.score,
            ordinal,
            now
        ])?;
    }
    Ok(())
}

pub fn list_citations(conn: &Connection, message_id: &str) -> AppResult<Vec<Citation>> {
    let mut stmt = conn.prepare(
        "SELECT id, message_id, document_id, chunk_id, course_id, filename, page, slide, section,
                excerpt, source_type, score, ordinal, page_kind
         FROM chat_citations WHERE message_id = ?1 ORDER BY ordinal ASC",
    )?;
    let rows = stmt.query_map([message_id], |row| {
        Ok(Citation {
            id: row.get(0)?,
            message_id: row.get(1)?,
            document_id: row.get(2)?,
            chunk_id: row.get(3)?,
            course_id: row.get(4)?,
            filename: row.get(5)?,
            page: row.get(6)?,
            slide: row.get(7)?,
            section: row.get(8)?,
            excerpt: row.get(9)?,
            source_type: row.get(10)?,
            score: row.get(11)?,
            ordinal: row.get(12)?,
            page_kind: row.get(13)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    // Messages written by 0.1.0 stored a 0-based ordinal, which the UI rendered
    // as "S0". Ordinals are always 1-based, so a row set containing 0 is shifted
    // up on read; new rows are already correct and are left untouched.
    if out.iter().any(|c: &Citation| c.ordinal == 0) {
        for citation in &mut out {
            citation.ordinal += 1;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

pub fn record_usage(
    conn: &Connection,
    provider_id: Option<&str>,
    model: Option<&str>,
    purpose: &str,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    latency_ms: Option<i64>,
    ok: bool,
    error: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO ai_usage_records (id, provider_id, model, purpose, prompt_tokens,
                                       completion_tokens, latency_ms, ok, error, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            Uuid::new_v4().to_string(),
            provider_id,
            model,
            purpose,
            prompt_tokens,
            completion_tokens,
            latency_ms,
            i(ok),
            error,
            now_iso()
        ],
    )?;
    Ok(())
}

pub fn usage_stats(conn: &Connection) -> AppResult<UsageStats> {
    Ok(conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0),
                COALESCE(SUM(CASE WHEN ok = 0 THEN 1 ELSE 0 END), 0)
         FROM ai_usage_records",
        [],
        |row| {
            Ok(UsageStats {
                requests: row.get(0)?,
                prompt_tokens: row.get(1)?,
                completion_tokens: row.get(2)?,
                failures: row.get(3)?,
            })
        },
    )?)
}

/// Cheap deterministic conversation title from the first user message.
pub fn derive_title(question: &str) -> String {
    let clean: String = question
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(48)
        .collect();
    if clean.trim().is_empty() {
        "新对话".to_string()
    } else {
        clean
    }
}

/// Helper so tests can build a stream event without importing Tauri.
pub fn started_event(request_id: &str, session_id: &str, message_id: &str) -> ChatStreamEvent {
    ChatStreamEvent::Started {
        request_id: request_id.into(),
        session_id: session_id.into(),
        message_id: message_id.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    #[test]
    fn provider_validation_and_default() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let err = save_provider(
                c,
                &ProviderUpsert {
                    id: None,
                    name: "Test".into(),
                    base_url: "ftp://nope".into(),
                    chat_model: "m".into(),
                    vision_model: None,
                    embedding_model: None,
                    max_output_tokens: None,
                    temperature: None,
                    timeout_seconds: 60,
                    is_default: false,
                },
            )
            .unwrap_err();
            assert_eq!(err.code(), "invalid_input");

            let p1 = save_provider(
                c,
                &ProviderUpsert {
                    id: None,
                    name: "DeepSeek".into(),
                    base_url: "https://api.deepseek.com".into(),
                    chat_model: "deepseek-chat".into(),
                    vision_model: None,
                    embedding_model: None,
                    max_output_tokens: Some(4096),
                    temperature: Some(0.3),
                    timeout_seconds: 120,
                    is_default: true,
                },
            )?;
            assert!(p1.is_default);

            let p2 = save_provider(
                c,
                &ProviderUpsert {
                    id: None,
                    name: "Local".into(),
                    base_url: "http://localhost:11434/v1".into(),
                    chat_model: "qwen".into(),
                    vision_model: None,
                    embedding_model: Some("nomic-embed".into()),
                    max_output_tokens: None,
                    temperature: None,
                    timeout_seconds: 60,
                    is_default: true,
                },
            )?;
            let all = list_providers(c)?;
            assert_eq!(all.len(), 2);
            assert_eq!(default_provider(c)?.unwrap().id, p2.id);
            assert!(!get_provider(c, &p1.id)?.is_default, "default must be exclusive");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn sessions_messages_and_citations() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let scope = ChatScope {
                kind: "course".into(),
                course_ids: vec!["k1".into()],
                material_ids: vec![],
            };
            let title = derive_title("什么是导数？");
            let session = create_session(c, &title, &scope, None, None)?;
            assert_eq!(session.message_count, 0);

            let user = add_message(c, &session.id, "user", "什么是导数？", "complete", Some("导数"))?;
            let assistant = add_message(c, &session.id, "assistant", "导数是……", "complete", None)?;
            add_citations(
                c,
                &assistant.id,
                &[Citation {
                    id: String::new(),
                    message_id: assistant.id.clone(),
                    document_id: "m1".into(),
                    chunk_id: "c1".into(),
                    course_id: Some("k1".into()),
                    filename: "lecture1.pdf".into(),
                    page: Some(3),
                    page_kind: Some("native".into()),
                    slide: None,
                    section: Some("1.1".into()),
                    excerpt: "导数是函数的变化率".into(),
                    source_type: "pdf".into(),
                    score: Some(0.9),
                    ordinal: 0,
                }],
            )?;

            let messages = list_messages(c, &session.id)?;
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[0].id, user.id);
            assert_eq!(messages[1].citations.len(), 1);
            assert_eq!(messages[1].citations[0].ordinal, 1, "labels start at S1");
            assert_eq!(messages[1].citations[0].page, Some(3));
            // Page provenance must survive persistence, otherwise the UI cannot
            // tell a real page from an app-generated one after a reload.
            assert_eq!(messages[1].citations[0].page_kind.as_deref(), Some("native"));
            assert_eq!(get_session(c, &session.id)?.message_count, 2);

            let turns = recent_turns(c, &session.id, 10)?;
            assert_eq!(turns.len(), 2);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn deleting_session_cascades() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let session = create_session(c, "tmp", &ChatScope::all(), None, None)?;
            add_message(c, &session.id, "user", "hi", "complete", None)?;
            delete_session(c, &session.id)?;
            let count: i64 = c.query_row("SELECT COUNT(*) FROM chat_messages", [], |r| r.get(0))?;
            assert_eq!(count, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn usage_stats_aggregate() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            record_usage(c, None, Some("m"), "chat", Some(10), Some(20), Some(100), true, None)?;
            record_usage(c, None, Some("m"), "chat", None, None, Some(50), false, Some("boom"))?;
            let stats = usage_stats(c)?;
            assert_eq!(stats.requests, 2);
            assert_eq!(stats.prompt_tokens, 10);
            assert_eq!(stats.completion_tokens, 20);
            assert_eq!(stats.failures, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn title_is_bounded() {
        assert_eq!(derive_title("  "), "新对话");
        let long = "a".repeat(200);
        assert!(derive_title(&long).chars().count() <= 48);
    }

    #[test]
    fn legacy_zero_based_ordinals_are_shifted_on_read() {
        // 0.1.0 stored ordinal 0 for the first citation; the UI showed "S0".
        // Reads must repair that so old conversations still line up with [S1].
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let scope = ChatScope { kind: "all".into(), course_ids: vec![], material_ids: vec![] };
            let session = create_session(c, "旧对话", &scope, None, None)?;
            let message = add_message(c, &session.id, "assistant", "见 [S1]", "complete", None)?;
            c.execute(
                "INSERT INTO chat_citations (id, message_id, document_id, chunk_id, filename,
                                             page, ordinal, excerpt, source_type, created_at)
                 VALUES ('c-old', ?1, 'm1', 'ch1', 'a.pdf', 8, 0, '摘录', 'paragraph', 't')",
                [&message.id],
            )?;
            let stored = list_citations(c, &message.id)?;
            assert_eq!(stored[0].ordinal, 1, "0-based rows are shifted to 1-based");
            Ok::<(), crate::error::AppError>(())
        })
        .unwrap();
    }
}
