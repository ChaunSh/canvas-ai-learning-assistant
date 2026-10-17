//! DOCX text extraction.
//!
//! DOCX has no fixed page boundaries (pagination is a render-time decision), so
//! we emit a few synthetic "pages" split on headings. Citations for DOCX
//! therefore point at a section, never at a page number — the UI reflects that by
//! showing the section name instead of a page.

use std::io::Read;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::documents::chunker::ExtractedPage;
use crate::documents::plain::decode_entities;
use crate::documents::xmlutil::{attr, entity_char, local_name};
use crate::error::{AppError, AppResult};

/// Roughly how much text to place in one synthetic page.
const PAGE_TARGET_CHARS: usize = 2600;

pub fn extract(path: &std::path::Path) -> AppResult<Vec<ExtractedPage>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::invalid(format!("无法打开 DOCX 压缩包：{e}")))?;
    let mut xml = String::new();
    {
        let mut entry = archive
            .by_name("word/document.xml")
            .map_err(|_| AppError::invalid("DOCX 缺少 word/document.xml，文件可能已损坏"))?;
        entry.read_to_string(&mut xml)?;
    }
    let blocks = document_blocks(&xml);
    if blocks.is_empty() {
        return Err(AppError::invalid("DOCX 中没有可提取的文本"));
    }
    Ok(paginate(&blocks))
}

#[derive(Debug, Clone)]
struct Block {
    text: String,
    heading_level: Option<u8>,
    /// Table rows are flattened into `text` with ` | ` separators; the flag is
    /// kept so table-aware chunking can be added without reshaping the parser.
    #[allow(dead_code)]
    is_table_row: bool,
}

