//! Resources: what an agent points the human to in its reply (contract: herdr-app/docs/protocol.md, `row`, `links`).
//! Pure. `detect` finds the targets in an assistant text; feed rows carry them as `links`, and the `resources`
//! command (P1) collects them with the same rules.

use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};
use herdr_core::{ConversationItem, ItemKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

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
    scan(text, false)
}

/// `detect`, and with `rel` also relative doc paths (`docs/a.md`, `./a.md`): what Resources collects. Feed rows
/// do not link them (the mod could not open them).
fn scan(text: &str, rel: bool) -> Vec<Found> {
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
                        if let Some((kind, target)) = whole_rel(&text[ds..de], rel) {
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
                    if let Some((kind, target)) = whole_rel(tok, rel) {
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
    if rel {
        bare_rel(text, &mut taken, &mut hits);
    }
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

/// `whole`, and with `rel` a relative doc path too.
fn whole_rel(tok: &str, rel: bool) -> Option<(&'static str, String)> {
    whole(tok).or_else(|| (rel && is_rel_doc(tok)).then(|| ("doc", tok.to_string())))
}

/// A relative path to a doc: a folder and a file (`docs/a.md`, `./a.md`, `../a.md`, `.claude/a.md`), the file a doc
/// kind. Not: a bare file name (`README.md`), an address without its scheme (`x.com/a.png`: a first part with a dot),
/// a pattern or a placeholder (`coord/*.md`, `contracts/<id>.md`).
fn is_rel_doc(tok: &str) -> bool {
    let Some((first, _)) = tok.split_once('/') else { return false };
    let first_ok = first == "." || first == ".." || first.starts_with('.') || (!first.is_empty() && !first.contains('.'));
    first_ok && !tok.starts_with('~') && !tok.contains("//") && !tok.contains(['*', '?', '<', '>', '{', '}', '$', '[', ']', '|']) && !tok.contains(':') && !tok.contains(char::is_whitespace) && doc_end(tok, 0, tok.len()) == Some(tok.len())
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

/// Relative doc paths in plain text (see `is_rel_doc`), after URLs and absolute paths.
fn bare_rel(text: &str, taken: &mut [bool], hits: &mut Vec<(usize, usize, &'static str, String)>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if taken[i] || !(b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') || !starts_here(text, i) {
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
            Some(e) if is_rel_doc(&text[i..e]) && !taken[i..e].iter().any(|&x| x) => {
                hits.push((i, e, "doc", text[i..e].to_string()));
                mark(taken, i, e, true);
                i = e;
            }
            _ => i = j.max(i + 1),
        }
    }
}

/// A one-line shell command the reply gives the human to run (`cmd`): where its block starts (UTF-16), what the
/// reply says right before it, and the command as written.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub start: usize,
    pub title: String,
    pub cmd: String,
}

/// A fenced block's language that holds shell commands (the info's first word, any case).
const CMD_LANGS: [&str; 6] = ["", "sh", "bash", "zsh", "shell", "console"];
/// A longer command is not one to keep.
const CMD_MAX: usize = 400;
const TITLE_MAX: usize = 80;
const HEAD_MAX: usize = 60;

/// The commands in `text`, in order: fenced code blocks in a shell language (`CMD_LANGS`) that hold one command, one
/// line or lines joined by a `\` at their end, at most `CMD_MAX` characters; a leading `$ ` is dropped. Not commands:
/// a block with more than one (a script, or a command and its output), other languages (text, log, json…), indented
/// code, inline code, a comment. Title: the last line of the paragraph or heading right before the block (its
/// ending `:` or `：` dropped, cut at `TITLE_MAX`); with none there, the command's start.
pub fn commands(text: &str) -> Vec<Command> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut out = vec![];
    // The text of the block right before (a paragraph or a heading), and of the one being read.
    let (mut before, mut cur): (Option<String>, Option<String>) = (None, None);
    // The fenced block being read: its start (bytes), language, body.
    let mut code: Option<(usize, String, String)> = None;
    for (ev, r) in Parser::new_ext(text, opts).into_offset_iter() {
        match ev {
            Event::Start(Tag::Paragraph | Tag::Heading { .. }) => cur = Some(String::new()),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_)) => before = cur.take(),
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                code = Some((r.start, info.split_whitespace().next().unwrap_or("").to_ascii_lowercase(), String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((start, lang, body)) = code.take() {
                    if let Some(cmd) = CMD_LANGS.contains(&lang.as_str()).then(|| one_command(&body)).flatten() {
                        let title = before.as_deref().map(title_of).filter(|t| !t.is_empty()).unwrap_or_else(|| cut(&one_line(&cmd), HEAD_MAX));
                        out.push(Command { start: text[..start].encode_utf16().count(), title, cmd });
                    }
                }
                before = None;
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some(c) = code.as_mut() {
                    c.2.push_str(&t);
                } else if let Some(c) = cur.as_mut() {
                    c.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(c) = cur.as_mut() {
                    c.push('\n');
                }
            }
            // Inside a paragraph: part of its text.
            Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. })
            | Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image) => {}
            // Another block between: the text before is not the command's.
            Event::Start(_) | Event::End(_) | Event::Rule if cur.is_none() => before = None,
            _ => {}
        }
    }
    out
}

