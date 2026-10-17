//! AI transport tests over a real TCP socket.
//!
//! These do not mock the client: a minimal HTTP/1.1 server is spawned on an
//! ephemeral loopback port and the real `OpenAiClient` (reqwest + rustls +
//! serde) talks to it. That covers the parts unit tests cannot: request
//! shaping, SSE framing across chunk boundaries, content-type fallback, status
//! mapping and mid-stream cancellation.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use canvas_ai_assistant_lib::ai::openai::{ChatMessageInput, ChatRequest, OpenAiClient};
use serde_json::json;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Normal behaviour: stream for chat when asked, JSON otherwise.
    Normal,
    /// Always answer 401 with an OpenAI-style error envelope.
    Unauthorized,
    /// Ignore `stream: true` and return a plain JSON completion.
    IgnoreStream,
}

struct MockServer {
    base_url: String,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    fn start(mode: Mode, connections: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let port = listener.local_addr().expect("local addr").port();
        let handle = thread::spawn(move || {
            for _ in 0..connections {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let _ = handle_connection(stream, mode);
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            handle: Some(handle),
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_connection(mut stream: TcpStream, mode: Mode) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    // Read headers.
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = find(&buffer, b"\r\n\r\n") {
            break pos + 4;
        }
        if buffer.len() > 64 * 1024 {
            return Ok(());
        }
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let request_line = head.lines().next().unwrap_or_default().to_string();
    let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_string();

    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    // Read the body (the client always sends it before waiting for a reply).
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8_lossy(&body).to_string();

    if mode == Mode::Unauthorized {
        return respond(
            stream,
            "401 Unauthorized",
            "application/json",
            r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#,
        );
    }

    if path.ends_with("/models") {
        return respond(
            stream,
            "200 OK",
            "application/json",
            r#"{"object":"list","data":[{"id":"mock-chat","owned_by":"mock"},{"id":"mock-small","owned_by":"mock"}]}"#,
        );
    }

    if path.ends_with("/embeddings") {
        return respond(
            stream,
            "200 OK",
            "application/json",
            r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1,0.2,0.3]},{"object":"embedding","index":1,"embedding":[0.4,0.5,0.6]}]}"#,
        );
    }

    if path.ends_with("/chat/completions") {
        let wants_stream = body.contains("\"stream\":true");
        if wants_stream && mode == Mode::IgnoreStream {
            return respond(
                stream,
                "200 OK",
                "application/json",
                r#"{"choices":[{"message":{"role":"assistant","content":"一次性回答"},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":3}}"#,
            );
        }
        if wants_stream {
            // Deliberately split an event across writes to exercise buffering.
            let first = "data: {\"choices\":[{\"delta\":{\"content\":\"导数\"},\"index\":0}]}\n\n";
            let second = "data: {\"choices\":[{\"delta\":{\"content\":\"描述\"},\"index\":0}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"变化率\"},\"index\":0,\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":42,\"completion_tokens\":9}}\n\n";
            let third = "data: [DONE]\n\n";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
            )?;
            stream.write_all(first.as_bytes())?;
            stream.flush()?;
            thread::sleep(Duration::from_millis(20));
            stream.write_all(second.as_bytes())?;
            stream.flush()?;
            thread::sleep(Duration::from_millis(20));
            stream.write_all(third.as_bytes())?;
            stream.flush()?;
            return Ok(());
        }
        return respond(
            stream,
            "200 OK",
            "application/json",
            r#"{"choices":[{"message":{"role":"assistant","content":"这是总结"},"finish_reason":"stop"}],"usage":{"prompt_tokens":11,"completion_tokens":5}}"#,
        );
    }

    respond(stream, "404 Not Found", "application/json", r#"{"error":{"message":"no route"}}"#)
}

