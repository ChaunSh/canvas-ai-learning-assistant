//! Minimal OpenAI-compatible HTTP client.
//!
//! Deliberately hand-written rather than using a vendor SDK: the project must
//! work with any "OpenAI compatible" endpoint (official APIs, Azure-style
//! gateways, local llama.cpp / Ollama / LM Studio servers), and every one of
//! those differs slightly in what it implements.

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::domain::ModelInfo;
use crate::error::{AppError, AppResult};

pub struct OpenAiClient<'a> {
    pub http: &'a reqwest::Client,
    pub base_url: String,
    pub api_key: String,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct ChatMessageInput {
    pub role: String,
    /// Either plain text or a full content array (vision).
    pub content: Value,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessageInput>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i64>,
    pub stream: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ChatResult {
    pub content: String,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub finish_reason: Option<String>,
}

impl<'a> OpenAiClient<'a> {
    pub fn new(http: &'a reqwest::Client, base_url: &str, api_key: &str, timeout_seconds: i64) -> Self {
        Self {
            http,
            base_url: normalize_base(base_url),
            api_key: api_key.to_string(),
            timeout: Duration::from_secs(timeout_seconds.clamp(5, 900) as u64),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let builder = self
            .http
            .request(method, url)
            .timeout(self.timeout)
            .header("Content-Type", "application/json");
        if self.api_key.trim().is_empty() {
            builder
        } else {
            builder.bearer_auth(self.api_key.trim())
        }
    }

    /// `GET /models`. Many gateways omit this endpoint, so a failure here is not
    /// fatal for chat — the caller reports it separately.
    pub async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        let response = self
            .request(reqwest::Method::GET, "/models")
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(map_status(status, &body));
        }
        let parsed: Value = serde_json::from_str(&body)
            .map_err(|e| AppError::Ai(format!("模型列表不是有效的 JSON：{e}")))?;
        let mut models = Vec::new();
        if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
            for item in data {
                if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                    models.push(ModelInfo {
                        id: id.to_string(),
                        owned_by: item
                            .get("owned_by")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    });
                }
            }
        }
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
    }

    /// Non-streaming completion, used for summaries and planning.
    pub async fn chat(&self, request: &ChatRequest) -> AppResult<ChatResult> {
        let payload = build_payload(request, false);
        let response = self
            .request(reqwest::Method::POST, "/chat/completions")
            .json(&payload)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(map_status(status, &body));
        }
        let parsed: Value = serde_json::from_str(&body)
            .map_err(|e| AppError::Ai(format!("模型响应不是有效的 JSON：{e}")))?;
        parse_completion(&parsed)
    }

    /// Streaming completion. `on_delta` receives incremental text; returning
    /// `false` stops consumption and drops the connection.
    pub async fn chat_stream<F>(
        &self,
        request: &ChatRequest,
        mut on_delta: F,
    ) -> AppResult<ChatResult>
    where
        F: FnMut(&str) -> bool,
    {
        let payload = build_payload(request, true);
        let response = self
            .request(reqwest::Method::POST, "/chat/completions")
            .header("Accept", "text/event-stream")
            .json(&payload)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(map_status(status, &body));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        // Some gateways ignore `stream: true` and return a normal JSON body.
        if !content_type.contains("event-stream") {
            let body = response.text().await.unwrap_or_default();
            let parsed: Value = serde_json::from_str(&body)
                .map_err(|e| AppError::Ai(format!("模型响应不是有效的 JSON：{e}")))?;
            let result = parse_completion(&parsed)?;
            on_delta(&result.content);
            return Ok(result);
        }

        let mut result = ChatResult::default();
        let mut buffer = String::new();
        let mut stream = response.bytes_stream();
        let mut started = Instant::now();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(map_reqwest_error)?;
            if started.elapsed() > self.timeout {
                return Err(AppError::Ai("模型响应超时（流式读取阶段）".into()));
            }
            started = Instant::now();
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            loop {
                let Some(index) = find_event_boundary(&buffer) else {
                    break;
                };
                let (event, rest) = buffer.split_at(index.0);
                let event = event.to_string();
                buffer = rest[index.1 - index.0..].to_string();
                if let Some(outcome) = handle_sse_event(&event, &mut result, &mut on_delta)? {
                    return Ok(outcome);
                }
            }
        }

        // Flush a trailing event without a final blank line.
        if !buffer.trim().is_empty() {
            let _ = handle_sse_event(&buffer, &mut result, &mut on_delta)?;
        }

        if result.content.is_empty() {
            return Err(AppError::Ai("模型返回了空回答".into()));
        }
        Ok(result)
    }

    /// `POST /embeddings`.
    pub async fn embed(&self, model: &str, inputs: &[String]) -> AppResult<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let payload = json!({ "model": model, "input": inputs });
        let response = self
            .request(reqwest::Method::POST, "/embeddings")
            .json(&payload)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(map_status(status, &body));
        }
        let parsed: Value = serde_json::from_str(&body)
            .map_err(|e| AppError::Ai(format!("向量响应不是有效的 JSON：{e}")))?;
        let data = parsed
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| AppError::Ai("向量响应缺少 data 字段".into()))?;
        let mut out = Vec::with_capacity(data.len());
        for item in data {
            let embedding = item
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| AppError::Ai("向量响应缺少 embedding 字段".into()))?;
            let vector: Vec<f32> = embedding
                .iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect();
            if vector.is_empty() {
                return Err(AppError::Ai("模型返回了空向量".into()));
            }
            out.push(vector);
        }
        Ok(out)
    }
}

