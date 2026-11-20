//! Resources: what an agent points the human to in its reply (contract: herdr-app/docs/protocol.md, `row`, `links`).
//! Pure. `detect` finds the targets in an assistant text; feed rows carry them as `links`, and the `resources`
//! command (P1) collects them with the same rules.

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use serde::Serialize;

/// A target in a row's `text`: UTF-16 offsets (what the mod's JavaScript strings count) and its kind.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FeedLink {
    pub start: usize,
    pub end: usize,
    pub res: &'static str,
}

/// One target: `start`..`end` in UTF-16 units, its kind, and the target as it opens (a URL with a scheme, or the
/// path as written).
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub kind: &'static str,
    pub target: String,
}

const DOC_EXT: [&str; 6] = ["md", "html", "htm", "pdf", "png", "svg"];
/// Full-width punctuation that ends a path or a URL (Chinese and Japanese text has no space before them).
const CJK_PUNCT: &str = "。，、；：！？「」『』（）《》【】…—“”‘’";

/// The targets in `text`, in order, none overlapping. The text is read as CommonMark (pulldown-cmark, with tables,
/// strikethrough and task lists); what is found:
/// - Inline links `[label](dest)` and autolinks `<url>`: the address only, once (the label is not searched).
/// - A code span that holds only a target; nothing else inside code spans or code blocks.
/// - In plain text only: bare `http(s)://` URLs; `localhost:<port>` and `127.0.0.1:<port>`, and
///   `claude.ai/artifact/…` and GitHub PR/issue/run addresses, also without a scheme; absolute paths (`/…`, `~/…`)
///   that end in a doc extension.
///
/// Not searched: code, HTML (and the rest of a block after an inline tag: it may open code or a link), pictures, reference links (`[a][b]`, `[a][]`, `[a]` with a definition) and reference
/// definitions (the parser takes them; they are not text). Text whose source differs from what it reads (an escape
/// `\[`, an entity `&amp;`) is not searched, and a target that runs into it is not found: to be safe.
/// Punctuation at the end (`.` `,` `)` without its `(`, `。`…) is not part of a target.
pub fn detect(text: &str) -> Vec<Found> {
    // Only plain text is searched: everything starts taken, a text event as written is freed.
    let mut taken = vec![true; text.len()];
    let mut hits: Vec<(usize, usize, &'static str, String)> = vec![];
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    // Inside a code block, a link, a picture or an HTML block: nothing is searched.
    let mut inside = 0usize;
    // After an inline HTML tag, until its block ends: the tag may open code or a link.
    let mut html = false;
    for (ev, r) in Parser::new_ext(text, opts).into_offset_iter() {
        match ev {
            Event::Start(Tag::Link { link_type, dest_url, .. }) => {
                if inside == 0 {
                    let dest = match link_type {
                        LinkType::Inline => inline_dest(text, r.clone(), &dest_url),
                        // "<url>": the address is the text between the brackets.
                        LinkType::Autolink if text[r.clone()].len() == dest_url.len() + 2 && text[r.start + 1..r.end - 1] == *dest_url => {
                            Some((r.start + 1, r.end - 1))
                        }
                        _ => None,
                    };
                    if let Some((ds, de)) = dest {
                        if let Some((kind, target)) = whole(&text[ds..de]) {
                            hits.push((ds, de, kind, target));
                        }
                    }
                }
                inside += 1;
            }
            Event::Start(Tag::CodeBlock(_) | Tag::Image { .. } | Tag::HtmlBlock) => inside += 1,
            Event::End(TagEnd::Link | TagEnd::CodeBlock | TagEnd::Image | TagEnd::HtmlBlock) => inside = inside.saturating_sub(1),
            Event::InlineHtml(_) => html = true,
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell | TagEnd::Item) => html = false,
            Event::Text(t) if inside == 0 && !html && text[r.clone()] == *t => mark(&mut taken, r.start, r.end, false),
            Event::Code(c) if inside == 0 && !html => {
                // A span that holds only a target: "`x`", "`` x ``".
                let src = &text[r.clone()];
                let n = src.bytes().take_while(|&x| x == b'\x60').count();
                if src.len() < 2 * n {
                    continue;
                }
                let inner = &src[n..src.len() - n];
                let lead = inner.len() - inner.trim_start().len();
                let tok = inner.trim();
                if tok == c.trim() {
                    if let Some((kind, target)) = whole(tok) {
                        let s = r.start + n + lead;
                        hits.push((s, s + tok.len(), kind, target));
                    }
                }
            }
            _ => {}
        }
    }
    bare_urls(text, &mut taken, &mut hits);
    bare_paths(text, &mut taken, &mut hits);
    hits.sort_by_key(|h| h.0);
    // Byte offsets to UTF-16, in one pass.
    let mut out = vec![];
    let (mut at, mut u) = (0usize, 0usize);
    let mut to16 = |byte: usize| {
        u += text[at..byte].encode_utf16().count();
        at = byte;
        u
    };
    for (s, e, kind, target) in hits {
        let start = to16(s);
        let end = to16(e);
        out.push(Found { start, end, kind, target });
    }
    out
}