/// A block's body as one command, or None.
fn one_command(body: &str) -> Option<String> {
    let mut lines: Vec<&str> = body.lines().collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let (last, rest) = lines.split_last()?;
    if last.trim_end().ends_with('\\') || rest.iter().any(|l| !l.trim_end().ends_with('\\')) {
        return None;
    }
    let all = lines.join("\n");
    let all = all.trim();
    let cmd = all.strip_prefix("$ ").map(str::trim_start).unwrap_or(all);
    (!cmd.is_empty() && !cmd.starts_with('#') && cmd.chars().count() <= CMD_MAX).then(|| cmd.to_string())
}

/// A command on one line: each line's ending `\` dropped, the spaces made one.
pub fn one_line(cmd: &str) -> String {
    cmd.lines().map(|l| l.trim().trim_end_matches('\\')).collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max - 1).collect::<String>().trim_end())
}

/// The title a paragraph gives the command after it: its last line, without the ending colon.
fn title_of(p: &str) -> String {
    let line = p.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    cut(line.trim_end_matches([':', '：', ' ']).trim(), TITLE_MAX)
}

pub fn links(text: &str) -> Vec<FeedLink> {
    detect(text).into_iter().map(|f| FeedLink { start: f.start, end: f.end, res: f.kind }).collect()
}


/// One thing the agent pointed the human to, as the `resources` command gives it (a `res` line).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Resource {
    #[serde(rename = "type")]
    pub t: &'static str,
    /// FNV-1a of the item's key (the normalized target): the same target is one item.
    pub id: String,
    pub kind: &'static str,
    /// What opens: an absolute path for a doc (`~/` expanded), else a URL.
    pub target: String,
    pub title: String,
    /// A doc's folder (the home as `~`); a URL's host, or "owner/repo" on GitHub.
    pub detail: String,
    /// The feed row (`row` line `id`) of the last mention.
    pub row: String,
    /// Milliseconds since 1970 of the first and the last mention; null when the log has no time.
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub mentions: usize,
    /// Docs only: the path is a file now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
}

/// The resources in `items` (a session log as parsed), newest first: by the last mention, and in one reply the
/// later target first. Collected: targets in assistant text (`detect`), and artifact URLs in the result of a
/// successful Artifact publish. Not collected: what the person typed, other tools' input and output.
/// `home`: the home `~/` stands for.
pub fn collect(items: &[ConversationItem], home: Option<&Path>) -> Vec<Resource> {
    let mut acc = Acc::default();
    acc.add(items, home);
    acc.finish(home)
}

/// The kinds a resource can be (`normal` gives one of them).
const KINDS: [&str; 8] = ["doc", "local", "cmd", "pr", "issue", "ci", "artifact", "link"];

fn kind_of(k: &str) -> Option<&'static str> {
    KINDS.iter().copied().find(|x| *x == k)
}

/// A title as given (a reply's text before a command, an Artifact call's `title`), or a published file whose own
/// title is read when the list is made (as `collect` reads it).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
enum Title {
    Given(String),
    File(String),
}

/// One target while the log is read: what `Resource` gets from its mentions so far.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Held {
    key: String,
    kind: String,
    target: String,
    row: String,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
    mentions: usize,
    /// The order of the last mention, over the whole log.
    seq: usize,
    /// The titles the mentions gave, the newest last: the newest that reads is the title. A given title makes the
    /// older ones no matter.
    titles: Vec<Title>,
}

/// The resources of a log read so far; `add` the items of more lines, `finish` makes the list. Items added in parts
/// give the same list as all at once when no run of tool calls is cut (`rows::merged` joins a run).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Acc {
    held: Vec<Held>,
    seq: usize,
    /// For relative doc paths (resolved in `finish`): the folders the log says the agent worked in (`cwd`), the
    /// folders its commands went to (`cd /…`), and the absolute doc paths its tool calls named.
    #[serde(default)]
    cwds: BTreeSet<String>,
    #[serde(default)]
    dirs: BTreeSet<String>,
    #[serde(default)]
    files: BTreeSet<String>,
    /// Key -> index in `held`; made again from `held` after a read from disk.
    #[serde(skip)]
    at: HashMap<String, usize>,
}