/// Locate the end of one SSE event and return `(event_end, next_start)`.
fn find_event_boundary(buffer: &str) -> Option<(usize, usize)> {
    if let Some(idx) = buffer.find("\r\n\r\n") {
        return Some((idx, idx + 4));
    }
    buffer.find("\n\n").map(|idx| (idx, idx + 2))
}

/// Parse one SSE event. Returns `Some(result)` when a terminal state is reached.
fn handle_sse_event<F>(
    event: &str,
    result: &mut ChatResult,
    on_delta: &mut F,
) -> AppResult<Option<ChatResult>>
where
    F: FnMut(&str) -> bool,
{
    for line in event.lines() {
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            return Ok(Some(result.clone()));
        }
        let Ok(parsed) = serde_json::from_str::<Value>(data) else {
            continue;
        };

        if let Some(error) = parsed.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("模型服务返回错误");
            return Err(AppError::Ai(message.to_string()));
        }

        if let Some(usage) = parsed.get("usage").filter(|u| !u.is_null()) {
            result.prompt_tokens = usage
                .get("prompt_tokens")
                .and_then(|v| v.as_i64())
                .or(result.prompt_tokens);
            result.completion_tokens = usage
                .get("completion_tokens")
                .and_then(|v| v.as_i64())
                .or(result.completion_tokens);
        }

        let Some(choice) = parsed
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
        else {
            continue;
        };

        if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            result.finish_reason = Some(reason.to_string());
        }

        let delta = choice
            .get("delta")
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
            .or_else(|| {
                // Some gateways only fill `message` even when streaming.
                choice
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
            });

        if let Some(text) = delta {
            if !text.is_empty() {
                result.content.push_str(text);
                if !on_delta(text) {
                    return Ok(Some(result.clone()));
                }
            }
        }
    }
    Ok(None)
}

fn build_payload(request: &ChatRequest, stream: bool) -> Value {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();
    let mut payload = json!({
        "model": request.model,
        "messages": messages,
        "stream": stream,
    });
    if let Some(temperature) = request.temperature {
        payload["temperature"] = json!(temperature.clamp(0.0, 2.0));
    }
    if let Some(max_tokens) = request.max_tokens {
        payload["max_tokens"] = json!(max_tokens.max(1));
    }
    if stream {
        // Ask for token accounting on the final chunk; harmless when unsupported.
        payload["stream_options"] = json!({ "include_usage": true });
    }
    payload
}

fn parse_completion(parsed: &Value) -> AppResult<ChatResult> {
    if let Some(error) = parsed.get("error") {
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("模型服务返回错误");
        return Err(AppError::Ai(message.to_string()));
    }
    let choice = parsed
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .ok_or_else(|| AppError::Ai("模型响应缺少 choices".into()))?;
    let content = choice
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    if content.trim().is_empty() {
        return Err(AppError::Ai("模型返回了空回答".into()));
    }
    Ok(ChatResult {
        content,
        prompt_tokens: parsed
            .get("usage")
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|v| v.as_i64()),
        completion_tokens: parsed
            .get("usage")
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_i64()),
        finish_reason: choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

/// Accept `https://host`, `https://host/v1`, `https://host/v1/` alike.
fn normalize_base(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1") || trimmed.contains("/v1/") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

fn map_reqwest_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        return AppError::Ai("请求 AI 服务超时，请检查网络或稍后重试".into());
    }
    if error.is_connect() {
        return AppError::Ai("无法连接 AI 服务，请检查 Base URL 与网络".into());
    }
    AppError::Ai(format!("AI 请求失败：{error}"))
}