/// The address of an inline link `[label](dest)` in `text[r]`, as written: after "](", spaces or a line break, and
/// maybe "<". None when it is not written as the parser read it (an escape or an entity in it).
fn inline_dest(text: &str, r: std::ops::Range<usize>, dest: &str) -> Option<(usize, usize)> {
    if dest.is_empty() {
        return None;
    }
    let src = &text[r.clone()];
    for (p, _) in src.match_indices(dest) {
        let before = src[..p].trim_end_matches('<').trim_end_matches([' ', '\t', '\n', '\r']);
        let after = src[p + dest.len()..].trim_start_matches('>');
        if before.ends_with("](") && after.starts_with([' ', '\t', '\n', '\r', ')']) {
            return Some((r.start + p, r.start + p + dest.len()));
        }
    }
    None
}

fn mark(taken: &mut [bool], s: usize, e: usize, to: bool) {
    taken[s..e].iter_mut().for_each(|x| *x = to);
}

fn is_url(s: &str) -> bool {
    let l = s.get(..8).unwrap_or(s).to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

/// A whole token that is one target: its kind and target.
fn whole(tok: &str) -> Option<(&'static str, String)> {
    if tok.is_empty() || tok.contains(char::is_whitespace) {
        return None;
    }
    if is_url(tok) {
        return host_of(tok).map(|_| (url_kind(tok), tok.to_string()));
    }
    if let Some(t) = bare_host(tok) {
        return Some((url_kind(&t), t));
    }
    if (tok.starts_with('/') && !tok.starts_with("//") || tok.starts_with("~/")) && doc_end(tok, 0, tok.len()) == Some(tok.len()) {
        return Some(("doc", tok.to_string()));
    }
    None
}

/// A scheme-less address that is still one: `localhost:<port>`, `127.0.0.1:<port>`, a claude.ai artifact, a GitHub
/// PR, issue or run. The URL it opens.
fn bare_host(tok: &str) -> Option<String> {
    let l = tok.to_ascii_lowercase();
    for h in ["localhost:", "127.0.0.1:"] {
        if let Some(rest) = l.strip_prefix(h) {
            return (rest.bytes().next().is_some_and(|c| c.is_ascii_digit())).then(|| format!("http://{tok}"));
        }
    }
    if l.starts_with("claude.ai/") || l.starts_with("github.com/") {
        let u = format!("https://{tok}");
        return matches!(url_kind(&u), "artifact" | "pr" | "issue" | "ci").then_some(u);
    }
    None
}

/// The host of a URL with a scheme, lowercased, without user and port; None without one.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let auth = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let h = auth.rsplit('@').next().unwrap_or(auth);
    let h = if h.starts_with('[') { &h[..h.find(']').map(|k| k + 1).unwrap_or(h.len())] } else { h.split(':').next().unwrap_or(h) };
    (!h.is_empty()).then(|| h.to_ascii_lowercase())
}

fn is_local(h: &str) -> bool {
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") || h == "0.0.0.0" || h == "[::1]" {
        return true;
    }
    let p: Vec<u8> = h.split('.').filter_map(|x| x.parse().ok()).collect();
    if p.len() != 4 || h.split('.').count() != 4 {
        return false;
    }
    p[0] == 127 || p[0] == 10 || (p[0] == 192 && p[1] == 168) || (p[0] == 172 && (16..=31).contains(&p[1])) || (p[0] == 169 && p[1] == 254)
}

