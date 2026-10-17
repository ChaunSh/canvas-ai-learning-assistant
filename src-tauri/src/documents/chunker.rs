//! Structure-aware chunking.
//!
//! Requirements we care about (task specification §26):
//!   * split on document structure (headings, paragraphs, slides) rather than a
//!     fixed character count;
//!   * never cut a formula in half;
//!   * carry a section breadcrumb so citations can name the section;
//!   * keep an overlap window so a definition spanning a boundary is still
//!     retrievable from either side.

use serde::{Deserialize, Serialize};

use crate::documents::tokenize::estimate_tokens;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedPage {
    /// 0-based page (PDF) or slide (PPTX) index.
    pub index: i64,
    pub text: String,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub has_image: bool,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub page_index: i64,
    pub ordinal: i64,
    pub kind: ChunkKind,
    pub section: Option<String>,
    pub context_prefix: Option<String>,
    pub content: String,
    pub token_estimate: i64,
    pub start_char: i64,
    pub end_char: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkKind {
    Heading,
    Paragraph,
    List,
    Formula,
    Definition,
    Theorem,
    Code,
    Table,
    Caption,
    Image,
}

impl ChunkKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChunkKind::Heading => "heading",
            ChunkKind::Paragraph => "paragraph",
            ChunkKind::List => "list",
            ChunkKind::Formula => "formula",
            ChunkKind::Definition => "definition",
            ChunkKind::Theorem => "theorem",
            ChunkKind::Code => "code",
            ChunkKind::Table => "table",
            ChunkKind::Caption => "caption",
            ChunkKind::Image => "image",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChunkOptions {
    /// Target characters per chunk before overlap.
    pub target_chars: usize,
    /// Characters of trailing context repeated at the start of the next chunk.
    pub overlap_chars: usize,
    /// Hard ceiling; an oversized block is split at sentence boundaries.
    pub max_chars: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        Self {
            target_chars: 900,
            overlap_chars: 140,
            max_chars: 1800,
        }
    }
}

/// A block of text on a page that has already been classified.
#[derive(Debug, Clone)]
struct Block {
    kind: ChunkKind,
    text: String,
    start: i64,
    end: i64,
}

/// Split a page's text into classified blocks.
fn blocks_for_page(page_index: i64, text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut offset = 0i64;
    let mut buffer = String::new();
    let mut buffer_start = 0i64;
    let mut in_code_fence = false;

    let flush = |buffer: &mut String, start: i64, end: i64, blocks: &mut Vec<Block>| {
        let trimmed = buffer.trim();
        if trimmed.is_empty() {
            buffer.clear();
            return;
        }
        let kind = classify(trimmed);
        blocks.push(Block {
            kind,
            text: trimmed.to_string(),
            start,
            end,
        });
        buffer.clear();
    };

    for line in text.split_inclusive('\n') {
        let line_start = offset;
        let line_end = offset + line.chars().count() as i64;
        offset = line_end;
        let trimmed = line.trim_end_matches(['\r', '\n']);

        if trimmed.trim_start().starts_with("```") {
            in_code_fence = !in_code_fence;
            if !in_code_fence {
                // Closing fence belongs to the code block.
                buffer.push_str(line);
                flush(&mut buffer, buffer_start, line_end, &mut blocks);
            } else {
                flush(&mut buffer, buffer_start, line_start, &mut blocks);
                buffer_start = line_start;
                buffer.push_str(line);
            }
            continue;
        }

        if in_code_fence {
            buffer.push_str(line);
            continue;
        }

        if trimmed.trim().is_empty() {
            flush(&mut buffer, buffer_start, line_start, &mut blocks);
            buffer_start = line_end;
            continue;
        }

        let is_boundary = is_heading(trimmed) || is_bullet(trimmed) || is_formula_line(trimmed);
        if is_boundary && !buffer.trim().is_empty() {
            flush(&mut buffer, buffer_start, line_start, &mut blocks);
            buffer_start = line_start;
        }
        if buffer.is_empty() {
            buffer_start = line_start;
        }
        buffer.push_str(line);
    }
    flush(&mut buffer, buffer_start, offset, &mut blocks);
    let _ = page_index;
    blocks
}