impl Acc {
    pub fn add(&mut self, items: &[ConversationItem], home: Option<&Path>) {
        if self.at.len() != self.held.len() {
            self.at = self.held.iter().enumerate().map(|(i, h)| (h.key.clone(), i)).collect();
        }
        for it in crate::rows::merged(items) {
            let ts = crate::rows::ms(it.timestamp);
            let mut found: Vec<(&'static str, String, Option<Title>)> = vec![];
            match &it.kind {
                ItemKind::AssistantText(s) => {
                    // In the order they are written: a later one is newer.
                    let mut f: Vec<(usize, &'static str, String, Option<Title>)> = scan(s, true).into_iter().map(|f| (f.start, f.kind, f.target, None)).collect();
                    f.extend(commands(s).into_iter().map(|c| (c.start, "cmd", c.cmd, Some(Title::Given(c.title)))));
                    f.sort_by_key(|x| x.0);
                    found.extend(f.into_iter().map(|(_, k, t, title)| (k, t, title)));
                }
                ItemKind::ToolChips(chips) => {
                    for c in chips.iter().flat_map(|c| &c.calls).filter(|c| !c.is_error && c.output.is_some()) {
                        self.places(&c.input, home);
                    }
                    for c in chips.iter().flat_map(|c| &c.calls) {
                        let (true, false, Some(out)) = (c.tool_name == "Artifact", c.is_error, c.output.as_deref()) else { continue };
                        let (action, title, file) = artifact_input(&c.input);
                        // No title given: the published page's own <title>, when the file is here.
                        let title = title.map(Title::Given).or_else(|| file.filter(|f| Path::new(f).is_absolute()).map(Title::File));
                        if action.as_deref().is_some_and(|a| a != "publish") {
                            continue;
                        }
                        found.extend(detect(out).into_iter().filter(|f| f.kind == "artifact").map(|f| (f.kind, f.target, title.clone())));
                    }
                }
                _ => {}
            }
            for (kind, target, title) in found {
                let (kind, key, target) = match kind == "doc" && is_rel_doc(&target) {
                    true => {
                        let r = rel_clean(&target);
                        ("doc", format!("rel:{r}"), r)
                    }
                    false => normal(kind, &target, home),
                };
                self.seq += 1;
                let k = match self.at.get(&key) {
                    Some(&k) => {
                        let h = &mut self.held[k];
                        h.mentions += 1;
                        h.last_ts = ts.or(h.last_ts);
                        h.first_ts = h.first_ts.or(ts);
                        h.row = it.id.clone();
                        h.target = target;
                        h.seq = self.seq;
                        k
                    }
                    None => {
                        self.at.insert(key.clone(), self.held.len());
                        let h = Held { key, kind: kind.into(), target, row: it.id.clone(), first_ts: ts, last_ts: ts, mentions: 1, seq: self.seq, titles: vec![] };
                        self.held.push(h);
                        self.held.len() - 1
                    }
                };
                let titles = &mut self.held[k].titles;
                match title {
                    Some(Title::Given(t)) => *titles = vec![Title::Given(t)],
                    Some(Title::File(p)) => {
                        titles.retain(|t| *t != Title::File(p.clone()));
                        titles.push(Title::File(p));
                    }
                    None => {}
                }
            }
        }
    }

    /// The folders and doc paths a tool call that worked names (see `cwds`): the doc path of a file tool
    /// (`file_path`, `path`, `notebook_path`) and the folders of a command's `cd /…` (`command`, `cmd`). Not any other
    /// string: a command's text or a description names paths it did not open.
    fn places(&mut self, input: &str, home: Option<&Path>) {
        let Ok(serde_json::Value::Object(o)) = serde_json::from_str::<serde_json::Value>(input) else { return };
        for k in ["file_path", "path", "notebook_path"] {
            if let Some(("doc", t)) = o.get(k).and_then(|v| v.as_str()).and_then(|s| whole(s.trim())) {
                self.files.insert(normal("doc", &t, home).2);
            }
        }
        for k in ["command", "cmd"] {
            let cmds: Vec<&str> = match o.get(k) {
                Some(serde_json::Value::String(s)) => vec![s],
                Some(serde_json::Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect(),
                _ => vec![],
            };
            for d in cmds.into_iter().flat_map(cd_targets) {
                self.dirs.insert(normal("doc", &d, home).2.trim_end_matches('/').to_string());
            }
        }
    }

    /// The working folders named by log lines (`cwd`), in order.
    pub fn cwd(&mut self, cwd: &str) {
        self.cwds.insert(cwd.to_string());
    }

    /// Where a relative doc path opens (its real path): under a working folder (`cwd`, `agent`: the pane's) when
    /// exactly one such file is there (more than one: none, the other clues do not pick); with none there, under a
    /// folder a command went to, or a doc path a file tool named that ends in it, when exactly one is. None: not listed.
    /// It does not open, and it may be an example (`docs/a.md`); once its file is there, the next read finds it.
    /// The path is looked up as written: the file system takes `..` after a link, as it opens it.
    fn resolve(&self, rel: &str, agent: Option<&str>) -> Option<String> {
        let one = |c: BTreeSet<String>| (c.len() == 1).then(|| c.into_iter().next()).flatten();
        let found = |b: &str| -> Option<String> {
            let p = format!("{}/{rel}", b.trim_end_matches('/'));
            Path::new(&p).is_file().then(|| real_doc(&p))
        };
        let here: BTreeSet<String> = self.cwds.iter().map(String::as_str).chain(agent).filter_map(found).collect();
        if !here.is_empty() {
            return one(here);
        }
        if rel.split('/').any(|x| x == "..") {
            return None;
        }
        let end = format!("/{rel}");
        let there: BTreeSet<String> = self.dirs.iter().filter_map(|d| found(d)).chain(self.files.iter().filter(|f| f.ends_with(&end)).map(|f| real_doc(f))).collect();
        one(there)
    }

    /// Each relative doc path and the key it has now (see `resolve`): when one changes, a list merged by id would
    /// keep the old item.
    fn resolved(&self, agent: Option<&str>) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.held.iter().filter_map(|h| h.key.strip_prefix("rel:")).map(|r| (r.to_string(), self.resolve(r, agent).map(|p| format!("doc:{p}")).unwrap_or_default())).collect();
        out.sort();
        out
    }

    /// The list, newest first (see `collect`).
    pub fn finish(&self, home: Option<&Path>) -> Vec<Resource> {
        self.finish_in(home, None)
    }

    /// `finish`, with the agent's folder for relative doc paths. A relative path found is the item of its file (one
    /// item with the absolute mentions of it, its id the absolute path's: mentions added, the last mention's row); one
    /// not found is not listed.
    pub fn finish_in(&self, home: Option<&Path>, agent: Option<&str>) -> Vec<Resource> {
        let keys: HashMap<String, String> = self.resolved(agent).into_iter().map(|(r, k)| (format!("rel:{r}"), k)).collect();
        let mut merged: Vec<Held> = vec![];
        let mut at: HashMap<String, usize> = HashMap::new();
        for h in &self.held {
            let mut h = h.clone();
            if let Some(k) = keys.get(&h.key) {
                // Not found: not listed.
                if k.is_empty() {
                    continue;
                }
                h.target = k.strip_prefix("doc:").unwrap_or(k).to_string();
                h.key = k.clone();
            } else if let Some(p) = h.key.strip_prefix("doc:") {
                // One file written two ways (a linked folder, another case): one item, named by its real path.
                let r = real_doc(p);
                if r != p {
                    (h.key, h.target) = (format!("doc:{r}"), r);
                }
            }
            match at.get(&h.key) {
                Some(&i) => {
                    let m = &mut merged[i];
                    m.mentions += h.mentions;
                    m.first_ts = match (m.first_ts, h.first_ts) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    if h.seq > m.seq {
                        (m.seq, m.row, m.target, m.last_ts) = (h.seq, h.row, h.target, h.last_ts.or(m.last_ts));
                    }
                }
                None => {
                    at.insert(h.key.clone(), merged.len());
                    merged.push(h);
                }
            }
        }
        let mut held: Vec<&Held> = merged.iter().collect();
        held.sort_by_key(|h| std::cmp::Reverse((h.last_ts.unwrap_or(i64::MIN), h.seq)));
        held.into_iter()
            .map(|h| {
                let title = h.titles.iter().rev().find_map(|t| match t {
                    Title::Given(t) => Some(t.clone()),
                    Title::File(p) => openable(Path::new(p)).ok().and_then(|p| file_title(&p)),
                });
                let r = Resource {
                    t: "res",
                    id: crate::protocol::fnv1a(&h.key),
                    kind: kind_of(&h.kind).unwrap_or("link"),
                    target: h.target.clone(),
                    title: title.unwrap_or_default(),
                    detail: String::new(),
                    row: h.row.clone(),
                    first_ts: h.first_ts,
                    last_ts: h.last_ts,
                    mentions: h.mentions,
                    exists: None,
                };
                described(r, home)
            })
            .collect()
    }

    fn known(&self) -> bool {
        self.held.iter().all(|h| kind_of(&h.kind).is_some())
    }
}

/// The `action`, `title` and `file_path` of an Artifact call's input (JSON, maybe cut at 4 KB: then read as text).
fn artifact_input(input: &str) -> (Option<String>, Option<String>, Option<String>) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(input) {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
        return (s("action"), s("title").filter(|t| !t.trim().is_empty()).map(|t| t.trim().to_string()), s("file_path"));
    }
    let field = |k: &str| -> Option<String> {
        let p = input.find(&format!("\"{k}\""))? + k.len() + 2;
        let rest = input[p..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
        let mut out = String::new();
        let mut esc = false;
        for c in rest.chars() {
            match (esc, c) {
                (true, 'n') => out.push(' '),
                (true, c) => out.push(c),
                (false, '\\') => {
                    esc = true;
                    continue;
                }
                (false, '"') => return Some(out),
                (false, c) => out.push(c),
            }
            esc = false;
        }
        None
    };
    (field("action"), field("title").filter(|t| !t.trim().is_empty()).map(|t| t.trim().to_string()), field("file_path"))
}

/// A URL's parts: scheme (lowercase), authority without the user (host lowercase), path, query (with "?"), fragment.
fn parts(url: &str) -> (String, String, String, String) {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let rest = rest.split('#').next().unwrap_or(rest);
    let a = rest.find(['/', '?']).unwrap_or(rest.len());
    let auth = &rest[..a];
    let auth = auth.rsplit('@').next().unwrap_or(auth).to_ascii_lowercase();
    let tail = &rest[a..];
    let (path, query) = match tail.find('?') {
        Some(q) => (&tail[..q], &tail[q..]),
        None => (tail, ""),
    };
    (scheme.to_ascii_lowercase(), auth, path.to_string(), query.to_string())
}

/// The item's key and the target it opens. Docs: the absolute path. Local servers: one per host:port (the newest URL
/// opens). GitHub PRs, issues and runs: their own page. Other URLs: host lowercase, without "#…" and a trailing "/".
fn normal(kind: &'static str, target: &str, home: Option<&Path>) -> (&'static str, String, String) {
    match kind {
        "doc" => {
            let p = match (target.strip_prefix("~/"), home) {
                (Some(rest), Some(h)) => h.join(rest).to_string_lossy().to_string(),
                _ => target.to_string(),
            };
            ("doc", format!("doc:{p}"), p)
        }
        "local" => {
            let (scheme, auth, _, _) = parts(target);
            ("local", format!("local:{}", host_port(&scheme, &auth)), target.to_string())
        }
        "cmd" => ("cmd", format!("cmd:{}", target.trim()), target.trim().to_string()),
        "pr" | "issue" | "ci" => {
            let (_, _, path, _) = parts(target);
            let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            // url_kind reads the path its own way: a URL it calls a PR whose path here is too short is a link.
            if seg.len() < if kind == "ci" { 5 } else { 4 } {
                return normal("link", target, home);
            }
            let page = if kind == "ci" { format!("actions/runs/{}", seg[4]) } else { format!("{}/{}", seg[2], seg[3]) };
            let u = format!("https://github.com/{}/{}/{page}", seg[0], seg[1]);
            (kind, u.clone(), u)
        }
        _ => {
            let (scheme, auth, path, query) = parts(target);
            let key = format!("{scheme}://{auth}{path}{query}");
            (kind, key.trim_end_matches('/').to_string(), target.to_string())
        }
    }
}

/// A relative path as one item: without leading "./".
fn rel_clean(p: &str) -> String {
    let mut p = p;
    while let Some(r) = p.strip_prefix("./") {
        p = r;
    }
    p.to_string()
}

/// A doc path as the file system names it: its folder's real path (links followed, the case on disk) and the file's
/// name, or the whole real path when the file itself is no link (a link to a file keeps its own name: `exists` and
/// `open-check` look at what it opens). The path as written when there is no such folder.
fn real_doc(p: &str) -> String {
    let path = Path::new(p);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return p.to_string() };
    let Ok(rdir) = std::fs::canonicalize(dir) else { return p.to_string() };
    let is_link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    match std::fs::canonicalize(path) {
        Ok(r) if !is_link => r.to_string_lossy().into_owned(),
        _ => rdir.join(name).to_string_lossy().into_owned(),
    }
}

/// The absolute folders a shell command goes to: `cd /…` or `cd ~/…` at its start or after `&&`, `;`, `||`,
/// `|`, `(` or a line break; quotes taken off.
fn cd_targets(s: &str) -> Vec<String> {
    let mut out = vec![];
    for (i, _) in s.match_indices("cd ") {
        let before = s[..i].trim_end_matches([' ', '\t']);
        if !(before.is_empty() || before.ends_with("&&") || before.ends_with(';') || before.ends_with('|') || before.ends_with('(') || before.ends_with('\n')) {
            continue;
        }
        let rest = s[i + 3..].trim_start();
        let (q, rest) = match rest.as_bytes().first() {
            Some(&c @ (b'"' | b'\'')) => (Some(c as char), &rest[1..]),
            _ => (None, rest),
        };
        let end = match q {
            Some(q) => rest.find(q).unwrap_or(rest.len()),
            None => rest.find(|c: char| c.is_whitespace() || ";&|)".contains(c)).unwrap_or(rest.len()),
        };
        let d = &rest[..end];
        if (d.starts_with('/') || d.starts_with("~/")) && !d.contains('$') {
            out.push(d.to_string());
        }
    }
    out
}

/// "host:port", the port the scheme's when none is written.
fn host_port(scheme: &str, auth: &str) -> String {
    let has_port = match auth.rfind(':') {
        Some(k) => !auth[k..].contains(']'),
        None => false,
    };
    if has_port {
        auth.to_string()
    } else {
        format!("{auth}:{}", if scheme == "https" { 443 } else { 80 })
    }
}

/// The home as "~" at the start of a path.
fn tilde(p: &str, home: Option<&Path>) -> String {
    if let Some(h) = home.map(|h| h.to_string_lossy().trim_end_matches('/').to_string()).filter(|h| !h.is_empty()) {
        if p == h {
            return "~".into();
        }
        if let Some(rest) = p.strip_prefix(&format!("{h}/")) {
            return format!("~/{rest}");
        }
    }
    p.to_string()
}

/// The real file a doc path opens (links followed), when it is a file of a kind in DOC_EXT; else why not. The
/// extension that counts is the real file's: a `preview.md` link to `notes.txt` does not open.
pub fn openable(p: &Path) -> Result<PathBuf, &'static str> {
    let real = std::fs::canonicalize(p).map_err(|_| "File not found.")?;
    if !std::fs::metadata(&real).map(|m| m.is_file()).unwrap_or(false) {
        return Err("Not a file.");
    }
    let ext = real.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if !DOC_EXT.contains(&ext.as_str()) {
        return Err("Flockbook does not open this kind of file.");
    }
    Ok(real)
}