/// The kind of a URL with a scheme: artifact, pr, issue, ci, local or link.
pub fn url_kind(url: &str) -> &'static str {
    let Some(host) = host_of(url) else { return "link" };
    if is_local(&host) {
        return "local";
    }
    let rest = url.split_once("://").map(|x| x.1).unwrap_or("");
    let path = rest.find('/').map(|k| &rest[k..]).unwrap_or("");
    let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
    let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let num = |s: Option<&&str>| s.is_some_and(|s| s.bytes().all(|c| c.is_ascii_digit()));
    match host.trim_start_matches("www.") {
        "claude.ai" => {
            let s: &[&str] = if seg.first() == Some(&"code") { &seg[1..] } else { &seg };
            if s.first() == Some(&"artifact") && s.len() >= 2 {
                return "artifact";
            }
        }
        "github.com" if seg.len() >= 4 => match seg[2] {
            "pull" if num(seg.get(3)) => return "pr",
            "issues" if num(seg.get(3)) => return "issue",
            "actions" if seg[3] == "runs" && num(seg.get(4)) => return "ci",
            _ => {}
        },
        _ => {}
    }
    "link"
}

/// Ends of a sentence, not of a URL: dropped from its end. A `)` or `]` only when it has no partner inside.
fn trim_end(s: &str) -> &str {
    let mut s = s;
    loop {
        let Some(c) = s.chars().last() else { return s };
        let drop = match c {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | '*' | '~' => true,
            ')' => s.matches('(').count() < s.matches(')').count(),
            ']' => s.matches('[').count() < s.matches(']').count(),
            _ => false,
        };
        if !drop {
            return s;
        }
        s = &s[..s.len() - c.len_utf8()];
    }
}

fn url_byte(c: u8) -> bool {
    c.is_ascii_graphic() && !b"<>\"\x60{}|\\^".contains(&c)
}

/// A place a bare target may start: the text's start, or after a character that cannot be part of one.
fn starts_here(text: &str, i: usize) -> bool {
    match text[..i].chars().last() {
        None => true,
        Some(c) => !(c.is_ascii_alphanumeric() || "/._-~:@%+=#$&\\".contains(c)),
    }
}

fn bare_urls(text: &str, taken: &mut [bool], hits: &mut Vec<(usize, usize, &'static str, String)>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if taken[i] || !text.is_char_boundary(i) || !starts_here(text, i) {
            i += 1;
            continue;
        }
        let rest = &b[i..];
        let l = String::from_utf8_lossy(&rest[..rest.len().min(12)]).to_ascii_lowercase();
        let schemed = l.starts_with("http://") || l.starts_with("https://");
        let hosty = ["localhost:", "127.0.0.1:", "claude.ai/", "github.com/"].iter().any(|h| l.starts_with(h));
        if !schemed && !hosty {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < b.len() && url_byte(b[j]) {
            j += 1;
        }
        let tok = trim_end(&text[i..j]);
        let e = i + tok.len();
        // A target that runs into markup, an escape or an entity is not found: its end is not sure. "\\_" after it
        // reads as "_", a part of it (a "\\" that ends the line is a line break).
        let escaped = b.get(j) == Some(&b'\\') && !matches!(b.get(j + 1), None | Some(b'\n' | b'\r'));
        if escaped || taken[i..e].iter().any(|&x| x) {
            i = j.max(i + 1);
            continue;
        }
        let found = if schemed { host_of(tok).map(|_| (url_kind(tok), tok.to_string())) } else { bare_host(tok).map(|t| (url_kind(&t), t)) };
        match found {
            Some((kind, target)) => {
                hits.push((i, e, kind, target));
                mark(taken, i, e, true);
                i = e;
            }
            None => i = j.max(i + 1),
        }
    }
}

fn path_char(c: char) -> bool {
    !(c.is_whitespace() || c.is_control() || "\x60<>\"'|()[]{}*,;".contains(c) || CJK_PUNCT.contains(c))
}

/// The end of the last doc extension in `text[s..j]` that ends the path: nothing that continues a name after it.
fn doc_end(text: &str, s: usize, j: usize) -> Option<usize> {
    let tok = &text[s..j];
    let b = text.as_bytes();
    let mut best = None;
    for (p, _) in tok.match_indices('.') {
        let after = &tok[p + 1..];
        for ext in DOC_EXT {
            if after.len() < ext.len() || !after.is_char_boundary(ext.len()) || !after[..ext.len()].eq_ignore_ascii_case(ext) {
                continue;
            }
            let e = s + p + 1 + ext.len();
            let next = b.get(e).copied();
            let ends = match next {
                None => true,
                Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b'/' || c == b'-' => false,
                Some(b'.') => !b.get(e + 1).is_some_and(|c| c.is_ascii_alphanumeric()),
                _ => true,
            };
            if ends && p > 0 {
                best = Some(e);
            }
        }
    }
    best
}

