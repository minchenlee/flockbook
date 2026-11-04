//! Swift: `Rows.swift`. What the mod draws, one row each: the conversation items, plus time separators and one
//! footer per agent turn, as the app's conversation view makes them.

use chrono::{Datelike, Duration, TimeZone, Timelike};
use herdr_core::util::Date;
use herdr_core::*;
use serde::Serialize;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FeedRow {
    /// Always "row".
    #[serde(rename = "type")]
    pub kind_tag: &'static str,
    /// "user", "assistant", "tools", "perm", "question", "status", "sep", "footer".
    pub k: String,
    /// Stable across reads of the same log.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Milliseconds since 1970 (Swift encodes a whole Double without a fraction).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chips: Option<Vec<FeedChip>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perm: Option<FeedPerm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<FeedQuestion>,
}

impl FeedRow {
    pub fn new(k: &str, id: &str, text: Option<String>, ts: Option<i64>) -> FeedRow {
        FeedRow { kind_tag: "row", k: k.into(), id: id.into(), text, ts, images: None, chips: None, perm: None, question: None }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FeedChip {
    pub symbol: String,
    pub text: String,
    pub calls: Vec<FeedCall>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FeedCall {
    pub name: String,
    pub summary: String,
    pub done: bool,
    pub error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FeedPerm {
    pub title: String,
    pub command: String,
    pub tool: String,
    pub tool_use_id: String,
    pub state: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FeedOption {
    pub label: String,
    pub detail: String,
    pub recommended: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FeedQuestionItem {
    pub id: String,
    pub header: String,
    pub text: String,
    pub multi: bool,
    pub options: Vec<FeedOption>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FeedQuestion {
    pub tool_use_id: String,
    pub agent: String,
    /// "picker" or "message".
    pub delivery: String,
    /// "pending", "answered", "declined".
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<Vec<String>>,
    pub questions: Vec<FeedQuestionItem>,
}

/// Longer text is cut (a trailing "…" marks the cut).
pub const TEXT_CAP: usize = 20_000;
pub const OUTPUT_CAP: usize = 400;
/// A gap this long between rows (or a new day) gets a time separator.
pub const SEPARATOR_GAP_S: i64 = 900;

/// Swift `s.count > n ? prefix(n) + "…" : s`, counted in grapheme clusters.
pub fn cut(s: &str, n: usize) -> String {
    let mut it = s.grapheme_indices(true);
    match it.nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Swift: (timeIntervalSince1970 * 1000).rounded()
pub fn ms(t: Option<f64>) -> Option<i64> {
    t.map(|t| (t * 1000.0).round() as i64)
}

/// A log time (seconds since 1970) as a date, to the microsecond.
pub fn date(t: f64) -> Date {
    let us = (t * 1_000_000.0).round() as i64;
    chrono::DateTime::<chrono::Utc>::from_timestamp_micros(us).unwrap_or_default()
}

/// Adjacent tool items become one row, keyed by the first one's id.
pub fn merged(items: &[ConversationItem]) -> Vec<ConversationItem> {
    let mut out: Vec<ConversationItem> = vec![];
    let mut run: Vec<ConversationItem> = vec![];
    fn flush(run: &mut Vec<ConversationItem>, out: &mut Vec<ConversationItem>) {
        let Some(first) = run.first().cloned() else { return };
        if run.len() == 1 {
            out.push(first);
        } else {
            let chips: Vec<ToolChip> = run
                .iter()
                .flat_map(|r| if let ItemKind::ToolChips(c) = &r.kind { c.clone() } else { vec![] })
                .collect();
            out.push(ConversationItem { id: first.id.clone(), kind: ItemKind::ToolChips(chips), timestamp: first.timestamp, images: vec![] });
        }
        run.clear();
    }
    for it in items {
        if matches!(it.kind, ItemKind::ToolChips(_)) {
            run.push(it.clone());
        } else {
            flush(&mut run, &mut out);
            out.push(it.clone());
        }
    }
    flush(&mut run, &mut out);
    out
}

pub fn question(q: &QuestionRequest) -> FeedQuestion {
    let (state, answers) = match &q.state {
        QuestionState::Pending => ("pending", None),
        QuestionState::Answered(a) => ("answered", a.clone()),
        QuestionState::Declined => ("declined", None),
    };
    FeedQuestion {
        tool_use_id: q.tool_use_id.clone(),
        agent: q.agent.clone(),
        delivery: if q.delivery == Delivery::Picker { "picker" } else { "message" }.into(),
        state: state.into(),
        answers,
        questions: q
            .questions
            .iter()
            .map(|x| FeedQuestionItem {
                id: x.id.clone(),
                header: x.header.clone(),
                text: x.text.clone(),
                multi: x.multi_select,
                options: x
                    .options
                    .iter()
                    .map(|o| FeedOption { label: o.label.clone(), detail: o.detail.clone(), recommended: o.recommended })
                    .collect(),
            })
            .collect(),
    }
}

pub fn row(it: &ConversationItem) -> FeedRow {
    let mut r = FeedRow::new("status", &it.id, None, ms(it.timestamp));
    match &it.kind {
        ItemKind::UserText(s) => {
            r.k = "user".into();
            r.text = Some(cut(s, TEXT_CAP));
            if !it.images.is_empty() {
                r.images = Some(it.images.len());
            }
        }
        ItemKind::AssistantText(s) => {
            r.k = "assistant".into();
            r.text = Some(cut(s, TEXT_CAP));
        }
        ItemKind::Status(s) => r.text = Some(s.clone()),
        ItemKind::ToolChips(chips) => {
            r.k = "tools".into();
            r.chips = Some(
                chips
                    .iter()
                    .map(|c| FeedChip {
                        symbol: c.symbol.clone(),
                        text: c.text.clone(),
                        calls: c
                            .calls
                            .iter()
                            .map(|x| FeedCall {
                                name: x.tool_name.clone(),
                                summary: cut(&x.summary, 300),
                                done: x.output.is_some(),
                                error: x.is_error,
                                output: x.output.as_ref().map(|o| cut(o, OUTPUT_CAP)),
                            })
                            .collect(),
                    })
                    .collect(),
            );
        }
        ItemKind::PermissionRequest(p) => {
            r.k = "perm".into();
            r.text = Some(p.title.clone());
            r.perm = Some(FeedPerm {
                title: p.title.clone(),
                command: cut(&p.command, TEXT_CAP),
                tool: p.tool_name.clone(),
                tool_use_id: p.tool_use_id.clone(),
                state: if p.state == PermState::Pending { "pending" } else { "resolved" }.into(),
            });
        }
        ItemKind::Question(q) => {
            r.k = "question".into();
            r.question = Some(question(q));
        }
    }
    r
}

fn secs(a: &Date, b: &Date) -> f64 {
    (*a - *b).num_microseconds().unwrap_or(i64::MAX) as f64 / 1e6
}

/// Rows with time separators and turn footers ("01:41 · Worked 36s · 3 tools"). `tz`: the calendar's time zone.
pub fn rows<Tz: TimeZone>(items: &[ConversationItem], now: &Date, tz: &Tz) -> Vec<FeedRow> {
    let mut out: Vec<FeedRow> = vec![];
    let mut prev_time: Option<Date> = None;
    let mut prev_row: Option<ConversationItem> = None;
    let (mut turn_start, mut turn_tools, mut turn_has_reply, mut turn_end): (Option<Date>, usize, bool, Option<Date>) = (None, 0, false, None);
    let close_turn = |out: &mut Vec<FeedRow>, turn_has_reply: bool, turn_end: Option<Date>, turn_start: Option<Date>, turn_tools: usize, prev_row: &Option<ConversationItem>| {
        let (true, Some(end), Some(last)) = (turn_has_reply, turn_end, prev_row) else { return };
        let mut parts = vec![clock(&end, tz)];
        if let Some(s) = turn_start {
            let d = secs(&end, &s);
            if d >= 1.0 {
                parts.push(format!("Worked {}", duration(d)));
            }
        }
        if turn_tools > 0 {
            parts.push(format!("{turn_tools} tool{}", if turn_tools == 1 { "" } else { "s" }));
        }
        out.push(FeedRow::new("footer", &format!("f-{}", last.id), Some(parts.join(" · ")), None));
    };
    for it in merged(items) {
        if matches!(it.kind, ItemKind::UserText(_)) {
            close_turn(&mut out, turn_has_reply, turn_end, turn_start, turn_tools, &prev_row);
            turn_start = it.timestamp.map(date);
            turn_tools = 0;
            turn_has_reply = false;
            turn_end = None;
        }
        if let Some(t) = it.timestamp.map(date) {
            if let Some(p) = prev_time {
                if secs(&t, &p) >= SEPARATOR_GAP_S as f64 || !same_day(&t, &p, tz) {
                    out.push(FeedRow::new("sep", &format!("s-{}", it.id), Some(separator(&t, now, tz)), None));
                }
            }
            prev_time = Some(t);
            turn_end = Some(t);
        }
        if matches!(it.kind, ItemKind::AssistantText(_)) {
            turn_has_reply = true;
        }
        if let ItemKind::ToolChips(c) = &it.kind {
            turn_tools += c.iter().map(|x| x.calls.len().max(1)).sum::<usize>();
        }
        out.push(row(&it));
        prev_row = Some(it);
    }
    close_turn(&mut out, turn_has_reply, turn_end, turn_start, turn_tools, &prev_row);
    out
}

fn same_day<Tz: TimeZone>(a: &Date, b: &Date, tz: &Tz) -> bool {
    a.with_timezone(tz).date_naive() == b.with_timezone(tz).date_naive()
}

pub fn clock<Tz: TimeZone>(d: &Date, tz: &Tz) -> String {
    let l = d.with_timezone(tz);
    format!("{:02}:{:02}", l.hour(), l.minute())
}

/// Swift `Int(s.rounded())`: half away from zero.
pub fn duration(s: f64) -> String {
    let n = s.round() as i64;
    if n >= 3600 {
        format!("{}h {}m", n / 3600, n % 3600 / 60)
    } else if n >= 60 {
        format!("{}m {}s", n / 60, n % 60)
    } else {
        format!("{n}s")
    }
}

pub fn separator<Tz: TimeZone>(d: &Date, now: &Date, tz: &Tz) -> String {
    let day = d.with_timezone(tz).date_naive();
    let today = now.with_timezone(tz).date_naive();
    if day == today {
        return clock(d, tz);
    }
    if today.checked_sub_signed(Duration::days(1)) == Some(day) {
        return format!("Yesterday {}", clock(d, tz));
    }
    // "EEE d MMM", en_US_POSIX.
    const WD: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MO: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{} {} {} {}",
        WD[day.weekday().num_days_from_monday() as usize],
        day.day(),
        MO[day.month0() as usize],
        clock(d, tz)
    )
}