/// Title, detail and (docs) exists.
fn described(mut r: Resource, home: Option<&Path>) -> Resource {
    match r.kind {
        "doc" => {
            let p = Path::new(&r.target);
            let real = openable(p).ok();
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| r.target.clone());
            r.title = real.as_deref().and_then(file_title).unwrap_or(name);
            r.detail = p.parent().map(|d| tilde(&d.to_string_lossy(), home)).unwrap_or_default();
            r.exists = Some(real.is_some());
        }
        "pr" | "issue" | "ci" => {
            let seg: Vec<&str> = r.target.trim_start_matches("https://github.com/").split('/').collect();
            r.detail = format!("{}/{}", seg[0], seg[1]);
            let n = seg.last().copied().unwrap_or("");
            r.title = if r.kind == "ci" { format!("CI run {n}") } else { format!("{}#{n}", r.detail) };
        }
        "cmd" => r.detail = one_line(&r.target),
        "local" => {
            let (scheme, auth, _, _) = parts(&r.target);
            r.title = if auth.contains(':') { auth.clone() } else { host_port(&scheme, &auth) };
            r.detail = auth.rsplit_once(':').map(|x| x.0.to_string()).unwrap_or(auth);
        }
        _ => {
            let (_, auth, path, _) = parts(&r.target);
            if r.title.is_empty() {
                r.title = if r.kind == "artifact" { "Artifact".into() } else { format!("{auth}{}", path.trim_end_matches('/')) };
            }
            r.detail = auth;
        }
    }
    r
}