fn respond(
    mut stream: TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn client() -> reqwest::Client {
    canvas_ai_assistant_lib::security::tls::install_crypto_provider();
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("build http client")
}

fn chat_request(stream: bool) -> ChatRequest {
    ChatRequest {
        model: "mock-chat".into(),
        messages: vec![ChatMessageInput {
            role: "user".into(),
            content: json!("解释一下导数"),
        }],
        temperature: Some(0.2),
        max_tokens: Some(256),
        stream,
    }
}

#[tokio::test]
async fn streaming_completion_collects_deltas_usage_and_finish_reason() {
    let server = MockServer::start(Mode::Normal, 1);
    let http = client();
    let api = OpenAiClient::new(&http, &server.base_url, "test-key", 30);

    let mut deltas: Vec<String> = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        api.chat_stream(&chat_request(true), |delta| {
            deltas.push(delta.to_string());
            true
        }),
    )
    .await
    .expect("stream should finish before the timeout")
    .expect("streaming chat should succeed");

    assert_eq!(deltas, vec!["导数", "描述", "变化率"]);
    assert_eq!(result.content, "导数描述变化率");
    assert_eq!(result.finish_reason.as_deref(), Some("stop"));
    assert_eq!(result.prompt_tokens, Some(42));
    assert_eq!(result.completion_tokens, Some(9));
}

#[tokio::test]
async fn streaming_stops_early_when_the_callback_returns_false() {
    // This is the path behind the UI's "停止生成" button: returning false must
    // end consumption without erroring.
    let server = MockServer::start(Mode::Normal, 1);
    let http = client();
    let api = OpenAiClient::new(&http, &server.base_url, "test-key", 30);

    let mut seen = 0usize;
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        api.chat_stream(&chat_request(true), |_| {
            seen += 1;
            false
        }),
    )
    .await
    .expect("cancelled stream should return promptly")
    .expect("cancelling is not an error");

    assert_eq!(seen, 1, "consumption must stop after the first delta");
    assert_eq!(result.content, "导数");
}

#[tokio::test]
async fn non_streaming_completion_returns_content_and_usage() {
    let server = MockServer::start(Mode::Normal, 1);
    let http = client();
    let api = OpenAiClient::new(&http, &server.base_url, "test-key", 30);

    let result = api.chat(&chat_request(false)).await.expect("chat should succeed");
    assert_eq!(result.content, "这是总结");
    assert_eq!(result.prompt_tokens, Some(11));
    assert_eq!(result.completion_tokens, Some(5));
    assert_eq!(result.finish_reason.as_deref(), Some("stop"));
}

#[tokio::test]
async fn gateway_ignoring_stream_falls_back_to_json_body() {
    let server = MockServer::start(Mode::IgnoreStream, 1);
    let http = client();
    let api = OpenAiClient::new(&http, &server.base_url, "test-key", 30);

    let mut deltas: Vec<String> = Vec::new();
    let result = api
        .chat_stream(&chat_request(true), |delta| {
            deltas.push(delta.to_string());
            true
        })
        .await
        .expect("fallback should succeed");

    assert_eq!(result.content, "一次性回答");
    assert_eq!(deltas, vec!["一次性回答"], "the whole answer is emitted once");
}

#[tokio::test]
async fn models_endpoint_is_parsed_and_sorted() {
    let server = MockServer::start(Mode::Normal, 1);
    let http = client();
    let api = OpenAiClient::new(&http, &server.base_url, "test-key", 30);

    let models = api.list_models().await.expect("models should parse");
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["mock-chat", "mock-small"]);
    assert!(models.iter().all(|m| m.owned_by.as_deref() == Some("mock")));
}

#[tokio::test]
async fn embeddings_endpoint_returns_one_vector_per_input() {
    let server = MockServer::start(Mode::Normal, 1);
    let http = client();
    let api = OpenAiClient::new(&http, &server.base_url, "test-key", 30);

    let vectors = api
        .embed("mock-embed", &["第一段".to_string(), "第二段".to_string()])
        .await
        .expect("embeddings should parse");
    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0], vec![0.1f32, 0.2, 0.3]);
    assert_eq!(vectors[1], vec![0.4f32, 0.5, 0.6]);
}

#[tokio::test]
async fn unauthorized_response_becomes_an_actionable_message() {
    let server = MockServer::start(Mode::Unauthorized, 1);
    let http = client();
    let api = OpenAiClient::new(&http, &server.base_url, "bad-key", 30);

    let error = api.chat(&chat_request(false)).await.expect_err("401 must fail");
    let message = error.to_string();
    assert!(
        message.contains("认证失败"),
        "expected an auth hint, got: {message}"
    );
    assert!(
        message.contains("API Key"),
        "expected the message to mention the API key, got: {message}"
    );
    assert!(
        message.contains("Incorrect API key"),
        "expected the upstream detail to be preserved, got: {message}"
    );
}
