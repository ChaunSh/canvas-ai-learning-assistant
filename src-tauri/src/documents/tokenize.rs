//! Text normalisation and FTS5 tokenisation.
//!
//! SQLite's `unicode61` tokenizer treats a run of Han characters as a single
//! token, which makes Chinese search behave badly (a query for 导数 would only
//! match documents containing exactly that whole run). We therefore write a
//! pre-tokenized copy of every chunk into the FTS table and tokenize queries the
//! same way:
//!
//!   * Latin letters/digits -> lowercased word tokens
//!   * CJK runs             -> overlapping bigrams (single char runs stay whole)
//!
//! Both sides use the identical function, so matching is symmetric.

/// Characters that should be treated as CJK for bigram expansion.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF      // CJK Extension A
        | 0x4E00..=0x9FFF    // CJK Unified Ideographs
        | 0xF900..=0xFAFF    // CJK Compatibility Ideographs
        | 0x3040..=0x30FF    // Hiragana / Katakana
        | 0xAC00..=0xD7AF    // Hangul syllables
        | 0x20000..=0x2FA1F  // CJK Extension B+
    )
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() && !is_cjk(c) || c == '_' || c == '\'' && false
}

/// Normalise full-width ASCII to half-width so `ＡＢＣ` matches `abc`.
fn normalize_char(c: char) -> char {
    match c as u32 {
        0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        0x3000 => ' ',
        _ => c,
    }
}

/// Produce the token stream used for both indexing and querying.
///
/// Returns lowercased tokens in reading order. Duplicates are preserved because
/// FTS5 positions matter for phrase queries.
pub fn tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut latin = String::new();
    let mut cjk: Vec<char> = Vec::new();

    let flush_latin = |latin: &mut String, out: &mut Vec<String>| {
        if !latin.is_empty() {
            let token = latin.to_lowercase();
            // Strip pure punctuation runs that survived classification.
            if token.chars().any(|c| c.is_alphanumeric()) {
                out.push(token);
            }
            latin.clear();
        }
    };

    for raw in text.chars() {
        let c = normalize_char(raw);
        if is_cjk(c) {
            flush_latin(&mut latin, &mut out);
            cjk.push(c);
        } else if is_word_char(c) {
            if !cjk.is_empty() {
                flush_cjk(&mut cjk, &mut out);
            }
            latin.push(c);
        } else if c.is_whitespace() {
            flush_latin(&mut latin, &mut out);
            flush_cjk(&mut cjk, &mut out);
        } else {
            // Punctuation: terminates the current run but is not indexed.
            flush_latin(&mut latin, &mut out);
            flush_cjk(&mut cjk, &mut out);
        }
    }
    flush_latin(&mut latin, &mut out);
    flush_cjk(&mut cjk, &mut out);

    out
}

fn flush_cjk(run: &mut Vec<char>, out: &mut Vec<String>) {
    if run.is_empty() {
        return;
    }
    if run.len() == 1 {
        out.push(run[0].to_string());
    } else {
        for pair in run.windows(2) {
            out.push(pair.iter().collect());
        }
    }
    run.clear();
}

/// The string stored in `chunk_fts.index_text`.
pub fn index_text(text: &str) -> String {
    tokens(text).join(" ")
}

/// Build an FTS5 MATCH expression.
///
/// `mode` selects AND (precise) or OR (recall). Tokens are always quoted so
/// FTS5 operators inside user input cannot change the query shape.
pub fn match_expression(text: &str, mode: MatchMode) -> Option<String> {
    let mut toks = tokens(text);
    toks.dedup();
    // Drop extremely common single Latin characters that only add noise.
    toks.retain(|t| t.chars().count() > 1 || !t.is_ascii());
    if toks.is_empty() {
        return None;
    }
    let joiner = match mode {
        MatchMode::And => " AND ",
        MatchMode::Or => " OR ",
    };
    let quoted: Vec<String> = toks
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    Some(quoted.join(joiner))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    And,
    Or,
}

/// Approximate token count used for context-budget calculations.
///
/// Latin words ≈ 1.3 tokens, CJK characters ≈ 0.9 tokens. This is deliberately
/// conservative; it only needs to keep prompts within the provider budget.
pub fn estimate_tokens(text: &str) -> i64 {
    let mut cjk = 0i64;
    let mut latin_chars = 0i64;
    let mut spaces = 0i64;
    for raw in text.chars() {
        let c = normalize_char(raw);
        if is_cjk(c) {
            cjk += 1;
        } else if c.is_alphanumeric() {
            latin_chars += 1;
        } else if c.is_whitespace() {
            spaces += 1;
        }
    }
    let latin_tokens = if latin_chars == 0 {
        0.0
    } else {
        // ~4.7 chars per word, ~1.3 tokens per word.
        (latin_chars as f64 / 4.7 * 1.3).max(spaces as f64 * 0.9)
    };
    (latin_tokens + cjk as f64 * 0.9).ceil() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_tokens_are_lowercased_words() {
        assert_eq!(tokens("Hello, World! 42"), vec!["hello", "world", "42"]);
    }

    #[test]
    fn cjk_runs_become_overlapping_bigrams() {
        assert_eq!(tokens("导数定义"), vec!["导数", "数定", "定义"]);
        assert_eq!(tokens("中"), vec!["中"]);
    }

    #[test]
    fn mixed_content_splits_correctly() {
        let t = tokens("第2章 Derivative 导数");
        assert_eq!(t, vec!["第", "2", "章", "derivative", "导数"]);
    }

    #[test]
    fn fullwidth_is_normalised() {
        assert_eq!(tokens("ＡＢＣ１２３"), vec!["abc123"]);
    }

    #[test]
    fn match_expression_quotes_tokens() {
        let and = match_expression("导数 定义", MatchMode::And).unwrap();
        assert_eq!(and, "\"导数\" AND \"定义\"");
        let or = match_expression("导数 定义", MatchMode::Or).unwrap();
        assert_eq!(or, "\"导数\" OR \"定义\"");
    }

    #[test]
    fn match_expression_escapes_quotes_and_rejects_empty() {
        assert!(match_expression("!!! ???", MatchMode::And).is_none());
        assert!(match_expression("", MatchMode::Or).is_none());
        // Single-character ASCII tokens are dropped as noise, so use longer ones.
        let expr = match_expression("alpha\"beta", MatchMode::And).unwrap();
        assert!(!expr.contains("alpha\"beta"), "raw quotes must not leak: {expr}");
        assert!(expr.contains("\"alpha\""));
    }

    #[test]
    fn sentence_punctuation_does_not_join_words() {
        assert_eq!(tokens("foo.bar"), vec!["foo", "bar"]);
    }

    #[test]
    fn estimate_is_positive_and_ordered() {
        let short = estimate_tokens("short");
        let long = estimate_tokens("this is a considerably longer piece of text with many words in it");
        assert!(short > 0);
        assert!(long > short);
        assert!(estimate_tokens("中文文本估算") > 0);
    }
}