/// A markdown file's first "# " heading, or an HTML file's <title>, in its first 4 KB.
fn file_title(p: &Path) -> Option<String> {
    let ext = p.extension()?.to_string_lossy().to_ascii_lowercase();
    let mut buf = vec![0u8; 4096];
    let n = std::fs::File::open(p).and_then(|mut f| f.read(&mut buf)).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]).to_string();
    let t = match ext.as_str() {
        "md" => head.lines().find_map(|l| l.strip_prefix("# ")).map(|t| t.trim().to_string()),
        "html" | "htm" => {
            let low = head.to_ascii_lowercase();
            let s = low.find("<title")?;
            let s = s + low[s..].find('>')? + 1;
            let e = s + low[s..].find("</title")?;
            Some(head[s..e].split_whitespace().collect::<Vec<_>>().join(" "))
        }
        _ => None,
    };
    t.filter(|t| !t.is_empty())
}

/// Bump when what is collected changes (`detect`, `commands`, `normal`, `Acc`): a state kept with another number is
/// not used.
pub const RULES: u32 = 2;

/// What one log's last `resources` call keeps, so the next reads only from `at`: the resources of the lines before
/// it. `at` is a line start where no tool call waits for its result and no run of tool calls goes on (see
/// `answer_kept`). Used only for the same log, home and rules, when the log is not shorter and its bytes before `at`
/// look the same (a hash of its first 4 KB and of the 4 KB before `at`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Kept {
    pub rules: u32,
    pub home: Option<String>,
    pub path: String,
    /// The log's size when read.
    pub size: u64,
    pub at: u64,
    head: String,
    near: String,
    /// The file's `identity` when kept (a state of another file at the path is not used).
    #[serde(default)]
    file: String,
    acc: Acc,
}

