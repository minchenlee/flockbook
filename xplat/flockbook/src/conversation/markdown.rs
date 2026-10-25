//! Markdown to flow rows. Uses pulldown-cmark directly (iced's markdown model hides table cells), so the plain
//! text of a row is exactly the text that is drawn.

use fb_ui::selectable::flow::{Inline, Para, RowKind};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// A row before it gets an id.
#[derive(Debug, Clone)]
pub struct Spec {
    pub kind: RowKind,
    pub paras: Vec<Para>,
    pub separator: &'static str,
    pub gap: f32,
}

struct TableB {
    cols: usize,
    cells: Vec<Para>,
}

struct B {
    out: Vec<Spec>,
    inl: Vec<Inline>,
    bold: u32,
    italic: u32,
    strike: u32,
    link: Option<String>,
    /// Open lists: the next number of an ordered list, or `None` for a bullet list.
    lists: Vec<Option<u64>>,
    marker: Option<String>,
    quote: u32,
    code: Option<(Option<String>, String)>,
    heading: Option<u8>,
    table: Option<TableB>,
    soft: &'static str,
}

impl B {
    fn push_inline(&mut self, text: &str, code: bool) {
        if text.is_empty() {
            return;
        }
        self.inl.push(Inline { text: text.to_string(), bold: self.bold > 0, italic: self.italic > 0, code, strike: self.strike > 0, link: self.link.clone(), ..Default::default() });
    }

    fn flush(&mut self) {
        if self.inl.is_empty() {
            return;
        }
        let para = Para { inlines: std::mem::take(&mut self.inl) };
        let prev_is_item = matches!(self.out.last().map(|s| &s.kind), Some(RowKind::ListItem { .. }));
        let (kind, separator) = if let Some(l) = self.heading.take() {
            (RowKind::Heading(l), "\n\n")
        } else if !self.lists.is_empty() {
            let marker = self.marker.take().unwrap_or_default();
            (RowKind::ListItem { marker, depth: self.lists.len() as u8 }, if prev_is_item { "\n" } else { "\n\n" })
        } else if self.quote > 0 {
            (RowKind::Quote, "\n\n")
        } else {
            (RowKind::Prose, "\n\n")
        };
        let gap = gap_between(self.out.last().map(|s| &s.kind), &kind);
        self.out.push(Spec { kind, paras: vec![para], separator, gap });
    }

    fn pad_row(&mut self) {
        if let Some(t) = &mut self.table {
            while t.cells.len() % t.cols != 0 {
                t.cells.push(Para::default());
            }
        }
    }
}

/// Blocks whose margin above and below is the wide one.
fn heavy(k: &RowKind) -> bool {
    matches!(k, RowKind::Code { .. } | RowKind::Table { .. } | RowKind::Quote)
}

/// The gap above a block, ported from `MarkdownView.gap` (reading.md 3): headings claim their own margins,
/// code, quotes and tables 20, list items tighten to their depth, rules 24, anything else 14.
fn gap_between(prev: Option<&RowKind>, cur: &RowKind) -> f32 {
    if let RowKind::Heading(l) = cur {
        return if *l == 1 {
            32.0
        } else if *l == 2 {
            28.0
        } else {
            22.0
        };
    }
    if let Some(RowKind::Heading(l)) = prev {
        return if *l == 1 {
            12.0
        } else if *l == 2 {
            10.0
        } else {
            6.0
        };
    }
    if matches!(prev, Some(k) if heavy(k)) || heavy(cur) {
        return 20.0;
    }
    match (prev, cur) {
        (Some(RowKind::ListItem { depth: a, .. }), RowKind::ListItem { depth: b, .. }) => {
            if b > a {
                4.0
            } else {
                6.0
            }
        }
        (Some(RowKind::ListItem { .. }), _) | (_, RowKind::ListItem { .. }) => 10.0,
        (Some(RowKind::Rule), _) | (_, RowKind::Rule) => 24.0,
        _ => 14.0,
    }
}

fn level(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        _ => 4,
    }
}

/// One paragraph of highlighted code: one inline per run, so `plain()` is the code itself.
pub fn code_para(lang: Option<&str>, code: &str) -> Para {
    let runs = fb_ui::highlight::tokenize(lang, code);
    if runs.is_empty() {
        return Para::text(code);
    }
    Para { inlines: runs.into_iter().map(|(tone, text)| Inline { text: text.to_string(), tone, ..Default::default() }).collect() }
}