fn document_blocks(xml: &str) -> Vec<Block> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut blocks = Vec::new();
    let mut paragraph = String::new();
    let mut style: Option<String> = None;
    let mut in_text = false;
    let mut in_table = false;
    let mut row_cells: Vec<String> = Vec::new();
    let mut in_cell = false;
    // Text of the table cell currently being read (may span several <w:p>).
    let mut cell_text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name();
                match local_name(name.as_ref()) {
                    "p" => {
                        paragraph.clear();
                        style = None;
                    }
                    "pStyle" => {
                        style = attr(&e, "w:val").or_else(|| attr(&e, "val"));
                    }
                    "t" => in_text = true,
                    "tab" => paragraph.push('\t'),
                    "br" => paragraph.push('\n'),
                    "tbl" => in_table = true,
                    "tc" => {
                        in_cell = true;
                        cell_text.clear();
                        paragraph.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let name = e.name();
                match local_name(name.as_ref()) {
                    "tab" => paragraph.push('\t'),
                    "br" => paragraph.push('\n'),
                    // `<w:pStyle w:val="Heading1"/>` is self-closing in the wild.
                    "pStyle" => {
                        style = attr(&e, "w:val").or_else(|| attr(&e, "val"));
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if in_text {
                    paragraph.push_str(&decode_entities(&t.into_inner()));
                }
            }
            // Entity references arrive as a dedicated event in quick-xml 0.42.
            Ok(Event::GeneralRef(reference)) => {
                if in_text {
                    let resolved = reference
                        .resolve_char_ref()
                        .ok()
                        .flatten()
                        .or_else(|| entity_char(reference.as_ref()));
                    if let Some(ch) = resolved {
                        paragraph.push(ch);
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                match local_name(name.as_ref()) {
                    "t" => in_text = false,
                    "tc" => {
                        if in_cell {
                            row_cells.push(cell_text.trim().to_string());
                            cell_text.clear();
                            in_cell = false;
                        }
                    }
                    "tr" => {
                        if !row_cells.is_empty() {
                            blocks.push(Block {
                                text: row_cells.join(" | "),
                                heading_level: None,
                                is_table_row: true,
                            });
                            row_cells.clear();
                        }
                    }
                    "tbl" => in_table = false,
                    "p" => {
                        let text = paragraph.trim().to_string();
                        if in_cell {
                            // Accumulate the cell's paragraphs instead of
                            // emitting them as standalone blocks.
                            if !text.is_empty() {
                                if !cell_text.is_empty() {
                                    cell_text.push(' ');
                                }
                                cell_text.push_str(&text);
                            }
                        } else if !text.is_empty() && !in_table {
                            blocks.push(Block {
                                heading_level: style.as_deref().and_then(heading_level),
                                text,
                                is_table_row: false,
                            });
                        }
                        paragraph.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::warn!(error = %e, "malformed DOCX XML");
                break;
            }
            _ => {}
        }
    }
    blocks
}

/// Word style ids such as `Heading1`, `Heading 2`, `标题 1` (localised templates).
fn heading_level(style: &str) -> Option<u8> {
    let lower = style.to_lowercase();
    let digits: String = lower.chars().filter(|c| c.is_ascii_digit()).collect();
    if lower.contains("heading") || lower.starts_with('h') {
        return digits.parse::<u8>().ok().filter(|l| (1..=6).contains(l));
    }
    if lower.contains("标题") || lower.contains("title") {
        return Some(digits.parse::<u8>().unwrap_or(1).clamp(1, 6));
    }
    None
}

/// Group blocks into synthetic pages, breaking before top-level headings.
fn paginate(blocks: &[Block]) -> Vec<ExtractedPage> {
    let mut pages: Vec<ExtractedPage> = Vec::new();
    let mut current = String::new();

    let flush = |current: &mut String, pages: &mut Vec<ExtractedPage>| {
        let text = current.trim();
        if !text.is_empty() {
            pages.push(ExtractedPage {
                index: pages.len() as i64,
                text: text.to_string(),
                width: None,
                height: None,
                has_image: false,
            });
            current.clear();
        }
    };

    for block in blocks {
        let is_top_heading = matches!(block.heading_level, Some(1) | Some(2));
        let projected = current.chars().count() + block.text.chars().count();
        if (is_top_heading && current.chars().count() > 400) || projected > PAGE_TARGET_CHARS {
            flush(&mut current, &mut pages);
        }
        if let Some(level) = block.heading_level {
            let hashes = "#".repeat(level as usize);
            current.push_str(&format!("{hashes} {}\n", block.text));
        } else {
            current.push_str(&block.text);
            current.push('\n');
        }
    }
    flush(&mut current, &mut pages);
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_styles_are_recognised() {
        assert_eq!(heading_level("Heading1"), Some(1));
        assert_eq!(heading_level("Heading 3"), Some(3));
        assert_eq!(heading_level("标题 2"), Some(2));
        assert_eq!(heading_level("Normal"), None);
        assert_eq!(heading_level("Heading9"), None);
    }

    #[test]
    fn extracts_paragraphs_and_headings() {
        let xml = r#"<w:document xmlns:w="w"><w:body>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>第一章</w:t></w:r></w:p>
            <w:p><w:r><w:t>正文内容。</w:t></w:r></w:p>
            <w:tbl><w:tr>
              <w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc>
              <w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc>
            </w:tr></w:tbl>
        </w:body></w:document>"#;
        let blocks = document_blocks(xml);
        assert!(blocks
            .iter()
            .any(|b| b.heading_level == Some(1) && b.text == "第一章"));
        assert!(blocks.iter().any(|b| b.text == "正文内容。"));
        assert!(blocks.iter().any(|b| b.is_table_row && b.text == "A | B"));
    }

    #[test]
    fn pagination_breaks_on_headings() {
        let mut blocks = vec![Block {
            text: "intro ".repeat(100),
            heading_level: None,
            is_table_row: false,
        }];
        blocks.push(Block {
            text: "第二章".into(),
            heading_level: Some(1),
            is_table_row: false,
        });
        blocks.push(Block {
            text: "more".into(),
            heading_level: None,
            is_table_row: false,
        });
        let pages = paginate(&blocks);
        assert_eq!(pages.len(), 2);
        assert!(pages[1].text.starts_with("# 第二章"));
    }

    #[test]
    fn entities_in_text_are_decoded() {
        let xml = r#"<w:p><w:r><w:t>a &amp; b</w:t></w:r></w:p>"#;
        let blocks = document_blocks(xml);
        assert_eq!(blocks[0].text, "a & b");
    }
}
