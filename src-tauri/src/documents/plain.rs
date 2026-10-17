//! Plain text, Markdown, CSV, JSON and HTML extraction.

use crate::documents::chunker::ExtractedPage;
use crate::error::{AppError, AppResult};

/// Read a text file, honouring UTF-8/UTF-16 byte-order marks and falling back to
/// lossy UTF-8 (course materials frequently contain stray GBK bytes).
pub fn read_text(path: &std::path::Path) -> AppResult<String> {
    let bytes = std::fs::read(path)?;
    if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        return Ok(String::from_utf8_lossy(&bytes[3..]).into_owned());
    }
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return Ok(String::from_utf16_lossy(&units));
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return Ok(String::from_utf16_lossy(&units));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Text-like files become a single unpaginated block. Page numbers are never
/// invented for formats that do not have them.
pub fn extract(path: &std::path::Path, kind: &str) -> AppResult<Vec<ExtractedPage>> {
    let raw = read_text(path)?;
    let text = match kind {
        "csv" => normalize_csv(&raw),
        "json" => normalize_json(&raw),
        "html" | "htm" => html_to_text(&raw),
        _ => raw,
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid("文件中没有可提取的文本"));
    }
    Ok(vec![ExtractedPage {
        index: 0,
        text: trimmed.to_string(),
        width: None,
        height: None,
        has_image: false,
    }])
}

fn normalize_csv(raw: &str) -> String {
    // Keep rows intact and make cells searchable as a single line.
    let mut out = String::new();
    for line in raw.lines().take(20_000) {
        let cells: Vec<&str> = line.split(',').map(|c| c.trim().trim_matches('"')).collect();
        if cells.iter().all(|c| c.is_empty()) {
            continue;
        }
        out.push_str(&cells.join(" | "));
        out.push('\n');
    }
    out
}

fn normalize_json(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => {
            let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| raw.to_string());
            // Very large JSON trees are truncated for indexing but the original
            // file is always available to the reader.
            pretty.chars().take(400_000).collect()
        }
        Err(_) => raw.to_string(),
    }
}

/// Convert HTML to readable plain text without pulling a full DOM dependency.
///
/// Used for Canvas pages and announcement bodies, which arrive as HTML.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut chars = html.chars().peekable();
    let mut in_tag = false;
    let mut tag = String::new();

    while let Some(c) = chars.next() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name: String = tag
                    .trim()
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                let closing = tag.trim_start().starts_with('/');
                let block = matches!(
                    name.as_str(),
                    "p" | "div" | "br" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                        | "section" | "article" | "blockquote" | "pre" | "table" | "ul" | "ol"
                );
                if block {
                    out.push('\n');
                }
                if name == "li" && !closing {
                    // Marker helps the chunker classify list blocks.
                    out.push_str("- ");
                }
                if name == "td" || name == "th" {
                    out.push_str(" | ");
                }
                if name == "img" {
                    out.push_str("[图片] ");
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }

    let decoded = decode_entities(&out);
    let mut cleaned = String::with_capacity(decoded.len());
    let mut blank_run = 0;
    for line in decoded.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            cleaned.push('\n');
        } else {
            blank_run = 0;
            cleaned.push_str(&squeeze_spaces(trimmed));
            cleaned.push('\n');
        }
    }
    cleaned.trim().to_string()
}

fn squeeze_spaces(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut prev_space = false;
    for c in line.chars() {
        if c == ' ' || c == '\t' || c == '\u{00a0}' {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Decode the HTML entities that actually appear in LMS content.
pub fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != '&' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let mut end = None;
        for j in (i + 1)..bytes.len().min(i + 12) {
            if bytes[j] == ';' {
                end = Some(j);
                break;
            }
        }
        let Some(end) = end else {
            out.push('&');
            i += 1;
            continue;
        };
        let entity: String = bytes[i + 1..end].iter().collect();
        let replacement = match entity.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "times" => Some('×'),
            "divide" => Some('÷'),
            "le" => Some('≤'),
            "ge" => Some('≥'),
            "ne" => Some('≠'),
            "rarr" => Some('→'),
            "larr" => Some('←'),
            "infin" => Some('∞'),
            "sum" => Some('∑'),
            "int" => Some('∫'),
            _ => {
                if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    None
                }
            }
        };
        match replacement {
            Some(c) => {
                out.push(c);
                i = end + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_decode() {
        assert_eq!(decode_entities("a &amp; b &lt; c &#65; &#x4E2D;"), "a & b < c A 中");
        assert_eq!(decode_entities("100 &unknown; 200"), "100 &unknown; 200");
        assert_eq!(decode_entities("bare & ampersand"), "bare & ampersand");
    }

    #[test]
    fn html_becomes_readable_text() {
        let html = "<h1>第二章</h1><p>导数&nbsp;定义</p><ul><li>第一点</li><li>第二点</li></ul>";
        let text = html_to_text(html);
        assert!(text.contains("第二章"));
        assert!(text.contains("导数 定义"));
        assert!(text.contains("- 第一点"));
    }

    #[test]
    fn csv_rows_join_cells() {
        let text = normalize_csv("a,b,c\n1,2,3\n");
        assert_eq!(text, "a | b | c\n1 | 2 | 3\n");
    }

    #[test]
    fn read_text_handles_bom() {
        let dir = std::env::temp_dir().join("caa-plain-test");
        std::fs::create_dir_all(&dir).unwrap();
        let utf8_bom = dir.join("bom.md");
        std::fs::write(&utf8_bom, [0xEF, 0xBB, 0xBF, b'h', b'i']).unwrap();
        assert_eq!(read_text(&utf8_bom).unwrap(), "hi");

        let utf16 = dir.join("bom16.txt");
        let mut bytes = vec![0xFF, 0xFE];
        for u in "hi".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(&utf16, bytes).unwrap();
        assert_eq!(read_text(&utf16).unwrap(), "hi");
    }
}
