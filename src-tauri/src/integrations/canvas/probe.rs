//! Script generation and bridge-URL parsing for Canvas probes.
//!
//! The page-side script performs a same-origin `fetch` (so the browser attaches
//! its own session cookies) and reports the response by navigating to a reserved
//! path. Rust intercepts that navigation and never lets it reach the network.

use percent_encoding::percent_decode_str;

#[derive(Debug, Clone, Default)]
pub struct BridgePayload {
    pub nonce: String,
    pub status: Option<i64>,
    pub body: Option<String>,
    pub error: Option<String>,
}

/// JavaScript executed inside the Canvas page.
///
/// `path` is JSON-encoded by this function; it never contains user input that has
/// not been validated by the caller.
pub fn build_fetch_script(nonce: &str, path: &str, bridge: &str) -> String {
    let nonce_literal = serde_json::to_string(nonce).unwrap_or_else(|_| "\"\"".into());
    let path_literal = serde_json::to_string(path).unwrap_or_else(|_| "\"/\"".into());
    let bridge_literal = serde_json::to_string(bridge).unwrap_or_else(|_| "\"/__caa_bridge__/\"".into());

    format!(
        r#"(function(){{
  try {{
    var nonce = {nonce_literal};
    var path = {path_literal};
    var bridge = {bridge_literal};
    var reply = function(query) {{
      try {{ window.location.href = window.location.origin + bridge + nonce + "?" + query; }}
      catch (e) {{ /* navigation blocked by the host: nothing more to do */ }}
    }};
    // A freshly opened window reports its URL before the document exists; a
    // fetch started then fails with a network error that looks like an expired
    // session. Wait for the current document, then retry once on a network
    // failure (the first request can still race the very first paint).
    var waitForDocument = function() {{
      if (document.readyState === "complete") return Promise.resolve();
      return new Promise(function(resolve) {{
        window.addEventListener("load", function() {{ resolve(); }}, {{ once: true }});
        setTimeout(resolve, 5000);
      }});
    }};
    var get = function(attempt) {{
      return fetch(path, {{
        method: "GET",
        credentials: "same-origin",
        redirect: "follow",
        headers: {{ "Accept": "application/json, text/html;q=0.8", "X-Requested-With": "XMLHttpRequest" }}
      }}).then(function(response) {{
        return response.text().then(function(text) {{
          reply("status=" + response.status + "&body=" + encodeURIComponent(text));
        }});
      }}).catch(function(error) {{
        if (attempt < 2) {{
          return new Promise(function(resolve) {{ setTimeout(resolve, 700); }}).then(function() {{
            return get(attempt + 1);
          }});
        }}
        reply("error=" + encodeURIComponent(String((error && error.message) || error)));
      }});
    }};
    waitForDocument().then(function() {{ return get(0); }});
  }} catch (outer) {{
    try {{ window.location.href = window.location.origin + "/__caa_bridge__/error?error=" + encodeURIComponent(String(outer)); }} catch (e) {{}}
  }}
}})();"#
    )
}

/// Extract a bridge payload from an intercepted navigation URL.
///
/// Returns `None` for any URL that is not a bridge response.
pub fn parse_bridge_url(url: &str) -> Option<BridgePayload> {
    let (before_query, query) = match url.split_once('?') {
        Some((head, tail)) => (head, tail),
        None => (url, ""),
    };
    let marker = "/__caa_bridge__/";
    let index = before_query.find(marker)?;
    let nonce_part = &before_query[index + marker.len()..];
    let nonce = nonce_part.trim_matches('/').to_string();
    if nonce.is_empty() {
        return None;
    }

    let mut payload = BridgePayload {
        nonce,
        ..Default::default()
    };

    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        let decoded = percent_decode_str(value)
            .decode_utf8_lossy()
            .to_string();
        match key {
            "status" => payload.status = decoded.parse::<i64>().ok(),
            "body" => payload.body = Some(decoded),
            "error" => payload.error = Some(decoded),
            _ => {}
        }
    }

    // A bridge URL must carry exactly one of body/error to be meaningful.
    if payload.body.is_none() && payload.error.is_none() && payload.status.is_none() {
        return None;
    }
    Some(payload)
}

/// Reject a payload that is too large to travel through a URL.
///
/// Callers page their requests instead (Canvas `per_page`), so hitting this limit
/// means the endpoint ignored pagination.
pub const MAX_BRIDGE_BYTES: usize = 900_000;

pub fn ensure_payload_size(body: &str) -> Result<(), String> {
    if body.len() > MAX_BRIDGE_BYTES {
        return Err(format!(
            "Canvas 响应过大（{} KB），已拒绝通过桥接通道传回。请缩小分页范围。",
            body.len() / 1024
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_embeds_literals_safely() {
        let script = build_fetch_script("abc", "/api/v1/courses", "/__caa_bridge__/");
        assert!(script.contains("\"abc\""));
        assert!(script.contains("\"/api/v1/courses\""));
        assert!(script.contains("credentials"));
        assert!(script.contains("same-origin"));
        // Must not disable security features.
        assert!(!script.contains("document.cookie"));
        assert!(!script.contains("localStorage"));
    }

    #[test]
    fn script_escapes_injected_quotes() {
        let script = build_fetch_script("a\"b", "/x", "/__caa_bridge__/");
        assert!(script.contains("\\\""));
    }

    #[test]
    fn parses_success_payload() {
        let url = "https://hkust-gz.instructure.com/__caa_bridge__/nonce-1?status=200&body=%5B%7B%22id%22%3A1%7D%5D";
        let payload = parse_bridge_url(url).unwrap();
        assert_eq!(payload.nonce, "nonce-1");
        assert_eq!(payload.status, Some(200));
        assert_eq!(payload.body.as_deref(), Some("[{\"id\":1}]"));
        assert!(payload.error.is_none());
    }

    #[test]
    fn parses_error_payload() {
        let url = "https://x/__caa_bridge__/n2?error=Network%20error";
        let payload = parse_bridge_url(url).unwrap();
        assert_eq!(payload.error.as_deref(), Some("Network error"));
        assert!(payload.status.is_none());
    }

    #[test]
    fn ignores_unrelated_urls() {
        assert!(parse_bridge_url("https://hkust-gz.instructure.com/courses/1").is_none());
        assert!(parse_bridge_url("https://hkust-gz.instructure.com/__caa_bridge__/").is_none());
        assert!(parse_bridge_url("https://hkust-gz.instructure.com/__caa_bridge__/n").is_none());
    }

    #[test]
    fn decodes_multibyte_payloads() {
        let encoded = "%E5%AF%BC%E6%95%B0"; // 导数
        let url = format!("https://x/__caa_bridge__/n?status=200&body={encoded}");
        let payload = parse_bridge_url(&url).unwrap();
        assert_eq!(payload.body.as_deref(), Some("导数"));
    }

    #[test]
    fn enforces_payload_size() {
        assert!(ensure_payload_size("small").is_ok());
        let big = "x".repeat(MAX_BRIDGE_BYTES + 1);
        assert!(ensure_payload_size(&big).is_err());
    }
}
