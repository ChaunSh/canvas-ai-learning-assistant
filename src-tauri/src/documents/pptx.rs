//! PPTX text extraction, one page per slide.
//!
//! A PPTX is a ZIP of DrawingML parts. We read `ppt/slides/slideN.xml`, keep the
//! slide order from the numeric filename suffix, and join `<a:t>` runs inside
//! each `<a:p>` paragraph. Slide size comes from `<p:sldSz>` in EMU.

use std::io::Read;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::documents::chunker::ExtractedPage;
use crate::documents::plain::decode_entities;
use crate::documents::xmlutil::{attr, entity_char, local_name, trailing_number};
use crate::error::{AppError, AppResult};

pub fn extract(path: &std::path::Path) -> AppResult<Vec<ExtractedPage>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::invalid(format!("无法打开 PPTX 压缩包：{e}")))?;

    let (width, height) = slide_size(&mut archive);

    let mut slide_names: Vec<(u32, String)> = archive
        .file_names()
        .filter_map(|entry| entry.ok().map(|name| name.to_string()))
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .filter_map(|name| trailing_number(&name).map(|idx| (idx, name)))
        .collect();
    slide_names.sort_by_key(|(idx, _)| *idx);

    if slide_names.is_empty() {
        return Err(AppError::invalid("PPTX 中未找到幻灯片"));
    }

    let mut pages = Vec::new();
    for (position, (_idx, name)) in slide_names.iter().enumerate() {
        let mut xml = String::new();
        {
            let mut entry = archive
                .by_name(name)
                .map_err(|e| AppError::invalid(format!("读取 {name} 失败：{e}")))?;
            entry.read_to_string(&mut xml)?;
        }
        let (text, has_image) = slide_text(&xml);
        pages.push(ExtractedPage {
            index: position as i64,
            text,
            width,
            height,
            has_image,
        });
    }
    Ok(pages)
}

fn slide_size(archive: &mut zip::ZipArchive<std::fs::File>) -> (Option<f64>, Option<f64>) {
    let mut xml = String::new();
    if let Ok(mut entry) = archive.by_name("ppt/presentation.xml") {
        if entry.read_to_string(&mut xml).is_err() {
            return (None, None);
        }
    } else {
        return (None, None);
    }
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == "sldSz" {
                    let cx = attr(&e, "cx").and_then(|v| v.parse::<f64>().ok());
                    let cy = attr(&e, "cy").and_then(|v| v.parse::<f64>().ok());
                    // EMU -> points (1pt = 12700 EMU)
                    return (cx.map(|v| v / 12700.0), cy.map(|v| v / 12700.0));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (None, None)
}

/// Extract paragraph text from one slide part.
fn slide_text(xml: &str) -> (String, bool) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut out = String::new();
    let mut paragraph = String::new();
    let mut in_text = false;
    let mut has_image = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name();
                match local_name(name.as_ref()) {
                    "t" => in_text = true,
                    "pic" | "blip" => has_image = true,
                    "br" => paragraph.push(' '),
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let name = e.name();
                if matches!(local_name(name.as_ref()), "pic" | "blip") {
                    has_image = true;
                }
            }
            Ok(Event::Text(t)) => {
                if in_text {
                    // Do not trim per fragment: text is split around entity
                    // references, so trimming each piece would eat real spaces.
                    paragraph.push_str(&decode_entities(&t.into_inner()));
                }
            }
            // quick-xml 0.42 emits entity references as their own event; without
            // this arm `a &amp; b` would degrade to `ab`.
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
                    "p" => {
                        let trimmed = paragraph.trim();
                        if !trimmed.is_empty() {
                            out.push_str(trimmed);
                            out.push('\n');
                        }
                        paragraph.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::warn!(error = %e, "malformed PPTX XML");
                break;
            }
            _ => {}
        }
    }
    (out.trim().to_string(), has_image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_paragraph_text() {
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
            <p:sp><p:txBody>
              <a:p><a:r><a:t>第一点：</a:t></a:r><a:r><a:t>导数定义</a:t></a:r></a:p>
              <a:p><a:r><a:t>Second point</a:t></a:r></a:p>
            </p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;
        let (text, has_image) = slide_text(xml);
        assert_eq!(text, "第一点：导数定义\nSecond point");
        assert!(!has_image);
    }

    #[test]
    fn detects_pictures() {
        let xml = r#"<p:sld><p:cSld><p:spTree><p:pic><p:blipFill/></p:pic></p:spTree></p:cSld></p:sld>"#;
        let (_, has_image) = slide_text(xml);
        assert!(has_image);
    }

    #[test]
    fn handles_entities() {
        let xml = r#"<a:p><a:r><a:t>a &amp; b &lt; c</a:t></a:r></a:p>"#;
        let (text, _) = slide_text(xml);
        assert_eq!(text, "a & b < c");
    }

    #[test]
    fn empty_slide_yields_empty_text() {
        let xml = r#"<p:sld><p:cSld><p:spTree/></p:cSld></p:sld>"#;
        let (text, has_image) = slide_text(xml);
        assert!(text.is_empty());
        assert!(!has_image);
    }
}
