//! Text matching for search: case-insensitive substring over `char`s, so CJK and other non-ASCII text matches
//! exactly like ASCII. No byte offsets leave this file, so a match can never split a character.

use herdr_core::{ConversationItem, ItemKind};

/// Lower-cases one char the cheap way (first char of its lower-case form). Keeps the char count of the text.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// The query as folded chars. Leading and trailing white space is not part of the query.
pub fn fold_query(q: &str) -> Vec<char> {
    q.trim().chars().map(fold).collect()
}

/// Char index of the first match of `needle` (already folded) in `text`.
pub fn find(text: &str, needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let hay: Vec<char> = text.chars().map(fold).collect();
    find_in(&hay, needle)
}

fn find_in(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    let first = needle[0];
    let last_start = hay.len() - needle.len();
    let mut i = 0;
    while i <= last_start {
        if hay[i] == first && hay[i..i + needle.len()] == *needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub fn contains(text: &str, needle: &[char]) -> bool {
    find(text, needle).is_some()
}

/// A text split around its first match.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parts {
    pub before: String,
    pub hit: String,
    pub after: String,
}

impl Parts {
    pub fn plain(s: &str) -> Parts {
        Parts { before: s.to_string(), ..Default::default() }
    }
}

fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out
}

/// The whole text split at its first match (used for names).
pub fn split(text: &str, needle: &[char]) -> Parts {
    match find(text, needle) {
        None => Parts::plain(text),
        Some(at) => {
            let chars: Vec<char> = text.chars().collect();
            let end = (at + needle.len()).min(chars.len());
            Parts { before: chars[..at].iter().collect(), hit: chars[at..end].iter().collect(), after: chars[end..].iter().collect() }
        }
    }
}

/// A short excerpt around the first match: `before` chars of context in front, `after` chars behind, white space
/// collapsed, "…" where the text was cut.
pub fn snippet(text: &str, needle: &[char], before: usize, after: usize) -> Option<Parts> {
    let at = find(text, needle)?;
    let chars: Vec<char> = text.chars().collect();
    let end = (at + needle.len()).min(chars.len());
    let from = at.saturating_sub(before);
    let to = (end + after).min(chars.len());
    let mut b = collapse(&chars[from..at].iter().collect::<String>());
    // Keep a single space between the context and the hit when the cut text ended in white space.
    if chars[from..at].last().is_some_and(|c| c.is_whitespace()) && !b.is_empty() {
        b.push(' ');
    }
    let mut a = collapse(&chars[end..to].iter().collect::<String>());
    if chars.get(end).is_some_and(|c| c.is_whitespace()) && !a.is_empty() {
        a.insert(0, ' ');
    }
    if from > 0 {
        b.insert(0, '\u{2026}');
    }
    if to < chars.len() {
        a.push('\u{2026}');
    }
    Some(Parts { before: b, hit: chars[at..end].iter().collect(), after: a })
}

/// What a message is, for the result row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    You,
    Agent,
    Tool,
    Other,
}

/// One searchable text of a conversation item.
#[derive(Debug, Clone)]
pub struct Entry {
    pub pane: String,
    pub item: String,
    pub role: Role,
    pub text: String,
    pub ts: Option<f64>,
}

/// Bytes of the main text of an item (0 for kinds without one). Cheap: used in the corpus signature.
pub fn text_len(kind: &ItemKind) -> usize {
    match kind {
        ItemKind::UserText(s) | ItemKind::AssistantText(s) => s.len(),
        _ => 0,
    }
}

/// A rough size of the searchable text of `items`, without building it: text bytes of messages plus a flat guess per other
/// item. Decides inline or worker.
pub fn estimate_chars<'a>(items: impl Iterator<Item = &'a ConversationItem>) -> usize {
    items.map(|it| match &it.kind {
        ItemKind::UserText(s) | ItemKind::AssistantText(s) => s.len(),
        _ => 400,
    }).sum()
}

