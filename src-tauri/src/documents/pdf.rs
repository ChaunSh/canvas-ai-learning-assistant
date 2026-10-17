//! PDF text extraction with page-level fidelity.
//!
//! Why hand-rolled instead of a convenience crate: citations must point at the
//! *actual* page a statement came from (task specification §29), and most
//! pure-Rust PDF text helpers flatten the whole document into one string.
//!
//! Pipeline per page:
//!   1. resolve the page's content stream and font resources;
//!   2. decode the content stream into text-showing operators;
//!   3. map glyph codes to Unicode through the font's `ToUnicode` CMap when
//!      present, otherwise through a WinAnsi table;
//!   4. reconstruct line breaks / spaces from text-positioning operators.

use std::collections::HashMap;

use lopdf::{Document, Object, ObjectId};

use crate::documents::chunker::ExtractedPage;
use crate::error::{AppError, AppResult};

/// Result of PDF extraction.
pub struct PdfExtraction {
    pub pages: Vec<ExtractedPage>,
    /// Pages whose visible content is (mostly) raster: text layer is missing.
    pub pages_needing_ocr: Vec<i64>,
}

pub fn extract(path: &std::path::Path) -> AppResult<PdfExtraction> {
    let doc = Document::load(path)
        .map_err(|e| AppError::invalid(format!("无法解析 PDF：{e}")))?;
    extract_from_doc(&doc)
}

