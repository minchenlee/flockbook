//! The chat's blocks (herdr-app/mods/flockbook/blocks.js `splitBlocks`, ported): the mod draws code fences and tables
//! itself and each other block alone as Markdown (TextView). A link is written as a Markdown link only where both the
//! whole reply and the block drawn alone read it as plain text; `block` on a FeedLink says which block that was
//! (docs/protocol.md, `links`). Both sides check the same cases: Tests/Fixtures/resources/md-links.json.

use crate::resources::FeedLink;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// JavaScript's `\s` (and what `trim` takes off).
fn js_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// JavaScript's line terminators (what `.` does not match).
fn js_line_end(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn skip_space(s: &str) -> &str {
    s.trim_start_matches(js_space)
}

/// A run of 3 or more of `c` at the start: (the run, the rest).
fn run_of(s: &str, c: char) -> Option<(&str, &str)> {
    let n = s.len() - s.trim_start_matches(c).len();
    (n >= 3 * c.len_utf8()).then(|| s.split_at(n))
}

/// blocks.js OPEN, then TILDE_OPEN: the fence (its run of backticks or tildes) when `line` opens one.
fn opens(line: &str) -> Option<&str> {
    let s = skip_space(line);
    let (fence, rest) = run_of(s, '`').or_else(|| run_of(s, '~'))?;
    // OPEN: nothing after it is a backtick.
    if !rest.contains('`') {
        return Some(fence);
    }
    // TILDE_OPEN: tildes; after spaces and one word, no line terminator (".*$").
    if fence.starts_with('~') {
        let after = skip_space(rest);
        let word = after.len() - after.trim_start_matches(|c: char| !js_space(c)).len();
        if !after[word..].contains(js_line_end) {
            return Some(fence);
        }
    }
    None
}

/// blocks.js `closes`: the same character, at least as many, nothing after it but spaces.
fn closes(line: &str, fence: &str) -> bool {
    let s = skip_space(line);
    let c = fence.chars().next().unwrap_or('`');
    match run_of(s, c) {
        Some((run, rest)) => run.len() >= fence.len() && skip_space(rest).is_empty(),
        None => false,
    }
}

/// blocks.js TABLE_SEP: "| --- | :--: |".
fn table_sep(line: &str) -> bool {
    // One cell: spaces, ":"?, 2 or more "-", ":"?, spaces. The rest after it, or None.
    fn cell(s: &str) -> Option<&str> {
        let s = skip_space(s);
        let s = s.strip_prefix(':').unwrap_or(s);
        let n = s.len() - s.trim_start_matches('-').len();
        if n < 2 {
            return None;
        }
        let s = &s[n..];
        Some(skip_space(s.strip_prefix(':').unwrap_or(s)))
    }
    let s = skip_space(line);
    let s = skip_space(s.strip_prefix('|').unwrap_or(s));
    let Some(mut rest) = cell(s) else { return false };
    loop {
        match rest.strip_prefix('|') {
            Some(after) => match cell(after) {
                Some(r) => rest = r,
                None => return skip_space(after).is_empty(),
            },
            None => return rest.is_empty(),
        }
    }
}

fn pipe_line(line: &str) -> bool {
    skip_space(line).starts_with('|')
}

/// The Markdown blocks the mod draws alone, as byte ranges of `text`, in order: what blocks.js `splitBlocks` gives as
/// `k: "md"` (lines between fences and tables; one of blank lines only is left out).
pub fn md_blocks(text: &str) -> Vec<std::ops::Range<usize>> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut starts = Vec::with_capacity(lines.len());
    let mut pos = 0;
    for l in &lines {
        starts.push(pos);
        pos += l.len() + 1;
    }
    let mut out = vec![];
    let mut md: Option<(usize, usize)> = None; // first and last line
    let flush = |md: &mut Option<(usize, usize)>, out: &mut Vec<std::ops::Range<usize>>| {
        if let Some((a, b)) = md.take() {
            let r = starts[a]..starts[b] + lines[b].len();
            if !text[r.clone()].trim_matches(js_space).is_empty() {
                out.push(r);
            }
        }
    };
    let mut i = 0;
    while i < lines.len() {
        if let Some(fence) = opens(lines[i]) {
            flush(&mut md, &mut out);
            i += 1;
            while i < lines.len() && !closes(lines[i], fence) {
                i += 1;
            }
            i += 1;
            continue;
        }
        if pipe_line(lines[i]) && i + 1 < lines.len() && table_sep(lines[i + 1]) {
            flush(&mut md, &mut out);
            i += 2;
            while i < lines.len() && pipe_line(lines[i]) {
                i += 1;
            }
            continue;
        }
        md = Some((md.map_or(i, |m| m.0), i));
        i += 1;
    }
    flush(&mut md, &mut out);
    out
}

/// Which bytes of `text` CommonMark reads as plain text, as written: not in code, a link, a picture or HTML (nor
/// after an inline tag in its block), and not an escape or an entity. The same reading as resources::detect.
fn plain(text: &str) -> Vec<bool> {
    let mut ok = vec![false; text.len()];
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let (mut inside, mut html) = (0usize, false);
    for (ev, r) in Parser::new_ext(text, opts).into_offset_iter() {
        match ev {
            Event::Start(Tag::Link { .. } | Tag::Image { .. } | Tag::CodeBlock(_) | Tag::HtmlBlock) => inside += 1,
            Event::End(TagEnd::Link | TagEnd::Image | TagEnd::CodeBlock | TagEnd::HtmlBlock) => inside = inside.saturating_sub(1),
            Event::InlineHtml(_) => html = true,
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell | TagEnd::Item) => html = false,
            Event::Text(t) if inside == 0 && !html && text[r.clone()] == *t => ok[r].iter_mut().for_each(|x| *x = true),
            _ => {}
        }
    }
    ok
}

/// Sets each link's `block`: the UTF-16 start of the block it is in, when it is plain text there and in the whole
/// `text` and not after "!" (it would be a picture). Else `None`: the mod leaves it as text. `text`: the row's
/// text as sent; the links' offsets are UTF-16 in it.
pub fn judge(text: &str, mut links: Vec<FeedLink>) -> Vec<FeedLink> {
    // UTF-16 offset -> byte offset (one entry per UTF-16 unit, and the end).
    let mut byte = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.char_indices() {
        for _ in 0..c.len_utf16() {
            byte.push(i);
        }
    }
    byte.push(text.len());
    let whole = plain(text);
    let blocks: Vec<(std::ops::Range<usize>, Vec<bool>)> = md_blocks(text).into_iter().map(|r| {
        let p = plain(&text[r.clone()]);
        (r, p)
    }).collect();
    for l in &mut links {
        l.block = None;
        let (Some(&s), Some(&e)) = (byte.get(l.start), byte.get(l.end)) else { continue };
        if s >= e || text[..s].ends_with('!') || !whole[s..e].iter().all(|&x| x) {
            continue;
        }
        if let Some((r, p)) = blocks.iter().find(|(r, _)| r.start <= s && e <= r.end) {
            if p[s - r.start..e - r.start].iter().all(|&x| x) {
                l.block = Some(text[..r.start].encode_utf16().count());
            }
        }
    }
    links
}