/// The searchable texts of one item (zero or more).
pub fn texts_of(pane: &str, it: &ConversationItem) -> Vec<Entry> {
    let mk = |role: Role, text: String| Entry { pane: pane.to_string(), item: it.id.clone(), role, text, ts: it.timestamp };
    match &it.kind {
        ItemKind::UserText(s) => vec![mk(Role::You, s.clone())],
        ItemKind::AssistantText(s) => vec![mk(Role::Agent, s.clone())],
        ItemKind::ToolChips(chips) => {
            let mut parts: Vec<String> = Vec::new();
            for c in chips {
                parts.push(c.text.clone());
                parts.extend(c.calls.iter().map(|k| k.summary.clone()));
            }
            vec![mk(Role::Tool, parts.join(" \u{b7} "))]
        }
        ItemKind::PermissionRequest(p) => vec![mk(Role::Tool, format!("{} {}", p.title, p.command))],
        ItemKind::Question(q) => {
            let t: Vec<String> = q.questions.iter().map(|x| format!("{} {}", x.header, x.text)).collect();
            vec![mk(Role::Other, t.join(" "))]
        }
        ItemKind::Status(s) => vec![mk(Role::Other, s.clone())],
        ItemKind::Images(_) => Vec::new(),
    }
}

/// A raw message hit, before it is joined with the tab it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    pub pane: String,
    pub item: String,
    pub role: Role,
    pub parts: Parts,
    pub ts: Option<f64>,
}

/// Searches `entries` (in order). Newest first within a pane (entries are oldest first), at most `per_pane` hits
/// per pane and `total` in all. Panes keep the order of their first entry.
pub fn search(entries: &[Entry], needle: &[char], per_pane: usize, total: usize) -> Vec<Raw> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<&str> = Vec::new();
    for e in entries {
        if !order.contains(&e.pane.as_str()) {
            order.push(&e.pane);
        }
    }
    let mut out: Vec<Raw> = Vec::new();
    for pane in order {
        let mut n = 0;
        for e in entries.iter().rev().filter(|e| e.pane == pane) {
            if let Some(parts) = snippet(&e.text, needle, 32, 96) {
                out.push(Raw { pane: e.pane.clone(), item: e.item.clone(), role: e.role, parts, ts: e.ts });
                n += 1;
                if n >= per_pane || out.len() >= total {
                    break;
                }
            }
        }
        if out.len() >= total {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_ignore_case_and_cjk_is_safe() {
        let n = fold_query("  Gala ");
        assert_eq!(find("Royal GALA apples", &n), Some(6));
        let n = fold_query("衰減");
        assert_eq!(find("長尾查詢可能受到 衰減 影響", &n), Some(9));
        assert!(find("", &n).is_none());
        assert!(find("abc", &[]).is_none());
        assert!(contains("Straße ÄPFEL", &fold_query("äpfel")));
    }

    #[test]
    fn snippet_cuts_on_char_boundaries_and_marks_the_cut() {
        let text = "前".repeat(60) + "命中" + &"後".repeat(200);
        let p = snippet(&text, &fold_query("命中"), 10, 20).unwrap();
        assert_eq!(p.hit, "命中");
        assert!(p.before.starts_with('\u{2026}') && p.after.ends_with('\u{2026}'));
        assert_eq!(p.before.chars().count(), 11);
        assert_eq!(p.after.chars().count(), 21);
        let p = snippet("a\n\n  b   needle\tc", &fold_query("NEEDLE"), 40, 40).unwrap();
        assert_eq!((p.before.as_str(), p.hit.as_str(), p.after.as_str()), ("a b ", "needle", " c"));
    }

    #[test]
    fn split_keeps_the_whole_text() {
        let p = split("quarry-rank-dev", &fold_query("RANK"));
        assert_eq!((p.before.as_str(), p.hit.as_str(), p.after.as_str()), ("quarry-", "rank", "-dev"));
        assert_eq!(split("x", &fold_query("y")), Parts::plain("x"));
    }
}