/// Writes `k` to `at` (one file per terminal), and removes the folder's state files untouched for 7 days (terminals
/// gone) and all but the newest `KEPT`. Only files named as `state_name` names them are removed; in a folder that is a
/// link nothing is written.
pub fn keep(k: &Kept, at: &Path) {
    if at.parent().is_some_and(|d| std::fs::symlink_metadata(d).is_ok_and(|m| !m.is_dir())) {
        return;
    }
    crate::locate::write_json(k, Some(at));
    let week = std::time::Duration::from_secs(7 * 24 * 3600);
    let hour = std::time::Duration::from_secs(3600);
    let mut states = vec![];
    for e in at.parent().and_then(|d| std::fs::read_dir(d).ok()).into_iter().flatten().flatten() {
        let p = e.path();
        let age = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).unwrap_or_default();
        let name = e.file_name().to_string_lossy().into_owned();
        if state_name(&name, true) {
            // A temp file a stopped write left (another call may be writing one now: only old ones go).
            if age > hour {
                let _ = std::fs::remove_file(&p);
            }
        } else if state_name(&name, false) && e.file_type().is_ok_and(|t| t.is_file()) {
            if age > week {
                let _ = std::fs::remove_file(&p);
            } else if p != at {
                states.push((age, p));
            }
        }
    }
    // At most KEPT files: the one just written and the newest others.
    states.sort();
    for (_, p) in states.into_iter().skip(KEPT - 1) {
        let _ = std::fs::remove_file(p);
    }
}

/// How many terminals keep a state (one file each, a few KB to a few hundred KB).
const KEPT: usize = 64;

/// What the bytes before `at` look like: a hash of the first 4 KB, and a hash of 16 pieces of 1 KB spread over them
/// and the 4 KB before `at`. An agent only adds lines to its log; this finds another file at the path, not every
/// change inside an old line.
fn print(log: &Path, at: u64) -> std::io::Result<(String, String)> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(log)?;
    let mut span = |from: u64, to: u64| -> std::io::Result<String> {
        f.seek(SeekFrom::Start(from))?;
        let mut d = vec![];
        (&mut f).take(to.saturating_sub(from)).read_to_end(&mut d)?;
        Ok(String::from_utf8_lossy(&d).into_owned())
    };
    let head = span(0, at.min(4096))?;
    let mut rest = String::new();
    for i in 0..16 {
        let s = at / 16 * i;
        rest.push_str(&span(s, (s + 1024).min(at))?);
    }
    rest.push_str(&span(at.saturating_sub(4096), at)?);
    Ok((crate::protocol::fnv1a(&head), crate::protocol::fnv1a(&rest)))
}