fn bare_paths(text: &str, taken: &mut [bool], hits: &mut Vec<(usize, usize, &'static str, String)>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let begins = match b[i] {
            b'/' => b.get(i + 1).is_some_and(|&c| c != b'/' && !c.is_ascii_whitespace()),
            b'~' => b.get(i + 1) == Some(&b'/'),
            _ => false,
        };
        if taken[i] || !begins || !starts_here(text, i) {
            i += 1;
            continue;
        }
        let mut j = i;
        for (k, c) in text[i..].char_indices() {
            if !path_char(c) {
                break;
            }
            j = i + k + c.len_utf8();
        }
        match doc_end(text, i, j) {
            Some(e) if taken[i..e].iter().any(|&x| x) => i = j.max(i + 1),
            Some(e) => {
                hits.push((i, e, "doc", text[i..e].to_string()));
                mark(taken, i, e, true);
                i = e;
            }
            None => i = j.max(i + 1),
        }
    }
}

pub fn links(text: &str) -> Vec<FeedLink> {
    detect(text).into_iter().map(|f| FeedLink { start: f.start, end: f.end, res: f.kind }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> (usize, usize, &'static str, String) {
        let f = detect(text);
        assert_eq!(f.len(), 1, "one target in {text:?}: {f:?}");
        (f[0].start, f[0].end, f[0].kind, f[0].target.clone())
    }
    fn kinds(text: &str) -> Vec<&'static str> {
        detect(text).iter().map(|f| f.kind).collect()
    }
    fn u16len(s: &str) -> usize {
        s.encode_utf16().count()
    }

    #[test]
    fn every_kind() {
        assert_eq!(one("Report: /home/me/reports/plan.md").2, "doc");
        for p in ["~/notes/a.html", "/tmp/b.htm", "/tmp/c.pdf", "/tmp/d.png", "/tmp/e.svg", "/tmp/F.PDF"] {
            assert_eq!(one(&format!("See {p} now")), (4, 4 + u16len(p), "doc", p.to_string()), "{p}");
        }
        assert_eq!(one("https://claude.ai/artifact/abc-123").2, "artifact");
        assert_eq!(one("https://claude.ai/code/artifact/0f3a").2, "artifact");
        assert_eq!(one("page: claude.ai/artifact/xyz").3, "https://claude.ai/artifact/xyz");
        assert_eq!(one("https://github.com/o/r/pull/12").2, "pr");
        assert_eq!(one("https://github.com/o/r/pull/12/files").2, "pr");
        assert_eq!(one("https://github.com/o/r/issues/7").2, "issue");
        assert_eq!(one("https://github.com/o/r/actions/runs/1870").2, "ci");
        assert_eq!(one("https://github.com/o/r").2, "link");
        assert_eq!(one("PR: github.com/o/r/pull/3").3, "https://github.com/o/r/pull/3");
        assert_eq!(one("https://example.com/a?b=1#c").2, "link");
        assert_eq!(one("http://localhost:5173/").2, "local");
        assert_eq!(one("open localhost:3000 now").3, "http://localhost:3000");
        assert_eq!(one("http://192.168.1.5:8080/x").2, "local");
        assert_eq!(one("http://127.0.0.1:8787").2, "local");
        assert_eq!(one("http://box.local:9000").2, "local");
    }

    #[test]
    fn chinese_mixed_with_english() {
        let t = "報告在 /home/me/r.md，PR 在 https://github.com/o/r/pull/3。";
        let f = detect(t);
        assert_eq!(f.len(), 2, "{f:?}");
        let s0 = u16len("報告在 ");
        assert_eq!((f[0].start, f[0].end, f[0].kind), (s0, s0 + u16len("/home/me/r.md"), "doc"));
        let s1 = u16len("報告在 /home/me/r.md，PR 在 ");
        assert_eq!((f[1].start, f[1].end, f[1].kind), (s1, s1 + u16len("https://github.com/o/r/pull/3"), "pr"));
        assert_eq!(one("見https://x.com/a的說明").3, "https://x.com/a");
        assert_eq!(one("檔案：/home/me/筆記/計畫.md的內容").3, "/home/me/筆記/計畫.md");
    }

    #[test]
    fn punctuation_at_the_end_is_not_part() {
        assert_eq!(one("Done: https://x.com/a。").3, "https://x.com/a");
        assert_eq!(one("(see https://x.com/a)").3, "https://x.com/a");
        assert_eq!(one("https://en.wikipedia.org/wiki/Foo_(bar)").3, "https://en.wikipedia.org/wiki/Foo_(bar)");
        assert_eq!(one("https://x.com/a, and more").3, "https://x.com/a");
        assert_eq!(one("It is https://x.com/a.").3, "https://x.com/a");
        assert_eq!(one("**https://x.com/a**").3, "https://x.com/a");
        assert_eq!(one("Wrote /home/me/a.md.").3, "/home/me/a.md");
        assert_eq!(one("(/home/me/a.md)").3, "/home/me/a.md");
        assert_eq!(one("/home/me/a.md, /x").3, "/home/me/a.md");
    }

    #[test]
    fn markdown_links_are_found_once() {
        let t = "[the PR](https://github.com/o/r/pull/1)";
        let (s, e, k, _) = one(t);
        assert_eq!((s, e, k), (9, t.len() - 1, "pr"), "the link's address");
        let t = "[https://x.com](https://x.com)";
        assert_eq!(one(t).0, 16, "a label that is a URL is not found again");
        assert_eq!(one("<https://x.com/a>").0, 1, "an autolink");
        assert_eq!(one("[plan](/home/me/plan.md)").2, "doc");
        assert!(detect("![shot](/home/me/a.png)").is_empty(), "a picture is not a link");
    }

    #[test]
    fn utf16_offsets_around_emoji() {
        let t = "🎉 https://x.com 👍";
        assert_eq!(one(t).0..one(t).1, 3..16);
        let t = "👨‍👩‍👧 /tmp/a.md";
        let s = u16len("👨‍👩‍👧 ");
        assert_eq!(one(t).0..one(t).1, s..s + 9);
    }

    #[test]
    fn code() {
        assert_eq!(one("Open \x60/home/me/a.md\x60 now").0..one("Open \x60/home/me/a.md\x60 now").1, 6..19, "a code span that is only the path");
        assert!(detect("Run \x60curl https://x.com\x60").is_empty(), "inside a command");
        assert!(detect("\x60\x60\x60sh\nopen https://x.com /tmp/a.md\n\x60\x60\x60").is_empty(), "a code block");
        assert_eq!(kinds("\x60\x60\x60\nx\n\x60\x60\x60\nthen https://x.com"), vec!["link"], "after the block");
    }

    #[test]
    fn reference_definitions_are_not_found() {
        assert!(detect("See [the docs][1].\n\n[1]: https://x.com/docs").is_empty(), "rewriting it would break [text][1]");
        assert!(detect("   [a]: <https://x.com> \"t\"").is_empty());
        assert_eq!(kinds("[1]: https://x.com\nthen https://y.com"), vec!["link"], "the next line is searched");
    }

    #[test]
    fn reference_links_are_left_alone() {
        assert!(detect("[1]:\n  https://x.com/docs").is_empty(), "skipped to be safe: the line after an empty definition is never searched (it may be text; a missed link, the text as it is)");
        assert!(detect("See [https://x.com][ref].\n\n[ref]: https://y.com").is_empty(), "a full reference");
        assert!(detect("See [https://x.com][].\n\n[https://x.com]: https://y.com").is_empty(), "a collapsed reference");
        assert_eq!(kinds("See [https://x.com][draft]."), vec!["link"], "without a definition it is text (CommonMark), not a reference");
        assert!(detect("See [https://x.com].\n\n[https://x.com]: https://y.com").is_empty(), "a shortcut reference");
        assert_eq!(kinds("See [https://x.com] there"), vec!["link"], "brackets without a definition are text");
    }

    #[test]
    fn indented_code_is_code() {
        assert!(detect("Run:\n\n    curl https://x.com\n    open /tmp/a.md").is_empty());
        assert!(detect("    https://x.com").is_empty(), "at the start");
        assert!(detect("\thttps://x.com").is_empty(), "a tab");
        assert_eq!(kinds("Text\n    https://x.com"), vec!["link"], "a paragraph's next line is not code");
        assert_eq!(kinds("- item\n\n    https://x.com"), vec!["link"], "a list item's paragraph, not code (CommonMark)");
        assert_eq!(kinds("    code\n\nafter https://x.com"), vec!["link"]);
    }

    #[test]
    fn a_trailing_underscore_is_part_of_the_url() {
        assert_eq!(one("https://x.com/a_").3, "https://x.com/a_");
        assert_eq!(one("see https://x.com/snake_case_ now").3, "https://x.com/snake_case_");
    }

    #[test]
    fn three_backticks_with_more_on_the_line_are_inline_code() {
        // As blocks.js: a backtick fence's info has no backtick.
        assert_eq!(kinds("```x``` then https://x.com"), vec!["link"]);
        assert!(detect("```sh\nhttps://x.com\n```").is_empty(), "still a fence");
        assert!(detect("~~~ a`b\nhttps://x.com\n~~~").is_empty(), "a tilde fence's info may have one");
    }

    #[test]
    fn quoted_definitions_and_nested_labels_are_skipped() {
        // Skipped to be safe: rewriting a URL there could break the reference.
        assert!(detect("> [1]: https://x.com/docs").is_empty(), "a definition in a quote");
        assert!(detect("> > [1]:\n> https://x.com/docs").is_empty(), "nested quote, address on the next line");
        assert!(detect("See [a [https://x.com] b][ref].\n\n[ref]: https://y.com").is_empty(), "nested brackets in a full reference");
        let t = "See [a [b] https://x.com](https://y.com).";
        let f = detect(t);
        assert_eq!((f.len(), f.first().map(|x| x.target.as_str())), (1, Some("https://y.com")), "nested brackets: the address only, not the label: {f:?}");
        assert_eq!(kinds("> quote https://x.com"), vec!["link"], "other quoted text is searched");
    }

    #[test]
    fn indented_lines_after_a_blank_or_a_heading_are_skipped() {
        // Skipped to be safe: code in a list item, or right after a heading.
        assert!(detect("- item\n\n      curl https://x.com").is_empty(), "code in a list item");
        assert!(detect("# Title\n    https://x.com").is_empty(), "right after a heading");
        assert!(detect("1. step\n\n   - sub\n\n         https://x.com").is_empty(), "a nested list");
    }

    // Codex round 3 (t122): each one rewrote a link or code before the parser.
    #[test]
    fn an_escaped_bracket_in_a_reference_text_is_left_alone() {
        let t = "[https://example.com/guide Guide \\[draft][ref]\n\n[ref]: https://y.com";
        assert!(detect(t).is_empty(), "{:?}", detect(t));
    }

    #[test]
    fn a_definition_in_a_list_item_is_left_alone() {
        let t = "- [ref]: https://example.com/guide\n\n[Guide][ref]";
        assert!(detect(t).is_empty(), "{:?}", detect(t));
    }

    #[test]
    fn indented_code_right_after_a_fence_is_code() {
        let t = "\x60\x60\x60\ncode\n\x60\x60\x60\n    curl https://example.com/guide";
        assert!(detect(t).is_empty(), "{:?}", detect(t));
    }

    #[test]
    fn a_url_that_runs_into_an_entity_or_an_escape_is_not_found() {
        // Its end is not sure: "&amp;" reads as "&", "\\_" as "_".
        assert!(detect("see https://x.com/?a=1&amp;b=2 now").is_empty(), "{:?}", detect("see https://x.com/?a=1&amp;b=2 now"));
        assert!(detect("see https://x.com/a\\_b now").is_empty());
        assert_eq!(one("see https://x.com/?a=1&b=2 now").3, "https://x.com/?a=1&b=2", "a plain & is text");
        assert!(detect("<a href=\"https://x.com\">x</a>").is_empty(), "HTML");
        assert!(detect("<div>\nhttps://x.com\n</div>").is_empty(), "an HTML block");
    }

    #[test]
    fn text_after_inline_html_is_not_searched() {
        // An HTML tag may open code or a link ("<code>", "<a href>"): the rest of its block is skipped, to be safe.
        assert!(detect("see <code>curl https://x.com</code> now").is_empty(), "{:?}", detect("see <code>curl https://x.com</code> now"));
        assert!(detect("<a href=\"https://y.com\">https://x.com</a>").is_empty());
        assert_eq!(kinds("see https://x.com <b>bold</b>"), vec!["link"], "before the tag");
        assert_eq!(kinds("a <b>x</b>\n\nnext https://x.com"), vec!["link"], "the next block");
    }

    #[test]
    fn not_collected() {
        for t in ["docs/a.md", "./a.md", "../a.md", "/usr/bin/env", "a/b/c.md", "//comment.md", "https://", "mail@x.com", "/tmp/a.md5", "/tmp/a.mdx", "/tmp/a.md/b"] {
            assert!(detect(t).is_empty(), "{t}: {:?}", detect(t));
        }
        assert_eq!(kinds("https://x.com/a/b.png"), vec!["link"], "a picture URL is a link, not a doc too");
    }

    #[test]
    fn several_in_order_without_overlap() {
        let f = detect("a https://a.com b /tmp/x.md c https://github.com/o/r/issues/2 d localhost:8080");
        assert_eq!(f.iter().map(|x| x.kind).collect::<Vec<_>>(), vec!["link", "doc", "issue", "local"]);
        assert!(f.windows(2).all(|w| w[0].end <= w[1].start));
    }

    #[test]
    fn a_url_cut_by_the_text_cap_is_not_a_link() {
        use herdr_core::*;
        let long = format!("{} https://x.com/abcdef", "a".repeat(crate::rows::TEXT_CAP - 12));
        let r = crate::rows::row(&ConversationItem { id: "a1".into(), kind: ItemKind::AssistantText(long), ..Default::default() });
        assert!(r.text.as_deref().unwrap().ends_with('…'));
        assert_eq!(r.links, None, "{:?}", r.links);
    }

    #[test]
    fn the_cut_is_found_by_its_place_not_by_lengths() {
        use herdr_core::*;
        let cap = crate::rows::TEXT_CAP;
        let row = |s: String| crate::rows::row(&ConversationItem { id: "a1".into(), kind: ItemKind::AssistantText(s), ..Default::default() });
        // Cut right after "a.": the URL in the text ends before the "." but went on ("a.html").
        let u = "https://x.com/a.";
        let r = row(format!("{} {u}html more", "a".repeat(cap - 1 - u.len())));
        assert!(r.text.as_deref().unwrap().ends_with("a.…"));
        assert_eq!(r.links, None, "cut after the dot");
        // The cut takes off exactly 3 bytes and adds the 3-byte "…": the lengths are the same.
        let u = "https://x.com/abc";
        let s = format!("{} {u}def", "a".repeat(cap - 1 - u.len()));
        let r = row(s.clone());
        assert_eq!(r.text.as_deref().unwrap().len(), s.len());
        assert_eq!(r.links, None, "same length after the cut");
        // A URL that ends before the cut is kept.
        let r = row(format!("https://x.com/ok {}", "a".repeat(cap)));
        assert_eq!(r.links, Some(vec![FeedLink { start: 0, end: 16, res: "link" }]));
    }

    #[test]
    fn assistant_rows_carry_links_others_do_not() {
        use herdr_core::*;
        let item = |kind| ConversationItem { id: "a1".into(), kind, ..Default::default() };
        let r = crate::rows::row(&item(ItemKind::AssistantText("PR: https://github.com/o/r/pull/9 ok".into())));
        assert_eq!(r.links, Some(vec![FeedLink { start: 4, end: 33, res: "pr" }]));
        let json = serde_json::to_value(&r).unwrap().to_string();
        assert!(json.contains(r#""links":[{"end":33,"res":"pr","start":4}]"#), "{json}");
        let r = crate::rows::row(&item(ItemKind::AssistantText("nothing here".into())));
        assert_eq!(r.links, None);
        assert!(!serde_json::to_value(&r).unwrap().to_string().contains("links"), "no field without links");
        let r = crate::rows::row(&item(ItemKind::UserText("see https://x.com".into())));
        assert_eq!(r.links, None, "user text is not searched");
    }

    #[test]
    fn links_for_a_row() {
        assert_eq!(links("no links here"), vec![]);
        assert_eq!(links("x https://x.com"), vec![FeedLink { start: 2, end: 15, res: "link" }]);
    }
}