fn map_status(status: reqwest::StatusCode, body: &str) -> AppError {
    let detail = extract_error_message(body);
    match status.as_u16() {
        401 | 403 => AppError::Ai(format!("认证失败（{status}）：API Key 无效或无权访问。{detail}")),
        404 => AppError::Ai(format!(
            "接口或模型不存在（404）：请检查 Base URL 与模型名称。{detail}"
        )),
        429 => AppError::Ai(format!("请求过于频繁或额度不足（429）。{detail}")),
        500..=599 => AppError::Ai(format!("AI 服务内部错误（{status}）。{detail}")),
        _ => AppError::Ai(format!("AI 服务返回 {status}。{detail}")),
    }
}

fn extract_error_message(body: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<Value>(body) {
        if let Some(message) = parsed
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return message.to_string();
        }
        if let Some(message) = parsed.get("message").and_then(|m| m.as_str()) {
            return message.to_string();
        }
    }
    body.chars().take(300).collect()
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProbe {
    pub chat_ok: bool,
    pub models: Vec<ModelInfo>,
    pub latency_ms: i64,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_normalisation() {
        assert_eq!(normalize_base("https://api.openai.com"), "https://api.openai.com/v1");
        assert_eq!(normalize_base("https://api.deepseek.com/v1"), "https://api.deepseek.com/v1");
        assert_eq!(normalize_base("http://localhost:11434/v1/"), "http://localhost:11434/v1");
        assert_eq!(
            normalize_base("https://gateway.example.com/openai/v1"),
            "https://gateway.example.com/openai/v1"
        );
    }

    #[test]
    fn payload_omits_unset_optionals() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessageInput {
                role: "user".into(),
                content: json!("hi"),
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
        };
        let payload = build_payload(&req, false);
        assert!(payload.get("temperature").is_none());
        assert!(payload.get("max_tokens").is_none());
        assert_eq!(payload["stream"], json!(false));
        assert!(payload.get("stream_options").is_none());
    }

    #[test]
    fn streaming_payload_requests_usage() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![],
            temperature: Some(0.7),
            max_tokens: Some(512),
            stream: true,
        };
        let payload = build_payload(&req, true);
        assert_eq!(payload["stream_options"]["include_usage"], json!(true));
        assert_eq!(payload["temperature"], json!(0.7));
    }

    #[test]
    fn sse_delta_is_accumulated() {
        let mut result = ChatResult::default();
        let mut collected = String::new();
        let event = "data: {\"choices\":[{\"delta\":{\"content\":\"导\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"数\"}}]}\n";
        let mut push = |text: &str| {
            collected.push_str(text);
            true
        };
        let done = handle_sse_event(event, &mut result, &mut push).unwrap();
        assert!(done.is_none());
        assert_eq!(collected, "导数");
        assert_eq!(result.content, "导数");
    }

    #[test]
    fn sse_done_terminates() {
        let mut result = ChatResult {
            content: "x".into(),
            ..Default::default()
        };
        let mut push = |_: &str| true;
        let done = handle_sse_event("data: [DONE]", &mut result, &mut push).unwrap();
        assert!(done.is_some());
    }

    #[test]
    fn sse_error_is_surfaced() {
        let mut result = ChatResult::default();
        let mut push = |_: &str| true;
        let err = handle_sse_event(
            "data: {\"error\":{\"message\":\"rate limited\"}}",
            &mut result,
            &mut push,
        )
        .unwrap_err();
        assert!(err.to_string().contains("rate limited"));
    }

    #[test]
    fn sse_usage_is_captured() {
        let mut result = ChatResult::default();
        let mut push = |_: &str| true;
        handle_sse_event(
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":22}}",
            &mut result,
            &mut push,
        )
        .unwrap();
        assert_eq!(result.prompt_tokens, Some(11));
        assert_eq!(result.completion_tokens, Some(22));
    }

    #[test]
    fn non_streaming_completion_parses() {
        let body = json!({
            "choices": [{"message": {"content": "答案"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 7}
        });
        let result = parse_completion(&body).unwrap();
        assert_eq!(result.content, "答案");
        assert_eq!(result.prompt_tokens, Some(5));
        assert_eq!(result.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn error_messages_are_extracted() {
        assert_eq!(
            extract_error_message("{\"error\":{\"message\":\"bad key\"}}"),
            "bad key"
        );
        let long = "x".repeat(1000);
        assert_eq!(extract_error_message(&long).chars().count(), 300);
    }
}