/// The tool calls of a run join one item: a next read starts only at a line whose first item is text (the person's or
/// the agent's), never a call or a call that waits (`PermissionRequest`).
fn starts_clean(i: Option<&ConversationItem>) -> bool {
    i.is_some_and(|i| matches!(i.kind, ItemKind::UserText(_) | ItemKind::AssistantText(_)))
}

/// The items of the lines in `from..to`, parsed on their own, and how many results there are for calls before `from`.
fn items_in(log: &Path, from: u64, to: u64) -> std::io::Result<(Vec<ConversationItem>, usize)> {
    let mut p = herdr_core::ConversationParser::new();
    crate::tail::each_line(log, from, to, |l, at| p.feed_at(l, Some(at)))?;
    Ok((crate::tail::without_pictures(p.items()), p.late_results()))
}

/// The working folder a log line names: Claude Code's `"cwd":"…"`, Codex's in `session_meta` and `turn_context`
/// (the first one in the line; a value with an escape is skipped).
fn line_cwd(l: &str) -> Option<String> {
    let i = l.find("\"cwd\":\"")? + 7;
    let rest = &l[i..];
    let e = rest.find('"')?;
    let c = &rest[..e];
    (c.starts_with('/') && !c.contains('\\')).then(|| c.trim_end_matches('/').to_string()).filter(|c| !c.is_empty())
}

/// The cursor's mark of the log's bytes before `size` (see `print`): another file at the path does not pass.
fn mark_of(log: &Path, size: u64) -> Option<String> {
    let id = identity(log)?;
    print(log, size).ok().map(|(h, n)| crate::protocol::fnv1a(&format!("{RULES}.{id}.{h}{n}")))
}

/// The file itself: its device, inode and birth time (where the file system has one). A log written again through a
/// new file (renamed over the old one, or made again) is another file, whatever its bytes. Not the change time: an
/// append changes it, and every read would then be whole.
fn identity(log: &Path) -> Option<String> {
    let m = std::fs::metadata(log).ok()?;
    let born = m.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!("{}.{}.{}", m.dev(), m.ino(), born.unwrap_or(0)))
    }
    #[cfg(not(unix))]
    {
        Some(format!("{}", born.unwrap_or(0)))
    }
}

/// The state folder beside `cache`, or None when it is a link or not a folder: nothing is read, written or removed
/// through a link.
pub fn state_dir(cache: &Path) -> Option<PathBuf> {
    let d = cache.with_file_name("resources");
    match std::fs::symlink_metadata(&d) {
        Ok(m) if !m.is_dir() => None,
        _ => Some(d),
    }
}

/// A name this command gives a state file: `<hex>.json` (FNV-1a of the terminal); with `temp`, the temp file a
/// write makes first (`.<hex>.json.<pid>`).
fn state_name(n: &str, temp: bool) -> bool {
    let hex = |s: &str| (1..=16).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
    match temp {
        false => n.strip_suffix(".json").is_some_and(hex),
        true => n.strip_prefix('.').and_then(|r| r.split_once(".json.")).is_some_and(|(h, pid)| hex(h) && !pid.is_empty() && pid.bytes().all(|c| c.is_ascii_digit())),
    }
}

/// How many line starts before the end `answer_kept` looks at for the next `at`, and how many of them it checks
/// by parsing to the end.
const LOOK: usize = 2000;
const CHECKS: usize = 4;

/// The answer to `resources`: the items to send, the next cursor, and whether they are all of them (`full`) or only
/// the ones newer than `since`.
pub struct Answer {
    pub items: Vec<Resource>,
    pub cursor: String,
    pub full: bool,
}

/// `answer_kept` with the agent's folder (the pane's), for relative doc paths.
pub fn answer_kept(log: &Path, since: Option<&str>, home: Option<&Path>, kept: Option<&Kept>, linear: bool) -> std::io::Result<(Answer, Option<Kept>, u64)> {
    answer_in(log, since, home, None, kept, linear)
}

/// The resources in the whole log (`answer_kept` with nothing kept).
pub fn answer(log: &Path, since: Option<&str>, home: Option<&Path>) -> std::io::Result<Answer> {
    answer_kept(log, since, home, None, false).map(|x| x.0)
}