fn extract_from_doc(doc: &Document) -> AppResult<PdfExtraction> {
    let page_map = doc.get_pages();
    if page_map.is_empty() {
        return Err(AppError::invalid("PDF 中未找到任何页面"));
    }

    let mut pages = Vec::new();
    let mut needs_ocr = Vec::new();
    // lopdf page numbers are 1-based insertion order; expose 0-based indexes.
    let mut index = 0i64;
    for (_page_num, page_id) in page_map {
        let (width, height) = page_media_box(doc, page_id);
        let fonts = page_fonts(doc, page_id);

        let mut text = String::new();
        let mut has_image = false;
        let content = doc.get_page_content(page_id);
        if !content.is_empty() {
            if let Ok(decoded) = lopdf::content::Content::decode(&content) {
                let mut state = TextState::new();
                for op in &decoded.operations {
                    match op.operator.as_str() {
                        "BT" => state.begin_text(),
                        "ET" => state.end_text(),
                        "Tf" => {
                            if let Some(name) = op.operands.first().and_then(|o| name_of(o)) {
                                state.set_font(&name);
                            }
                            if let Some(size) = op.operands.get(1).and_then(|o| o.as_float().ok().map(|v| v as f64)) {
                                state.font_size = size;
                            }
                        }
                        "Td" | "TD" => {
                            let tx = op.operands.first().and_then(|o| o.as_float().ok().map(|v| v as f64)).unwrap_or(0.0);
                            let ty = op.operands.get(1).and_then(|o| o.as_float().ok().map(|v| v as f64)).unwrap_or(0.0);
                            // Vertical movement starts a new line; horizontal
                            // movement is usually a word gap.
                            if ty.abs() > 0.5 {
                                state.newline(&mut text);
                            } else if tx > state.font_size.max(1.0) * 0.2 {
                                state.space(&mut text);
                            }
                        }
                        "Tm" => {
                            let y = op.operands.get(5).and_then(|o| o.as_float().ok().map(|v| v as f64)).unwrap_or(0.0);
                            let x = op.operands.get(4).and_then(|o| o.as_float().ok().map(|v| v as f64)).unwrap_or(0.0);
                            if (y - state.last_y).abs() > 0.5 {
                                state.newline(&mut text);
                            } else if (x - state.last_x) > state.font_size.max(1.0) * 0.25 {
                                state.space(&mut text);
                            }
                            state.last_x = x;
                            state.last_y = y;
                        }
                        "T*" => state.newline(&mut text),
                        "Tj" => {
                            if let Some(bytes) = op.operands.first().and_then(string_bytes) {
                                state.show(bytes, &fonts, &mut text);
                            }
                        }
                        "'" => {
                            state.newline(&mut text);
                            if let Some(bytes) = op.operands.first().and_then(string_bytes) {
                                state.show(bytes, &fonts, &mut text);
                            }
                        }
                        "\"" => {
                            state.newline(&mut text);
                            if let Some(bytes) = op.operands.get(2).and_then(string_bytes) {
                                state.show(bytes, &fonts, &mut text);
                            }
                        }
                        "TJ" => {
                            if let Some(items) = op.operands.first().and_then(|o| o.as_array().ok()) {
                                for item in items {
                                    if let Ok(bytes) = string_bytes(item).ok_or(()) {
                                        state.show(bytes, &fonts, &mut text);
                                    } else if let Ok(adj) = item.as_float() {
                                        // Large negative kerning is a visual word gap.
                                        if adj < -180.0 {
                                            state.space(&mut text);
                                        }
                                    }
                                }
                            }
                        }
                        "Do" => {
                            if let Some(name) = op.operands.first().and_then(|o| name_of(o)) {
                                if xobject_is_image(doc, page_id, &name) {
                                    has_image = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let cleaned = tidy(&text);
        if cleaned.trim().is_empty() && has_image {
            needs_ocr.push(index);
        }
        pages.push(ExtractedPage {
            index,
            text: cleaned,
            width,
            height,
            has_image,
        });
        index += 1;
    }

    if pages.is_empty() {
        return Err(AppError::invalid("PDF 解析未产生任何页面"));
    }

    Ok(PdfExtraction {
        pages,
        pages_needing_ocr: needs_ocr,
    })
}

fn string_bytes(obj: &Object) -> Option<&[u8]> {
    match obj {
        Object::String(bytes, _) => Some(bytes),
        _ => None,
    }
}

/// Read a PDF name object as a UTF-8 string.
fn name_of(obj: &Object) -> Option<String> {
    obj.as_name()
        .ok()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
}

fn page_media_box(doc: &Document, page_id: ObjectId) -> (Option<f64>, Option<f64>) {
    let page = match doc.get_object(page_id).ok().and_then(|o| o.as_dict().ok()) {
        Some(d) => d,
        None => return (None, None),
    };
    let media = page
        .get(b"MediaBox")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_array().ok());
    let Some(arr) = media else {
        return (None, None);
    };
    if arr.len() < 4 {
        return (None, None);
    }
    let v: Vec<f64> = arr
        .iter()
        .map(|o| resolve(doc, o).as_float().map(|f| f as f64).unwrap_or(0.0))
        .collect();
    let w = (v[2] - v[0]).abs();
    let h = (v[3] - v[1]).abs();
    (Some(w), Some(h))
}

fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> &'a Object {
    match doc.dereference(obj) {
        Ok((_, resolved)) => resolved,
        Err(_) => obj,
    }
}

fn page_fonts(doc: &Document, page_id: ObjectId) -> HashMap<String, FontInfo> {
    let mut fonts = HashMap::new();
    let Some(page) = doc.get_object(page_id).ok().and_then(|o| o.as_dict().ok()) else {
        return fonts;
    };
    let Some(resources) = page
        .get(b"Resources")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return fonts;
    };
    let Some(font_dict) = resources
        .get(b"Font")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return fonts;
    };

    for (name, value) in font_dict.iter() {
        let font_ref = match value {
            Object::Reference(id) => Some(*id),
            _ => None,
        };
        let font_obj = resolve(doc, value);
        let Ok(dict) = font_obj.as_dict() else { continue };
        let subtype = dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| name_of(o))
            .unwrap_or_default();
        let two_byte = subtype == "Type0";

        let mut cmap = HashMap::new();
        if let Some(tu) = dict.get(b"ToUnicode").ok() {
            let stream_obj = resolve(doc, tu);
            if let Ok(stream) = stream_obj.as_stream() {
                if let Ok(bytes) = stream.decompressed_content() {
                    cmap = parse_to_unicode(&bytes);
                }
            }
        }
        // Also honour a descendant font's ToUnicode for Type0 fonts.
        if cmap.is_empty() && two_byte {
            if let Some(desc) = dict.get(b"DescendantFonts").ok().map(|o| resolve(doc, o)) {
                if let Ok(arr) = desc.as_array() {
                    if let Some(first) = arr.first() {
                        let d = resolve(doc, first);
                        if let Ok(dd) = d.as_dict() {
                            if let Some(tu) = dd.get(b"ToUnicode").ok() {
                                if let Ok(stream) = resolve(doc, tu).as_stream() {
                                    if let Ok(bytes) = stream.decompressed_content() {
                                        cmap = parse_to_unicode(&bytes);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        fonts.insert(
            String::from_utf8_lossy(name).to_string(),
            FontInfo {
                two_byte,
                cmap,
                font_ref,
            },
        );
    }
    fonts
}

fn xobject_is_image(doc: &Document, page_id: ObjectId, name: &str) -> bool {
    let Some(page) = doc.get_object(page_id).ok().and_then(|o| o.as_dict().ok()) else {
        return false;
    };
    let Some(resources) = page
        .get(b"Resources")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return false;
    };
    let Some(xobjects) = resources
        .get(b"XObject")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return false;
    };
    match xobjects.get(name.as_bytes()).ok().map(|o| resolve(doc, o)) {
        Some(Object::Stream(s)) => s
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| name_of(o))
            .map(|v| v == "Image")
            .unwrap_or(false),
        _ => false,
    }
}

struct FontInfo {
    two_byte: bool,
    cmap: HashMap<u32, String>,
    /// Kept for diagnosing CID fonts; not needed by the text path itself.
    #[allow(dead_code)]
    font_ref: Option<ObjectId>,
}

struct TextState {
    font: Option<String>,
    font_size: f64,
    last_x: f64,
    last_y: f64,
    at_line_start: bool,
}

impl TextState {
    fn new() -> Self {
        Self {
            font: None,
            font_size: 12.0,
            last_x: 0.0,
            last_y: 0.0,
            at_line_start: true,
        }
    }

    fn begin_text(&mut self) {
        self.at_line_start = true;
    }

    fn end_text(&mut self) {}

    fn set_font(&mut self, name: &str) {
        self.font = Some(name.to_string());
    }

    fn newline(&mut self, out: &mut String) {
        if !out.ends_with('\n') && !out.is_empty() {
            out.push('\n');
        }
        self.at_line_start = true;
    }

    fn space(&mut self, out: &mut String) {
        if !self.at_line_start && !out.ends_with(' ') && !out.ends_with('\n') {
            out.push(' ');
        }
    }

    fn show(&mut self, bytes: &[u8], fonts: &HashMap<String, FontInfo>, out: &mut String) {
        let font = self.font.as_deref().and_then(|n| fonts.get(n));
        let decoded = match font {
            Some(f) => decode_text(bytes, f),
            None => decode_winansi(bytes),
        };
        if decoded.is_empty() {
            return;
        }
        if self.at_line_start {
            self.at_line_start = false;
        }
        out.push_str(&decoded);
    }
}

/// Decode a PDF string into Unicode using the font's CMap when available.
fn decode_text(bytes: &[u8], font: &FontInfo) -> String {
    if font.cmap.is_empty() {
        // Type0 fonts without a ToUnicode map are usually Identity-H with an
        // embedded CID font: the text layer is not recoverable as Unicode.
        return if font.two_byte {
            String::new()
        } else {
            decode_winansi(bytes)
        };
    }

    let prefer_two_byte = font.two_byte || looks_two_byte(bytes, &font.cmap);
    let primary = decode_with_cmap(bytes, &font.cmap, prefer_two_byte);
    if primary.chars().any(|c| c != '\u{FFFD}') {
        return primary;
    }
    decode_with_cmap(bytes, &font.cmap, !prefer_two_byte)
}

fn looks_two_byte(bytes: &[u8], cmap: &HashMap<u32, String>) -> bool {
    if bytes.len() < 2 || bytes.len() % 2 != 0 {
        return false;
    }
    let two: usize = bytes
        .chunks_exact(2)
        .filter(|c| cmap.contains_key(&(u32::from(c[0]) << 8 | u32::from(c[1]))))
        .count();
    let one: usize = bytes.iter().filter(|b| cmap.contains_key(&u32::from(**b))).count();
    two > one
}

fn decode_with_cmap(bytes: &[u8], cmap: &HashMap<u32, String>, two_byte: bool) -> String {
    let mut out = String::new();
    if two_byte {
        for chunk in bytes.chunks(2) {
            let code = if chunk.len() == 2 {
                u32::from(chunk[0]) << 8 | u32::from(chunk[1])
            } else {
                u32::from(chunk[0])
            };
            match cmap.get(&code) {
                Some(s) => out.push_str(s),
                None => out.push('\u{FFFD}'),
            }
        }
    } else {
        for &b in bytes {
            match cmap.get(&u32::from(b)) {
                Some(s) => out.push_str(s),
                None => out.push_str(&decode_winansi(&[b])),
            }
        }
    }
    out.replace('\u{FFFD}', "")
}

/// CP1252-ish decoding for simple fonts. 0x80..0x9F differ from Latin-1.
fn decode_winansi(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        let c = match b {
            0x80 => '€',
            0x82 => '‚',
            0x83 => 'ƒ',
            0x84 => '„',
            0x85 => '…',
            0x86 => '†',
            0x87 => '‡',
            0x88 => 'ˆ',
            0x89 => '‰',
            0x8A => 'Š',
            0x8B => '‹',
            0x8C => 'Œ',
            0x8E => 'Ž',
            0x91 => '‘',
            0x92 => '’',
            0x93 => '“',
            0x94 => '”',
            0x95 => '•',
            0x96 => '–',
            0x97 => '—',
            0x98 => '˜',
            0x99 => '™',
            0x9A => 'š',
            0x9B => '›',
            0x9C => 'œ',
            0x9E => 'ž',
            0x9F => 'Ÿ',
            0x00..=0x1F => ' ',
            _ => b as char,
        };
        out.push(c);
    }
    out
}

/// Minimal `ToUnicode` CMap parser sufficient for text extraction.
///
/// Handles both `beginbfchar`/`endbfchar` pairs and `beginbfrange`/`endbfrange`
/// ranges (destination string or array forms). Codes are keyed by full
/// multi-byte value, so 1-byte and 2-byte fonts both work.
fn parse_to_unicode(data: &[u8]) -> HashMap<u32, String> {
    let text = String::from_utf8_lossy(data);
    let mut map = HashMap::new();

    // bfchar: <src> <dst>
    let mut rest = text.as_ref();
    while let Some(start) = rest.find("beginbfchar") {
        let after = &rest[start + "beginbfchar".len()..];
        let Some(end) = after.find("endbfchar") else { break };
        let block = &after[..end];
        // Source and destination are both hex tokens, so pair them positionally
        // instead of trying to classify each token on its own.
        let tokens: Vec<&str> = block.split_whitespace().collect();
        for pair in tokens.chunks(2) {
            if pair.len() != 2 {
                continue;
            }
            let (Some(source), Some(target)) =
                (parse_hex_token(pair[0]), parse_hex_string_token(pair[1]))
            else {
                continue;
            };
            map.insert(source, target);
        }
        rest = &after[end + "endbfchar".len()..];
    }

    // bfrange: <lo> <hi> <dstStart>  |  <lo> <hi> [ <d1> <d2> ... ]
    rest = text.as_ref();
    while let Some(start) = rest.find("beginbfrange") {
        let after = &rest[start + "beginbfrange".len()..];
        let Some(end) = after.find("endbfrange") else { break };
        let block = &after[..end];
        parse_bfrange_block(block, &mut map);
        rest = &after[end + "endbfrange".len()..];
    }

    map
}

fn parse_bfrange_block(block: &str, map: &mut HashMap<u32, String>) {
    let mut chars = block.trim().chars().peekable();
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    while let Some(c) = chars.next() {
        match c {
            '<' => current.push(c),
            '>' => {
                current.push(c);
                tokens.push(std::mem::take(&mut current));
            }
            '[' | ']' => {
                if !current.trim().is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(c.to_string());
            }
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let mut i = 0usize;
    while i + 2 < tokens.len() {
        let lo = parse_hex_token(&tokens[i]);
        let hi = parse_hex_token(&tokens[i + 1]);
        let (Some(lo), Some(hi)) = (lo, hi) else {
            i += 1;
            continue;
        };
        if tokens[i + 2] == "[" {
            let mut j = i + 3;
            let mut code = lo;
            while j < tokens.len() && tokens[j] != "]" {
                if let Some(dst) = parse_hex_string_token(&tokens[j]) {
                    if code <= hi {
                        map.insert(code, dst);
                    }
                    code += 1;
                }
                j += 1;
            }
            i = j + 1;
        } else if let Some(dst_start) = parse_hex_string_token(&tokens[i + 2]) {
            // Destination is incremented per code point.
            let base: Vec<u32> = dst_start.chars().map(|c| c as u32).collect();
            if base.is_empty() {
                i += 3;
                continue;
            }
            for offset in 0..=(hi.saturating_sub(lo).min(65535)) {
                let mut chars: Vec<char> = base.iter().map(|c| char::from_u32(*c).unwrap_or('\u{FFFD}')).collect();
                if let Some(last) = chars.last_mut() {
                    if let Some(next) = char::from_u32(*base.last().unwrap() + offset) {
                        *last = next;
                    }
                }
                map.insert(lo + offset, chars.into_iter().collect());
            }
            i += 3;
        } else {
            i += 1;
        }
    }
}

fn parse_hex_token(token: &str) -> Option<u32> {
    let t = token.trim();
    if !t.starts_with('<') {
        return None;
    }
    let inner = t.trim_start_matches('<').trim_end_matches('>');
    if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(inner, 16).ok()
}

fn parse_hex_string_token(token: &str) -> Option<String> {
    let t = token.trim();
    if !t.starts_with('<') {
        return None;
    }
    let inner = t.trim_start_matches('<').trim_end_matches('>');
    if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut units = Vec::new();
    let mut idx = 0;
    while idx + 4 <= inner.len() {
        if let Ok(v) = u16::from_str_radix(&inner[idx..idx + 4], 16) {
            units.push(v);
        }
        idx += 4;
    }
    if units.is_empty() && inner.len() >= 2 {
        if let Ok(v) = u16::from_str_radix(&inner[..2], 16) {
            units.push(v);
        }
    }
    while units.last() == Some(&0) {
        units.pop();
    }
    Some(String::from_utf16_lossy(&units))
}

/// Collapse extraction artifacts: repeated whitespace, hyphen line breaks.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.split('\n') {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            out.push('\n');
            continue;
        }
        blank_run = 0;
        // Join "deriva-\ntion" style hyphenation. The buffer always ends with the
        // previous line's newline, so inspect past it and then drop "-" + "\n".
        let join_hyphen = {
            let previous = out.trim_end_matches('\n');
            previous.ends_with('-') && !previous.ends_with(" -")
        };
        if join_hyphen && out.ends_with('\n') {
            out.pop(); // newline
            out.pop(); // hyphen
            out.push_str(trimmed.trim_start());
            out.push('\n');
        } else {
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winansi_decodes_smart_quotes() {
        assert_eq!(decode_winansi(&[0x93, b'h', b'i', 0x94]), "“hi”");
    }

    #[test]
    fn parses_bfchar_cmap() {
        let cmap = b"/CIDInit /ProcSet findresource begin\n\
                     1 beginbfchar\n<0003> <0020>\n<0024> <0041>\nendbfchar\n\
                     1 beginbfchar\n<0056> <4E2D>\nendbfchar\n";
        let map = parse_to_unicode(cmap);
        assert_eq!(map.get(&0x0003), Some(&" ".to_string()));
        assert_eq!(map.get(&0x0024), Some(&"A".to_string()));
        assert_eq!(map.get(&0x0056), Some(&"中".to_string()));
    }

    #[test]
    fn parses_bfrange_cmap() {
        let cmap = b"1 beginbfrange\n<0020> <0022> <0041>\nendbfrange\n";
        let map = parse_to_unicode(cmap);
        assert_eq!(map.get(&0x0020), Some(&"A".to_string()));
        assert_eq!(map.get(&0x0021), Some(&"B".to_string()));
        assert_eq!(map.get(&0x0022), Some(&"C".to_string()));
    }

    #[test]
    fn decodes_two_byte_codes() {
        let mut cmap = HashMap::new();
        cmap.insert(0x0001u32, "导".to_string());
        cmap.insert(0x0002u32, "数".to_string());
        let font = FontInfo {
            two_byte: true,
            cmap,
            font_ref: None,
        };
        let bytes = [0x00u8, 0x01, 0x00, 0x02];
        assert_eq!(decode_text(&bytes, &font), "导数");
    }

    #[test]
    fn tidy_collapses_blank_lines() {
        let raw = "line one\n\n\n\nline two\n";
        assert_eq!(tidy(raw), "line one\n\nline two");
    }

    #[test]
    fn tidy_joins_hyphenation() {
        assert_eq!(tidy("deriva-\ntion"), "derivation");
    }
}
