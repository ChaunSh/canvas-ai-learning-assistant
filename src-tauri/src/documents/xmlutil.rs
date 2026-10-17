//! Small helpers shared by the OOXML extractors.
//!
//! quick-xml 0.42 exposes names as `&str` (`QName(pub &'a str)`), so these
//! helpers work on string slices rather than byte slices.

use quick_xml::events::BytesStart;

/// Read an attribute value from a start/empty tag.
pub fn attr(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    for attribute in tag.attributes().flatten() {
        if attribute.key.as_ref() == name {
            return Some(attribute.value.to_string());
        }
    }
    None
}

/// Local name of a tag, dropping any namespace prefix.
pub fn local_name(name: &str) -> &str {
    match name.split_once(':') {
        Some((_, local)) => local,
        None => name,
    }
}

/// Resolve a named XML entity reference (the text between `&` and `;`).
///
/// Numeric references are handled by quick-xml's `resolve_char_ref`.
pub fn entity_char(reference: &str) -> Option<char> {
    match reference {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        "hellip" => Some('…'),
        "times" => Some('×'),
        "divide" => Some('÷'),
        "le" => Some('≤'),
        "ge" => Some('≥'),
        "ne" => Some('≠'),
        "rarr" => Some('→'),
        "infin" => Some('∞'),
        "sum" => Some('∑'),
        "int" => Some('∫'),
        _ => None,
    }
}

/// Numeric suffix of `ppt/slides/slide12.xml` -> 12.
pub fn trailing_number(path: &str) -> Option<u32> {
    let stem = path.rsplit('/').next()?.trim_end_matches(".xml");
    let digits: String = stem.chars().rev().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.chars().rev().collect::<String>().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_name_strips_prefix() {
        assert_eq!(local_name("w:t"), "t");
        assert_eq!(local_name("t"), "t");
    }

    #[test]
    fn trailing_number_reads_slide_index() {
        assert_eq!(trailing_number("ppt/slides/slide12.xml"), Some(12));
        assert_eq!(trailing_number("ppt/slides/_rels/slide3.xml.rels"), None);
        assert_eq!(trailing_number("ppt/slides/slide.xml"), None);
    }
}
