//! Content-type detection and extraction dispatch.

use std::path::Path;

use crate::documents::chunker::ExtractedPage;
use crate::documents::{docx, pdf, plain, pptx};
use crate::error::{AppError, AppResult};

/// Classify a file by extension first, then MIME type.
///
/// Returns one of the `materials.kind` values.
pub fn detect_kind(filename: &str, mime: Option<&str>) -> &'static str {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "pdf",
        "ppt" | "pptx" | "pps" | "ppsx" | "potx" => "pptx",
        "doc" | "docx" | "rtf" => "docx",
        "md" | "markdown" | "mdown" | "mkd" => "markdown",
        "txt" | "text" | "log" | "tex" => "text",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" => "image",
        "ics" | "ical" => "ics",
        "csv" => "csv",
        "json" => "json",
        "zip" => "zip",
        "html" | "htm" | "xhtml" => "other",
        _ => match mime.unwrap_or("") {
            m if m.starts_with("application/pdf") => "pdf",
            m if m.contains("presentationml") => "pptx",
            m if m.contains("wordprocessingml") => "docx",
            m if m.starts_with("text/") => "text",
            m if m.starts_with("image/") => "image",
            m if m.contains("calendar") => "ics",
            _ => "other",
        },
    }
}

pub fn guess_mime(filename: &str) -> Option<String> {
    mime_guess::from_path(filename)
        .first()
        .map(|m| m.essence_str().to_string())
}

/// Everything the indexing pipeline needs from a parsed document.
pub struct Extraction {
    pub pages: Vec<ExtractedPage>,
    pub needs_ocr: bool,
    pub page_count: i64,
    pub slide_count: i64,
    pub char_count: i64,
}

/// Extract text for a material. Formats we cannot parse return an explicit
/// `Unsupported` error rather than an empty document that would look indexed.
pub fn extract(path: &Path, kind: &str) -> AppResult<Extraction> {
    match kind {
        "pdf" => {
            let result = pdf::extract(path)?;
            let pages = result.pages;
            let char_count = pages.iter().map(|p| p.text.chars().count() as i64).sum();
            Ok(Extraction {
                page_count: pages.len() as i64,
                slide_count: 0,
                char_count,
                needs_ocr: !result.pages_needing_ocr.is_empty(),
                pages,
            })
        }
        "pptx" => {
            let pages = pptx::extract(path)?;
            let char_count = pages.iter().map(|p| p.text.chars().count() as i64).sum();
            let needs_ocr = pages.iter().any(|p| p.text.trim().is_empty() && p.has_image);
            Ok(Extraction {
                page_count: 0,
                slide_count: pages.len() as i64,
                char_count,
                needs_ocr,
                pages,
            })
        }
        "docx" => {
            let pages = docx::extract(path)?;
            let char_count = pages.iter().map(|p| p.text.chars().count() as i64).sum();
            Ok(Extraction {
                page_count: 0,
                slide_count: 0,
                char_count,
                needs_ocr: false,
                pages,
            })
        }
        "text" | "markdown" | "csv" | "json" => {
            let pages = plain::extract(path, kind)?;
            let char_count = pages.iter().map(|p| p.text.chars().count() as i64).sum();
            Ok(Extraction {
                page_count: 0,
                slide_count: 0,
                char_count,
                needs_ocr: false,
                pages,
            })
        }
        "image" => Ok(Extraction {
            // A raster image has no text layer; it is searchable only through the
            // multimodal path, which is explicit in the UI.
            pages: vec![ExtractedPage {
                index: 0,
                text: String::new(),
                width: None,
                height: None,
                has_image: true,
            }],
            needs_ocr: true,
            page_count: 0,
            slide_count: 0,
            char_count: 0,
        }),
        "ics" => {
            // ICS is imported as calendar events, not indexed as a document, but
            // we still make its text searchable so the file is not a dead end.
            let raw = plain::read_text(path)?;
            let text = raw.trim().to_string();
            Ok(Extraction {
                char_count: text.chars().count() as i64,
                pages: vec![ExtractedPage {
                    index: 0,
                    text,
                    width: None,
                    height: None,
                    has_image: false,
                }],
                needs_ocr: false,
                page_count: 0,
                slide_count: 0,
            })
        }
        other => Err(AppError::Unsupported(format!(
            "暂不支持自动解析该文件类型（{other}）。文件已保存，可在阅读器中查看或手动导入导出内容。"
        ))),
    }
}

/// True when a page number is meaningful for this material kind. Used to avoid
/// fabricating page numbers for formats without pagination.
pub fn has_real_pages(kind: &str) -> bool {
    matches!(kind, "pdf")
}

pub fn has_real_slides(kind: &str) -> bool {
    matches!(kind, "pptx")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension() {
        assert_eq!(detect_kind("lecture1.PDF", None), "pdf");
        assert_eq!(detect_kind("slides.pptx", None), "pptx");
        assert_eq!(detect_kind("notes.md", None), "markdown");
        assert_eq!(detect_kind("data.csv", None), "csv");
        assert_eq!(detect_kind("photo.JPG", None), "image");
        assert_eq!(detect_kind("unknown.xyz", Some("application/pdf")), "pdf");
        assert_eq!(detect_kind("unknown.xyz", Some("image/png")), "image");
        assert_eq!(detect_kind("unknown.xyz", None), "other");
    }

    #[test]
    fn page_numbers_only_for_paginated_formats() {
        assert!(has_real_pages("pdf"));
        assert!(!has_real_pages("docx"));
        assert!(!has_real_pages("pptx"));
        assert!(has_real_slides("pptx"));
        assert!(!has_real_slides("pdf"));
    }
}