/// Heuristic heading detection for Chinese and English course material.
pub fn is_heading(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() || t.chars().count() > 80 {
        return false;
    }
    // Markdown headings
    if t.starts_with('#') {
        let hashes = t.chars().take_while(|c| *c == '#').count();
        return hashes <= 6 && t.chars().nth(hashes) == Some(' ');
    }
    // Ends like a sentence -> not a heading
    if t.ends_with('。') || t.ends_with('；') || t.ends_with('.') || t.ends_with(';') || t.ends_with(',') {
        return false;
    }
    let lower = t.to_lowercase();
    if lower.starts_with("chapter ") || lower.starts_with("section ") || lower.starts_with("part ") {
        return true;
    }
    if t.starts_with("第") && (t.contains('章') || t.contains('节') || t.contains('讲'))
        && t.chars().count() <= 40
    {
        return true;
    }
    // "1.2 Title" / "3 Title" enumeration
    let leading_digits = t.chars().take_while(|c| c.is_ascii_digit() || *c == '.').count();
    if leading_digits > 0 && leading_digits < t.chars().count() {
        let rest = &t[leading_digits..];
        if rest.starts_with(' ') && t.chars().count() <= 60 && !t.ends_with('。') {
            return true;
        }
    }
    false
}

fn is_bullet(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ")
        || t.starts_with("• ")
        || t.starts_with("· ")
        || t.starts_with("● ")
        || t.starts_with("▪ ")
        || t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
            && (t.contains(". ") || t.contains(") ") || t.contains("、"))
            && t.chars().take_while(|c| c.is_ascii_digit()).count() <= 2
}

/// Lines that look like standalone formulas must never be split.
pub fn is_formula_line(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 3 {
        return false;
    }
    if t.starts_with("$$") || (t.starts_with("\\[") && t.ends_with("\\]")) {
        return true;
    }
    let math_symbols = ['=', '∑', '∫', '∂', '√', '≤', '≥', '≠', '≈', '∈', '∀', '∃', '→', '⇒', '∞', '±', '×', '÷', '∇'];
    let symbol_hits = t.chars().filter(|c| math_symbols.contains(c)).count();
    let digit_hits = t.chars().filter(|c| c.is_ascii_digit()).count();
    let paren_hits = t.chars().filter(|c| *c == '(' || *c == ')').count();
    // Require a symbol and either digits or brackets, and a short dense line.
    symbol_hits >= 1 && (digit_hits + paren_hits) >= 1 && t.chars().count() <= 200
}

fn classify(text: &str) -> ChunkKind {
    let t = text.trim();
    if t.starts_with("```") {
        return ChunkKind::Code;
    }
    if is_heading(t) && !t.contains('\n') {
        return ChunkKind::Heading;
    }
    let lower = t.to_lowercase();
    let head: String = t.chars().take(40).collect();
    let head_lower = head.to_lowercase();
    if head_lower.starts_with("定义") || head_lower.starts_with("definition") {
        return ChunkKind::Definition;
    }
    if head_lower.starts_with("定理")
        || head_lower.starts_with("引理")
        || head_lower.starts_with("推论")
        || head_lower.starts_with("theorem")
        || head_lower.starts_with("lemma")
        || head_lower.starts_with("corollary")
        || head_lower.starts_with("proposition")
    {
        return ChunkKind::Theorem;
    }
    if head_lower.starts_with("证明") || head_lower.starts_with("proof") {
        return ChunkKind::Theorem;
    }
    if head_lower.starts_with("figure ") || head_lower.starts_with("图 ") || head_lower.starts_with("图1") {
        return ChunkKind::Caption;
    }
    if t.contains('\n') && t.lines().filter(|l| l.trim_start().starts_with("- ")).count() >= 2 {
        return ChunkKind::List;
    }
    if is_formula_line(t) && !t.contains('\n') {
        return ChunkKind::Formula;
    }
    if lower.contains(" | ") && t.lines().count() >= 2 {
        return ChunkKind::Table;
    }
    ChunkKind::Paragraph
}

/// Heading text used for the section breadcrumb.
fn heading_label(block: &Block) -> String {
    let t = block.text.trim();
    let t = t.trim_start_matches('#').trim();
    t.chars().take(80).collect()
}

