//! Conversation items to flow rows: merged chip rows, time separators, one footer per agent turn, row gaps.
//! Ports the entry rules of the Swift `ConversationView` (gaps: 32 before a user bubble, 20 after it, 8 around
//! chips, 24 around separators, 12 before a footer).

use super::icon_for_symbol;
use super::markdown::{self, Spec};
use chrono::{DateTime, Local, TimeZone};
use fb_ui::icons::Icon;
use fb_ui::selectable::flow::{row_id, Chip, Para, Row, RowKind};
use herdr_core::{ConversationItem, ItemKind, PermState, QuestionState, ToolChip};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Rows cached per item id: (signature, first-row gap, rows).
pub type RowCache = HashMap<String, (u64, f32, Vec<Arc<Row>>)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tag {
    User,
    Assistant,
    Chips,
    Other,
    Status,
    Footer,
    Sep,
}

/// One displayed item: adjacent tool-chip items are merged into one.
struct Disp<'a> {
    id: &'a str,
    ts: Option<f64>,
    kind: DKind<'a>,
}

enum DKind<'a> {
    User(&'a str),
    Assistant(&'a str),
    Chips(Vec<&'a ToolChip>),
    Running(&'a str),
    Answered(String),
    Status(&'a str),
    Images(&'a [herdr_core::ImageRef]),
    Nothing,
}

fn tag_of(k: &DKind) -> Tag {
    match k {
        DKind::User(_) => Tag::User,
        DKind::Assistant(_) => Tag::Assistant,
        DKind::Chips(_) | DKind::Running(_) => Tag::Chips,
        DKind::Status(_) | DKind::Answered(_) => Tag::Status,
        DKind::Images(_) | DKind::Nothing => Tag::Other,
    }
}

fn gap(prev: Tag, cur: Tag) -> f32 {
    match (prev, cur) {
        (Tag::Sep, _) | (_, Tag::Sep) => 24.0,
        (_, Tag::Footer) => 12.0,
        (_, Tag::User) => 32.0,
        (Tag::User, _) => 20.0,
        (Tag::Chips, _) | (_, Tag::Chips) => 8.0,
        (Tag::Status, _) | (_, Tag::Status) => 16.0,
        _ => 14.0,
    }
}

fn hash_str(h: &mut DefaultHasher, s: &str) {
    s.hash(h);
}

fn item_sig(d: &Disp, expanded: &HashSet<String>) -> u64 {
    let mut h = DefaultHasher::new();
    hash_str(&mut h, d.id);
    match &d.kind {
        DKind::User(s) | DKind::Assistant(s) | DKind::Status(s) | DKind::Running(s) => {
            tag_of(&d.kind).hash_tag(&mut h);
            hash_str(&mut h, s)
        }
        DKind::Answered(s) => hash_str(&mut h, s),
        DKind::Chips(cs) => {
            if let Some(f) = cs.first() {
                expanded.contains(&format!("more:{}", f.id)).hash(&mut h);
            }
            for c in cs {
                hash_str(&mut h, &c.id);
                hash_str(&mut h, &c.text);
                hash_str(&mut h, &c.symbol);
                expanded.contains(&c.id).hash(&mut h);
                for call in &c.calls {
                    hash_str(&mut h, &call.summary);
                    call.output.as_deref().map(str::len).hash(&mut h);
                    call.is_error.hash(&mut h);
                }
            }
        }
        DKind::Images(rs) => rs.hash(&mut h),
        DKind::Nothing => {}
    }
    h.finish()
}

impl Tag {
    fn hash_tag(self, h: &mut DefaultHasher) {
        (self as u8).hash(h);
    }
}

fn clock(ts: f64) -> String {
    local(ts).format("%H:%M").to_string()
}

fn local(ts: f64) -> DateTime<Local> {
    Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .unwrap_or_else(Local::now)
}

fn duration(s: f64) -> String {
    let n = s.round() as i64;
    if n >= 3600 {
        format!("{}h {}m", n / 3600, n % 3600 / 60)
    } else if n >= 60 {
        format!("{}m {}s", n / 60, n % 60)
    } else {
        format!("{n}s")
    }
}

fn separator_text(ts: f64) -> String {
    let d = local(ts);
    let today = Local::now().date_naive();
    if d.date_naive() == today {
        clock(ts)
    } else if d.date_naive() == today.pred_opt().unwrap_or(today) {
        format!("Yesterday {}", clock(ts))
    } else {
        d.format("%a %-d %b %H:%M").to_string()
    }
}

/// Output lines shown under an expanded call.
const OUTPUT_LINES: usize = 14;

fn call_text(c: &herdr_core::ToolCall) -> String {
    let mut s = c.summary.clone();
    if let Some(out) = &c.output {
        let out = out.trim_end();
        if !out.is_empty() {
            let lines: Vec<&str> = out.lines().collect();
            s.push_str("\n\n");
            s.push_str(
                &lines
                    .iter()
                    .take(OUTPUT_LINES)
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            if lines.len() > OUTPUT_LINES {
                s.push_str(&format!(
                    "\n\u{2026} {} more lines",
                    lines.len() - OUTPUT_LINES
                ));
            }
        }
    }
    s
}

fn specs_of(d: &Disp, expanded: &HashSet<String>) -> Vec<Spec> {
    match &d.kind {
        DKind::User(s) => {
            // One bubble: every block of the message joins into one paragraph.
            let mut inl = Vec::new();
            for (i, sp) in markdown::parse(s, true).into_iter().enumerate() {
                if i > 0 {
                    inl.push(fb_ui::selectable::flow::Inline::plain("\n\n"));
                }
                for p in sp.paras {
                    inl.extend(p.inlines);
                }
            }
            if inl.is_empty() {
                inl.push(fb_ui::selectable::flow::Inline::plain(s.to_string()));
            }
            vec![Spec {
                kind: RowKind::Bubble,
                paras: vec![Para { inlines: inl }],
                separator: "\n\n",
                gap: 0.0,
            }]
        }
        DKind::Assistant(s) => markdown::parse(s, false),
        DKind::Chips(cs) => {
            // Swift ChipGroup: more than five chips show the first four and a "+N more" chip that shows them all.
            let more_id = format!("more:{}", cs.first().map_or("", |c| c.id.as_str()));
            let show_all = expanded.contains(&more_id);
            let shown = if cs.len() > 5 && !show_all { 4 } else { cs.len() };
            let mut chips: Vec<Chip> = cs[..shown]
                .iter()
                .map(|c| {
                    let running = c.calls.iter().any(|x| x.output.is_none());
                    Chip {
                        id: c.id.clone(),
                        icon: if running {
                            Icon::Activity
                        } else {
                            icon_for_symbol(&c.symbol)
                        },
                        text: c.text.clone(),
                        expandable: !c.calls.is_empty(),
                        expanded: expanded.contains(&c.id),
                        running,
                        error: c.calls.iter().any(|x| x.is_error),
                    }
                })
                .collect();
            if cs.len() > 5 {
                chips.push(Chip {
                    id: more_id,
                    icon: if show_all { Icon::ChevronUp } else { Icon::Ellipsis },
                    text: if show_all {
                        "Show fewer".to_string()
                    } else {
                        format!("+{} more", cs.len() - 4)
                    },
                    expandable: false,
                    expanded: false,
                    running: false,
                    error: false,
                });
            }
            let mut out = vec![Spec {
                kind: RowKind::Chips(chips),
                paras: vec![],
                separator: "",
                gap: 0.0,
            }];
            for c in cs[..shown].iter().filter(|c| expanded.contains(&c.id)) {
                for call in &c.calls {
                    out.push(Spec {
                        kind: RowKind::Code {
                            lang: Some(call.tool_name.clone()),
                        },
                        paras: vec![Para::text(call_text(call))],
                        separator: "\n\n",
                        gap: 8.0,
                    });
                }
            }
            out
        }
        DKind::Running(tool) => {
            let name = if tool.is_empty() { "tool" } else { tool };
            let chip = Chip {
                id: format!("running-{name}"),
                icon: Icon::Activity,
                text: format!("Running {name}\u{2026}"),
                expandable: false,
                expanded: false,
                running: true,
                error: false,
            };
            vec![Spec {
                kind: RowKind::Chips(vec![chip]),
                paras: vec![],
                separator: "",
                gap: 0.0,
            }]
        }
        DKind::Answered(s) => vec![Spec {
            kind: RowKind::Caption {
                center: true,
                rule: false,
            },
            paras: vec![Para::text(s.clone())],
            separator: "",
            gap: 0.0,
        }],
        DKind::Status(s) => vec![Spec {
            kind: RowKind::Caption {
                center: true,
                rule: false,
            },
            paras: vec![Para::text(s.to_string())],
            separator: "",
            gap: 0.0,
        }],
        DKind::Images(rs) => rs
            .iter()
            .map(|r| Spec {
                kind: RowKind::Picture {
                    width: r.width,
                    height: r.height,
                    digest: r.digest,
                },
                paras: vec![],
                separator: "",
                gap: 8.,
            })
            .collect(),
        DKind::Nothing => vec![],
    }
}

fn disp(items: &[ConversationItem]) -> Vec<Disp<'_>> {
    let mut out: Vec<Disp> = Vec::with_capacity(items.len());
    for it in items {
        let kind = match &it.kind {
            ItemKind::Images(rs) => DKind::Images(rs),
            ItemKind::UserText(s) => DKind::User(s),
            ItemKind::AssistantText(s) => DKind::Assistant(s),
            ItemKind::ToolChips(c) => {
                if let Some(Disp {
                    kind: DKind::Chips(prev),
                    ..
                }) = out.last_mut()
                {
                    prev.extend(c.iter());
                    continue;
                }
                DKind::Chips(c.iter().collect())
            }
            ItemKind::PermissionRequest(p) => {
                if p.state == PermState::Pending {
                    DKind::Running(&p.tool_name)
                } else {
                    DKind::Nothing
                }
            }
            ItemKind::Question(q) => match &q.state {
                QuestionState::Pending => DKind::Nothing,
                QuestionState::Answered(Some(a)) => DKind::Answered(format!(
                    "Answered: {}",
                    a.iter()
                        .filter(|x| !x.is_empty())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" \u{00B7} ")
                )),
                QuestionState::Answered(None) => DKind::Answered("Answered".to_string()),
                QuestionState::Declined => DKind::Answered("Declined".to_string()),
            },
            ItemKind::Status(s) => DKind::Status(s),
        };
        out.push(Disp {
            id: &it.id,
            ts: it.timestamp,
            kind,
        });
    }
    out
}

fn mk_row(id: &str, i: usize, s: Spec, gap: f32) -> Arc<Row> {
    Arc::new(Row::new(
        row_id(&[id, &i.to_string()]),
        s.kind,
        s.paras,
        s.separator,
        gap,
    ))
}

/// Builds all rows. `cache` keeps rows per item so a refresh only parses what changed.
pub fn build_rows(
    items: &[ConversationItem],
    expanded: &HashSet<String>,
    cache: &mut RowCache,
    more_earlier: bool,
    loading: bool,
) -> Vec<Arc<Row>> {
    let disps = disp(items);
    let mut rows: Vec<Arc<Row>> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut prev_tag: Option<Tag> = None;
    let mut prev_row_tag: Option<Tag> = None;
    let mut prev_time: Option<f64> = None;
    // Turn bookkeeping for footers.
    let (mut turn_start, mut turn_end): (Option<f64>, Option<f64>) = (None, None);
    let (mut tools, mut has_reply) = (0usize, false);
    let mut last_id: Option<&str> = None;

    if more_earlier {
        rows.push(Arc::new(Row::new(
            row_id(&["load-earlier"]),
            RowKind::LoadEarlier { loading },
            vec![],
            "",
            0.0,
        )));
    }

    macro_rules! push_caption {
        ($id:expr, $text:expr, $center:expr, $rule:expr, $tag:expr) => {{
            let g = if rows.is_empty() {
                0.0
            } else {
                gap(prev_tag.unwrap_or(Tag::Other), $tag)
            };
            let spec = Spec {
                kind: RowKind::Caption {
                    center: $center,
                    rule: $rule,
                },
                paras: vec![Para::text($text)],
                separator: "",
                gap: g,
            };
            rows.push(mk_row($id, 0, spec, g));
            prev_tag = Some($tag);
        }};
    }

    let close_turn = |rows: &mut Vec<Arc<Row>>,
                      prev_tag: &mut Option<Tag>,
                      start: Option<f64>,
                      end: Option<f64>,
                      tools: usize,
                      has_reply: bool,
                      last: Option<&str>,
                      prev_row_tag: Option<Tag>| {
        let (Some(end), Some(last), true) = (end, last, has_reply) else {
            return;
        };
        let mut parts = vec![clock(end)];
        if let Some(s) = start {
            if end - s >= 1.0 {
                parts.push(format!("Worked {}", duration(end - s)));
            }
        }
        if tools > 0 {
            parts.push(format!("{tools} tool{}", if tools == 1 { "" } else { "s" }));
        }
        let _ = prev_row_tag;
        let g = gap(prev_tag.unwrap_or(Tag::Other), Tag::Footer);
        let spec = Spec {
            kind: RowKind::Caption {
                center: false,
                rule: false,
            },
            paras: vec![Para::text(parts.join(" \u{00B7} "))],
            separator: "",
            gap: g,
        };
        rows.push(mk_row(&format!("f-{last}"), 0, spec, g));
        *prev_tag = Some(Tag::Footer);
    };

    for d in &disps {
        let tag = tag_of(&d.kind);
        if tag == Tag::User {
            close_turn(
                &mut rows,
                &mut prev_tag,
                turn_start,
                turn_end,
                tools,
                has_reply,
                last_id,
                prev_row_tag,
            );
            turn_start = d.ts;
            turn_end = None;
            tools = 0;
            has_reply = false;
        }
        if let Some(t) = d.ts {
            if let Some(p) = prev_time {
                let new_day = local(t).date_naive() != local(p).date_naive();
                if t - p >= 900.0 || new_day {
                    // A separator ends the turn's footer logic only through the next user message, as in Swift.
                    push_caption!(
                        &format!("s-{}", d.id),
                        separator_text(t),
                        true,
                        true,
                        Tag::Sep
                    );
                }
            }
            prev_time = Some(t);
            turn_end = Some(t);
        }
        if tag == Tag::Assistant {
            has_reply = true;
        }
        if let DKind::Chips(cs) = &d.kind {
            tools += cs.iter().map(|c| c.calls.len().max(1)).sum::<usize>();
        }
        if matches!(d.kind, DKind::Nothing) {
            continue;
        }
        // The gap before the item's first row.
        let first_gap = if rows.is_empty() || (rows.len() == 1 && more_earlier) {
            if more_earlier {
                20.0
            } else {
                0.0
            }
        } else if prev_tag == Some(Tag::Footer) {
            gap(prev_row_tag.unwrap_or(Tag::Other), tag)
        } else {
            gap(prev_tag.unwrap_or(Tag::Other), tag)
        };
        let sig = item_sig(d, expanded);
        seen.insert(d.id);
        let reuse = cache
            .get(d.id)
            .filter(|(s, g, _)| *s == sig && (*g - first_gap).abs() < 0.01)
            .map(|(_, _, r)| r.clone());
        let item_rows = match reuse {
            Some(r) => r,
            None => {
                let specs = specs_of(d, expanded);
                let r: Vec<Arc<Row>> = specs
                    .into_iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let g = if i == 0 { first_gap } else { s.gap };
                        mk_row(d.id, i, s, g)
                    })
                    .collect();
                cache.insert(d.id.to_string(), (sig, first_gap, r.clone()));
                r
            }
        };
        if item_rows.is_empty() {
            continue;
        }
        rows.extend(item_rows);
        prev_tag = Some(tag);
        prev_row_tag = Some(tag);
        last_id = Some(d.id);
    }
    close_turn(
        &mut rows,
        &mut prev_tag,
        turn_start,
        turn_end,
        tools,
        has_reply,
        last_id,
        prev_row_tag,
    );
    cache.retain(|k, _| seen.contains(k.as_str()));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_core::{ToolCall, ToolChip};

    fn item(id: &str, kind: ItemKind, ts: f64) -> ConversationItem {
        ConversationItem {
            id: id.into(),
            kind,
            timestamp: Some(ts),
        }
    }

    fn chip(id: &str) -> ToolChip {
        ToolChip {
            id: id.into(),
            symbol: "terminal".into(),
            text: "Ran a command".into(),
            calls: vec![ToolCall {
                id: "c".into(),
                tool_name: "Bash".into(),
                summary: "ls".into(),
                input: "ls".into(),
                output: Some("a\nb".into()),
                is_error: false,
            }],
        }
    }

    #[test]
    fn rows_have_gaps_footer_and_merged_chips() {
        let items = vec![
            item("u", ItemKind::UserText("hi".into()), 1000.0),
            item("c1", ItemKind::ToolChips(vec![chip("a")]), 1010.0),
            item("c2", ItemKind::ToolChips(vec![chip("b")]), 1011.0),
            item(
                "a",
                ItemKind::AssistantText("hello\n\nworld".into()),
                1100.0,
            ),
        ];
        let mut cache = RowCache::new();
        let rows = build_rows(&items, &HashSet::new(), &mut cache, false, false);
        let kinds: Vec<_> = rows
            .iter()
            .map(|r| std::mem::discriminant(&r.kind))
            .collect();
        assert_eq!(kinds.len(), 5, "bubble, one chip row, two prose, footer");
        assert!(matches!(&rows[1].kind, RowKind::Chips(c) if c.len() == 2));
        assert_eq!(rows[0].gap_before, 0.0);
        assert_eq!(rows[1].gap_before, 20.0);
        assert!(matches!(
            rows[4].kind,
            RowKind::Caption { center: false, .. }
        ));
        assert!(rows[4].paras[0].plain().contains("Worked 1m 40s"));
        assert!(rows[4].paras[0].plain().contains("2 tools"));
        // A second build reuses the cached rows.
        let again = build_rows(&items, &HashSet::new(), &mut cache, false, false);
        assert!(Arc::ptr_eq(&rows[1], &again[1]));
    }

    #[test]
    fn expanded_chip_adds_a_code_row_and_changes_the_signature() {
        let items = vec![item("c1", ItemKind::ToolChips(vec![chip("a")]), 1000.0)];
        let mut cache = RowCache::new();
        let closed = build_rows(&items, &HashSet::new(), &mut cache, false, false);
        let open = build_rows(
            &items,
            &["a".to_string()].into_iter().collect(),
            &mut cache,
            false,
            false,
        );
        assert_eq!(closed.len(), 1);
        assert_eq!(open.len(), 2);
        assert!(matches!(&open[1].kind, RowKind::Code { lang } if lang.as_deref() == Some("Bash")));
        assert!(open[1].plain.contains("ls") && open[1].plain.contains("a\nb"));
    }

    #[test]
    fn long_pauses_get_a_separator() {
        let items = vec![
            item("a", ItemKind::AssistantText("one".into()), 1000.0),
            item("b", ItemKind::AssistantText("two".into()), 1000.0 + 3600.0),
        ];
        let mut cache = RowCache::new();
        let rows = build_rows(&items, &HashSet::new(), &mut cache, false, false);
        assert!(rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::Caption { rule: true, .. })));
    }
}
