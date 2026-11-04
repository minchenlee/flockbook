//! Swift `String` semantics: Characters are grapheme clusters; trimming sets as Foundation's.

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// Byte offset `i` starts a Character.
pub fn is_boundary(s: &str, i: usize) -> bool {
    if i == 0 || i == s.len() {
        return true;
    }
    if !s.is_char_boundary(i) {
        return false;
    }
    GraphemeCursor::new(i, s.len(), true).is_boundary(s, 0).unwrap_or(false)
}

/// `hasPrefix`: compares whole Characters.
pub fn has_prefix(s: &str, p: &str) -> bool {
    s.starts_with(p) && is_boundary(s, p.len())
}

/// `hasSuffix`: compares whole Characters.
pub fn has_suffix(s: &str, p: &str) -> bool {
    s.ends_with(p) && is_boundary(s, s.len() - p.len())
}

/// `prefix(n)` in Characters.
pub fn prefix(s: &str, n: usize) -> &str {
    match s.grapheme_indices(true).nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// `dropFirst(n)` in Characters.
pub fn drop_first(s: &str, n: usize) -> &str {
    match s.grapheme_indices(true).nth(n) {
        Some((i, _)) => &s[i..],
        None => "",
    }
}

/// `trimmingCharacters(in: .whitespacesAndNewlines)`.
pub fn trim(s: &str) -> &str {
    s.trim()
}

/// `CharacterSet.whitespaces`: general category Zs, and tab.
pub fn is_ws(c: char) -> bool {
    matches!(c, '\t' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

/// `trimmingCharacters(in: .whitespaces)`: newlines stay.
pub fn trim_ws(s: &str) -> &str {
    s.trim_matches(is_ws)
}

/// `Character.isNewline`, for a scalar on its own (newlines are always Characters of their own).
fn is_newline(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// `split(whereSeparator: \.isNewline).first`
pub fn first_line(s: &str) -> Option<&str> {
    s.split(is_newline).find(|p| !p.is_empty())
}

/// `split(separator: "\n", omittingEmptySubsequences:)`: "\r\n" is one Character, so it does not split there.
pub fn split_lf(s: &str, omit_empty: bool) -> Vec<&str> {
    let mut out = vec![];
    let mut start = 0;
    for (i, g) in s.grapheme_indices(true) {
        if g == "\n" {
            if !(omit_empty && i == start) {
                out.push(&s[start..i]);
            }
            start = i + 1;
        }
    }
    if !(omit_empty && start == s.len()) {
        out.push(&s[start..]);
    }
    out
}

/// `String(format:)` with one `%d`.
pub fn format_d(f: &str, n: usize) -> String {
    f.replacen("%d", &n.to_string(), 1)
}