/// Chunk extracted pages into retrieval units.
pub fn chunk_pages(pages: &[ExtractedPage], options: &ChunkOptions) -> Vec<Chunk> {
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut ordinal = 0i64;
    let mut section: Option<String> = None;
    let mut chapter: Option<String> = None;

    for page in pages {
        let blocks = blocks_for_page(page.index, &page.text);
        if blocks.is_empty() && page.has_image {
            chunks.push(Chunk {
                page_index: page.index,
                ordinal,
                kind: ChunkKind::Image,
                section: section.clone(),
                context_prefix: breadcrumb(&chapter, &section),
                content: format!("[第 {} 页为图像内容，无可用文本层]", page.index + 1),
                token_estimate: 20,
                start_char: 0,
                end_char: 0,
            });
            ordinal += 1;
            continue;
        }

        let mut current = String::new();
        let mut current_kind = ChunkKind::Paragraph;
        let mut current_start = 0i64;
        let mut current_end = 0i64;

        let push_chunk = |text: &str,
                              kind: ChunkKind,
                              start: i64,
                              end: i64,
                              section: &Option<String>,
                              chapter: &Option<String>,
                              chunks: &mut Vec<Chunk>,
                              ordinal: &mut i64| {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            chunks.push(Chunk {
                page_index: page.index,
                ordinal: *ordinal,
                kind,
                section: section.clone(),
                context_prefix: breadcrumb(chapter, section),
                content: trimmed.to_string(),
                token_estimate: estimate_tokens(trimmed),
                start_char: start,
                end_char: end,
            });
            *ordinal += 1;
        };

        for block in blocks {
            // Headings update the breadcrumb and are emitted on their own so a
            // citation can point at the section title.
            if block.kind == ChunkKind::Heading {
                if !current.trim().is_empty() {
                    push_chunk(&current, current_kind, current_start, current_end, &section, &chapter, &mut chunks, &mut ordinal);
                    current.clear();
                }
                let label = heading_label(&block);
                // Top-level headings (no leading number) start a new chapter.
                let starts_chapter = label
                    .chars()
                    .next()
                    .map(|c| !c.is_ascii_digit())
                    .unwrap_or(false);
                if starts_chapter || chapter.is_none() {
                    chapter = Some(label.clone());
                }
                section = Some(label);
                push_chunk(&block.text, block.kind, block.start, block.end, &section, &chapter, &mut chunks, &mut ordinal);
                continue;
            }

            let projected = current.chars().count() + block.text.chars().count() + 1;
            if current.is_empty() {
                current_start = block.start;
                current_kind = block.kind;
            } else if projected > options.target_chars {
                let overlap = tail_overlap(&current, options.overlap_chars);
                push_chunk(&current, current_kind, current_start, current_end, &section, &chapter, &mut chunks, &mut ordinal);
                current = overlap;
                current_start = block.start;
                current_kind = block.kind;
            }

            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(&block.text);
            current_end = block.end;

            // A single block above the hard ceiling is split at sentence ends.
            while current.chars().count() > options.max_chars {
                let split_at = sentence_split(&current, options.target_chars);
                let (head, tail) = split_string(&current, split_at);
                push_chunk(&head, current_kind, current_start, current_start + head.chars().count() as i64, &section, &chapter, &mut chunks, &mut ordinal);
                let overlap = tail_overlap(&head, options.overlap_chars);
                current = format!("{overlap}{tail}");
                current_start += head.chars().count() as i64;
            }
        }

        if !current.trim().is_empty() {
            push_chunk(&current, current_kind, current_start, current_end, &section, &chapter, &mut chunks, &mut ordinal);
        }
    }

    chunks
}