/// Parses markdown. `preserve_breaks` keeps single newlines (user bubbles).
pub fn parse(text: &str, preserve_breaks: bool) -> Vec<Spec> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut b = B { out: Vec::new(), inl: Vec::new(), bold: 0, italic: 0, strike: 0, link: None, lists: Vec::new(), marker: None, quote: 0, code: None, heading: None, table: None, soft: if preserve_breaks { "\n" } else { " " } };
    for ev in Parser::new_ext(text, opts) {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Heading { level: l, .. } => b.heading = Some(level(l)),
                Tag::BlockQuote(_) => b.quote += 1,
                Tag::CodeBlock(kind) => {
                    b.flush();
                    let lang = match kind {
                        CodeBlockKind::Fenced(l) => Some(l.split_whitespace().next().unwrap_or("").to_string()).filter(|l| !l.is_empty()),
                        CodeBlockKind::Indented => None,
                    };
                    b.code = Some((lang, String::new()));
                }
                Tag::List(start) => {
                    b.flush();
                    b.lists.push(start);
                }
                Tag::Item => {
                    b.marker = Some(match b.lists.last_mut() {
                        Some(Some(n)) => {
                            let m = format!("{n}.");
                            *n += 1;
                            m
                        }
                        _ => "\u{2022}".to_string(),
                    });
                }
                Tag::Table(aligns) => {
                    b.flush();
                    b.table = Some(TableB { cols: aligns.len().max(1), cells: Vec::new() });
                }
                Tag::Emphasis => b.italic += 1,
                Tag::Strong => b.bold += 1,
                Tag::Strikethrough => b.strike += 1,
                Tag::Link { dest_url, .. } => b.link = Some(dest_url.to_string()),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::Heading(_) => b.flush(),
                TagEnd::BlockQuote(_) => {
                    b.flush();
                    b.quote = b.quote.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    if let Some((lang, code)) = b.code.take() {
                        let code = code.trim_end_matches('\n').to_string();
                        let kind = RowKind::Code { lang: lang.clone() };
                        let gap = gap_between(b.out.last().map(|s| &s.kind), &kind);
                        b.out.push(Spec { kind, paras: vec![code_para(lang.as_deref(), &code)], separator: "\n\n", gap });
                    }
                }
                TagEnd::List(_) => {
                    b.flush();
                    b.lists.pop();
                }
                TagEnd::Item => b.flush(),
                TagEnd::TableCell => {
                    let cell = Para { inlines: std::mem::take(&mut b.inl) };
                    if let Some(t) = &mut b.table {
                        t.cells.push(cell);
                    }
                }
                TagEnd::TableHead | TagEnd::TableRow => b.pad_row(),
                TagEnd::Table => {
                    if let Some(t) = b.table.take() {
                        let kind = RowKind::Table { cols: t.cols };
                        let gap = gap_between(b.out.last().map(|s| &s.kind), &kind);
                        b.out.push(Spec { kind, paras: t.cells, separator: "\n\n", gap });
                    }
                }
                TagEnd::Emphasis => b.italic = b.italic.saturating_sub(1),
                TagEnd::Strong => b.bold = b.bold.saturating_sub(1),
                TagEnd::Strikethrough => b.strike = b.strike.saturating_sub(1),
                TagEnd::Link => b.link = None,
                _ => {}
            },
            Event::Text(t) => {
                if let Some((_, buf)) = &mut b.code {
                    buf.push_str(&t);
                } else {
                    b.push_inline(&t, false);
                }
            }
            Event::Code(t) => b.push_inline(&t, true),
            Event::Html(t) | Event::InlineHtml(t) => {
                if let Some((_, buf)) = &mut b.code {
                    buf.push_str(&t);
                } else {
                    b.push_inline(t.trim_end_matches('\n'), false);
                }
            }
            Event::SoftBreak => {
                let s = b.soft;
                b.push_inline(s, false);
            }
            Event::HardBreak => b.push_inline("\n", false),
            Event::Rule => {
                b.flush();
                let kind = RowKind::Rule;
                let gap = gap_between(b.out.last().map(|s| &s.kind), &kind);
                b.out.push(Spec { kind, paras: vec![], separator: "\n\n", gap });
            }
            Event::TaskListMarker(done) => b.push_inline(if done { "\u{2611} " } else { "\u{2610} " }, false),
            _ => {}
        }
    }
    b.flush();
    b.out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &[Spec]) -> Vec<String> {
        s.iter().map(|x| format!("{:?}", x.kind).split(['{', '(']).next().unwrap().trim().to_string()).collect()
    }

    #[test]
    fn mixed_markdown_flattens_to_rows() {
        let md = "Intro with `code` and [a link](https://example.com).\n\n## Heading\n\n1. one\n2. two\n\n```python\nx = 1\n```\n\n| A | B |\n|---|---|\n| 1 | **2** |\n\n> quoted\n";
        let s = parse(md, false);
        assert_eq!(kinds(&s), ["Prose", "Heading", "ListItem", "ListItem", "Code", "Table", "Quote"]);
        let RowKind::ListItem { marker, depth } = &s[3].kind else { panic!() };
        assert_eq!((marker.as_str(), *depth), ("2.", 1));
        let RowKind::Table { cols } = &s[5].kind else { panic!() };
        assert_eq!(*cols, 2);
        assert_eq!(s[5].paras.len(), 4);
        assert!(s[0].paras[0].inlines.iter().any(|i| i.code));
        assert!(s[0].paras[0].inlines.iter().any(|i| i.link.as_deref() == Some("https://example.com")));
        let RowKind::Code { lang } = &s[4].kind else { panic!() };
        assert_eq!(lang.as_deref(), Some("python"));
        assert_eq!(s[4].paras[0].plain(), "x = 1");
    }

    #[test]
    fn code_blocks_carry_syntax_tones_and_keep_the_text() {
        use fb_ui::highlight::Tone;
        let s = parse("```python\ndef f(a):\n    return a + 1  # one\n```", false);
        let p = &s[0].paras[0];
        assert_eq!(p.plain(), "def f(a):\n    return a + 1  # one");
        let has = |t: Tone, x: &str| p.inlines.iter().any(|i| i.tone == t && i.text == x);
        assert!(has(Tone::Keyword, "def") && has(Tone::Function, "f") && has(Tone::Number, "1") && has(Tone::Comment, "# one"));
        // No language: plain.
        let s = parse("```\ndef f(a): pass\n```", false);
        assert!(s[0].paras[0].inlines.iter().all(|i| i.tone == Tone::Plain));
    }

    #[test]
    fn nested_lists_and_tight_items() {
        let s = parse("- a\n  - b\n- c\n", false);
        let d: Vec<u8> = s.iter().map(|x| if let RowKind::ListItem { depth, .. } = x.kind { depth } else { 0 }).collect();
        assert_eq!(d, [1, 2, 1]);
        assert_eq!(s[1].separator, "\n");
    }

    #[test]
    fn soft_breaks_follow_the_mode() {
        assert_eq!(parse("a\nb", false)[0].paras[0].plain(), "a b");
        assert_eq!(parse("a\nb", true)[0].paras[0].plain(), "a\nb");
    }

    #[test]
    fn cjk_and_empty_input() {
        let s = parse("結果摘要：新上架的商品現在排第一。", false);
        assert_eq!(s[0].paras[0].plain(), "結果摘要：新上架的商品現在排第一。");
        assert!(parse("", false).is_empty());
    }

    #[test]
    fn cjk_inline_span_boundaries_add_no_spaces() {
        let s = parse("長尾查詢可能受到*衰減*影響，需要重放驗證。", false);
        assert_eq!(s[0].paras[0].plain(), "長尾查詢可能受到衰減影響，需要重放驗證。");
        assert!(s[0].paras[0].inlines.iter().any(|i| i.text == "衰減" && i.italic), "emphasis is its own inline");
        let s = parse("`half_life_days`設為 30 只是起點。", false);
        assert_eq!(s[0].paras[0].plain(), "half_life_days設為 30 只是起點。");
        let s = parse("- 受到**衰減**影響\n- `x`設為", false);
        let plains: Vec<String> = s.iter().map(|r| r.paras[0].plain()).collect();
        assert_eq!(plains, ["受到衰減影響", "x設為"]);
    }
}