/// The resources in the whole log; with `kept` from the call before (and a log that still fits it), only the lines
/// from its `at` are parsed. `linear`: the agent's log is read line after line with nothing later changing what an
/// earlier line gave (Claude Code, Codex, Cursor, Grok; not pi or agy, whose entries are a tree or slots): only then is a
/// state returned. Returns the answer, the state to keep, and the offset parsing started at.
///
/// The next `at` is a line start before which no tool call waits for its result, and whose lines parsed on their
/// own give the same items as the end of this read, the first not tool calls: the items before it are then final.
///
/// The cursor is "r4.<hash of the path>.<size>.<newest last_ts>.<mark>.<where relative docs open>" (`mark_of`: the
/// file and its bytes before `size`; the last part: a hash of `Acc::resolved`). An r1, r2 or r3 cursor (old Codex
/// ids; a log cut at 64 MB; no relative docs) gets all of it again. With a cursor of this log whose mark still
/// passes: the same size, no items; a bigger size, only the items mentioned at or after its newest one (a run of tool
/// calls has its first call's time, so a later call in it can have that same time; sent again, the mod merges by id).
/// Else (another file at the path, a shorter one, a bad cursor) all of them. All of them too when the kept state does
/// not fit the log, or a result comes for a call before the kept start: the log is then read whole.
pub fn answer_in(log: &Path, since: Option<&str>, home: Option<&Path>, cwd: Option<&Path>, kept: Option<&Kept>, linear: bool) -> std::io::Result<(Answer, Option<Kept>, u64)> {
    let agent = cwd.map(|c| normal("doc", &c.to_string_lossy(), home).2);
    let agent = agent.as_deref();
    let hash = crate::protocol::fnv1a(&log.to_string_lossy());
    let size = std::fs::metadata(log)?.len();
    // The cursor's size and newest time, when it is one of this file.
    let was = since.and_then(|c| {
        let p: Vec<&str> = c.split('.').collect();
        match p.as_slice() {
            ["r4", h, n, ts, m, r] if *h == hash => {
                let n = n.parse::<u64>().ok().filter(|&n| n <= size)?;
                (mark_of(log, n).as_deref() == Some(*m)).then_some((n, ts.parse::<i64>().ok()?, r.to_string()))
            }
            _ => None,
        }
    });
    let path = log.to_string_lossy().to_string();
    let home_s = home.map(|h| h.to_string_lossy().to_string());
    let file = identity(log).unwrap_or_default();
    let base = match kept {
        Some(k) if linear && k.rules == RULES && k.home == home_s && k.path == path && k.file == file && k.at <= k.size && k.size <= size && k.acc.known() => {
            Some(k).filter(|_| print(log, k.at).ok() == Some((k.head.clone(), k.near.clone())))
        }
        _ => None,
    };
    // A kept state that does not fit: the log changed under it (or the rules did), so the old list may not hold.
    let mut whole = linear && kept.is_some() && base.is_none();
    // Unchanged (and a kept state, if any, that fits): nothing is read or parsed.
    if let (Some(c), Some((n, _, _)), false) = (since, &was, whole) {
        if *n == size {
            return Ok((Answer { items: vec![], cursor: c.to_string(), full: false }, kept.cloned(), size));
        }
    }
    let (mut from, mut acc) = match base {
        Some(k) => (k.at, k.acc.clone()),
        None => (0, Acc::default()),
    };
    // Line starts with no tool call waiting before them: where the next read may start.
    let (items, starts, end) = loop {
        let mut p = herdr_core::ConversationParser::new();
        let mut starts: Vec<(u64, usize)> = vec![];
        let mut cwds: Vec<String> = vec![];
        let end = crate::tail::each_line(log, from, size, |l, at| {
            if !p.has_open_tool_use() {
                starts.push((at, l.len()));
            }
            if let Some(c) = line_cwd(l) {
                if cwds.last() != Some(&c) {
                    cwds.push(c);
                }
            }
            p.feed_at(l, Some(at));
        })?;
        // A result for a call before the kept start: what it changes is in the kept part. Read it all again.
        if from > 0 && p.late_results() > 0 {
            (from, acc, whole) = (0, Acc::default(), true);
            continue;
        }
        // The folders are kept with the state as they are read: a set, and the last one (lines after the next start
        // are read again next time, and give the same).
        cwds.iter().for_each(|c| acc.cwd(c));
        break (crate::tail::without_pictures(p.items()), starts, end);
    };
    // The next start: the latest line start that passes, else this one (`items[..k]` are then the final ones).
    let (mut at, mut k) = (from, 0usize);
    if linear {
        let mut checks = 0;
        for &(s, len) in starts.iter().rev().take(LOOK).filter(|x| x.0 > from) {
            // Cheap first: the line alone must start with text.
            if !starts_clean(items_in(log, s, s + len as u64 + 1)?.0.first()) {
                continue;
            }
            // No result after it for a call before it: that call's item could still change.
            let (b, late) = items_in(log, s, end)?;
            if late == 0 && starts_clean(b.first()) && b.len() <= items.len() && items[items.len() - b.len()..] == b[..] {
                (at, k) = (s, items.len() - b.len());
                break;
            }
            checks += 1;
            if checks == CHECKS {
                break;
            }
        }
    }
    acc.add(&items[..k], home);
    let next = match linear {
        true => {
            let (head, near) = print(log, at)?;
            Some(Kept { rules: RULES, home: home_s, path, size, at, head, near, file, acc: acc.clone() })
        }
        false => None,
    };
    acc.add(&items[k..], home);
    let all = acc.finish_in(home, agent);
    let newest = all.iter().filter_map(|r| r.last_ts).max().unwrap_or(0);
    // Where the relative doc paths open now: when one moved (its item has another id), all of it again.
    let res = crate::protocol::fnv1a(&format!("{:?}", acc.resolved(agent)));
    let cursor = format!("r4.{hash}.{size}.{newest}.{}.{res}", mark_of(log, size).unwrap_or_default());
    let answer = match was {
        Some((n, ts, r)) if n < size && !whole && r == res => Answer { items: all.into_iter().filter(|r| r.last_ts.is_some_and(|t| t >= ts)).collect(), cursor, full: false },
        _ => Answer { items: all, cursor, full: true },
    };
    Ok((answer, next, from))
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