fn breadcrumb(chapter: &Option<String>, section: &Option<String>) -> Option<String> {
    match (chapter, section) {
        (Some(c), Some(s)) if c != s => Some(format!("{c} › {s}")),
        (Some(c), _) => Some(c.clone()),
        (None, Some(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Last `n` characters of `text`, extended back to a sentence boundary if one is
/// nearby (but never more than 2x `n`).
fn tail_overlap(text: &str, n: usize) -> String {
    let total = text.chars().count();
    if total <= n || n == 0 {
        return String::new();
    }
    let start = total - n;
    let slice: String = text.chars().skip(start).collect();
    // Extend to the previous sentence terminator inside the slice.
    if let Some(pos) = slice.find(['。', '.', '！', '？', '\n', '；', ';']) {
        let candidate: String = slice.chars().skip(pos + 1).collect();
        if candidate.chars().count() >= n / 2 {
            return candidate.trim().to_string();
        }
    }
    slice.trim().to_string()
}

/// Character index of the best sentence boundary at or before `target`.
fn sentence_split(text: &str, target: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let limit = target.min(chars.len());
    let mut best = limit;
    for i in (limit / 2..limit).rev() {
        if matches!(chars.get(i), Some('。' | '.' | '！' | '？' | '\n' | '；' | ';')) {
            return i + 1;
        }
        best = i;
    }
    best.max(1)
}

fn split_string(text: &str, at: usize) -> (String, String) {
    let head: String = text.chars().take(at).collect();
    let tail: String = text.chars().skip(at).collect();
    (head, tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(index: i64, text: &str) -> ExtractedPage {
        ExtractedPage {
            index,
            text: text.to_string(),
            width: None,
            height: None,
            has_image: false,
        }
    }

    #[test]
    fn headings_are_detected() {
        assert!(is_heading("第二章 极限"));
        assert!(is_heading("2.3 导数的定义"));
        assert!(is_heading("Chapter 4 Integration"));
        assert!(is_heading("# 课程介绍"));
        assert!(!is_heading("这是一个完整的句子。"));
        assert!(!is_heading("This is a normal sentence."));
        assert!(!is_heading(&"x".repeat(200)));
    }

    #[test]
    fn chunks_carry_section_and_page() {
        let pages = vec![page(
            0,
            "# 第一章\n\n导数描述函数的变化率。\n\n## 1.1 定义\n\n设函数 f(x) 在点 x0 处可导。",
        )];
        let chunks = chunk_pages(&pages, &ChunkOptions::default());
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| c.page_index == 0));
        assert!(chunks.iter().any(|c| c.section.as_deref().map(|s| s.contains("定义")).unwrap_or(false)));
        assert!(chunks.iter().any(|c| c.context_prefix.is_some()));
    }

    #[test]
    fn ordinals_are_contiguous_and_unique() {
        let pages = vec![
            page(0, "第一页内容。\n\n第二段内容。"),
            page(1, "第二页内容，包含更多文字。"),
        ];
        let chunks = chunk_pages(&pages, &ChunkOptions::default());
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.ordinal, i as i64);
        }
    }

    #[test]
    fn formulas_are_not_split_midway() {
        let pages = vec![page(0, "定理：\\[ f'(x) = \\lim_{h \\to 0} \\frac{f(x+h)-f(x)}{h} \\]\n\n以上为导数定义。")];
        let chunks = chunk_pages(&pages, &ChunkOptions::default());
        let formula_chunk = chunks.iter().find(|c| c.content.contains("\\lim"));
        assert!(formula_chunk.is_some());
        assert!(formula_chunk.unwrap().content.contains("\\frac"));
    }

    #[test]
    fn long_pages_respect_max_chars() {
        let long = "句子。".repeat(400);
        let pages = vec![page(0, &long)];
        let opts = ChunkOptions {
            target_chars: 200,
            overlap_chars: 20,
            max_chars: 400,
        };
        let chunks = chunk_pages(&pages, &opts);
        assert!(chunks.len() > 1);
        for c in &chunks {
            // Overlap may push a chunk slightly over target, never wildly over max.
            assert!(c.content.chars().count() <= opts.max_chars + opts.overlap_chars + 10);
        }
    }

    #[test]
    fn image_only_page_is_marked() {
        let pages = vec![ExtractedPage {
            index: 3,
            text: String::new(),
            width: Some(800.0),
            height: Some(600.0),
            has_image: true,
        }];
        let chunks = chunk_pages(&pages, &ChunkOptions::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].kind, ChunkKind::Image);
        assert_eq!(chunks[0].page_index, 3);
    }
}
