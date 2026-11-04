//! Cursor agent (`~/.cursor/projects/<slug>/agent-transcripts/<id>/<id>.jsonl`). Lines are `{role, message:{content}}`
//! and `{type:"turn_ended", status}`; no ids, no times, no tool results. The real prompt is inside `<user_query>`;
//! user lines without one are injected context. Tool calls become chips without output (results live only in
//! Cursor's store.db) and never a pending item.

use std::sync::OnceLock;

use chrono::NaiveDate;
use regex::Regex;

use crate::util::json::{self, Obj};
use super::{cut, label, pretty_json, summary, text, ConversationParser, RunEntry};
use crate::model::{ConversationItem, ItemKind, ToolCall};

pub fn is_cursor(o: &Obj) -> bool {
    !o.contains_key("uuid")
        && ((!o.contains_key("type") && json::str_(o.get("role")).is_some() && o.contains_key("message"))
            || json::str_(o.get("type")) == Some("turn_ended"))
}

/// The typed prompt: the `<user_query>` body; plain text without tags as is; other tagged text is injected context.
pub fn cursor_query(raw: &str) -> Option<String> {
    if let Some(a) = raw.find("<user_query>") {
        let rest = &raw[a + "<user_query>".len()..];
        let body = rest.find("</user_query>").map(|e| &rest[..e]).unwrap_or(rest);
        return Some(text::trim(body).to_string());
    }
    let t = text::trim(raw);
    if text::has_prefix(t, "<") || text::has_prefix(t, "[") {
        None
    } else {
        Some(t.to_string())
    }
}

fn time_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"<timestamp>\w+, (\w{3}) (\d{1,2}), (\d{4}), (\d{1,2}):(\d{2}) (AM|PM) \(UTC(?:([+-])(\d{1,2})(?::(\d{2}))?)?\)</timestamp>",
        )
        .unwrap()
    })
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `<timestamp>Saturday, Sep 12, 2026, 4:09 PM (UTC+8)</timestamp>` (minute precision). Out-of-range fields roll
/// over, as `Calendar.date(from:)` does.
pub fn cursor_time(raw: &str) -> Option<f64> {
    let m = time_re().captures(raw)?;
    let month = MONTHS.iter().position(|x| *x == &m[1])?;
    let day: i64 = m[2].parse().ok()?;
    let year: i32 = m[3].parse().ok()?;
    let mut hour: i64 = m[4].parse().ok()?;
    let minute: i64 = m[5].parse().ok()?;
    if hour == 12 {
        hour = 0;
    }
    if &m[6] == "PM" {
        hour += 12;
    }
    let hours: i64 = m.get(8).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
    let mins: i64 = m.get(9).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
    let sign: i64 = if m.get(7).map(|x| x.as_str()) == Some("-") { -1 } else { 1 };
    let off = (hours * 3600 + mins * 60) * sign;
    if off.abs() > 18 * 3600 {
        return None;
    }
    let first = NaiveDate::from_ymd_opt(year, month as u32 + 1, 1)?.and_hms_opt(0, 0, 0)?;
    let local = first
        + chrono::Duration::days(day - 1)
        + chrono::Duration::hours(hour)
        + chrono::Duration::minutes(minute)
        - chrono::Duration::seconds(off);
    Some(local.and_utc().timestamp() as f64)
}

/// Cursor's tools under Claude Code's names, so chips read the same.
pub fn cursor_tool(name: &str, input: &Obj) -> String {
    match name {
        "Shell" | "AwaitShell" => "Bash".into(),
        "StrReplace" => "Edit".into(),
        "Task" => "Agent".into(),
        "CallMcpTool" | "CallDynamicTool" => format!(
            "mcp__{}__{}",
            json::str_(input.get("server")).or(json::str_(input.get("namespace"))).unwrap_or("cursor"),
            json::str_(input.get("toolName")).unwrap_or("tool")
        ),
        _ => name.into(),
    }
}

impl ConversationParser {
    pub(super) fn feed_cursor(&mut self, o: &Obj, offset: Option<u64>) {
        self.codex_seq += 1;
        let bid = offset.map(|o| format!("cu@{o}")).unwrap_or_else(|| format!("cu{}", self.codex_seq));
        if json::str_(o.get("type")) == Some("turn_ended") {
            let status = json::str_(o.get("status"));
            if status != Some("error") && status != Some("aborted") {
                return;
            }
            self.flush();
            let s = if status == Some("error") { "Error" } else { "Interrupted" };
            self.done.push(ConversationItem::new(&bid, ItemKind::Status(s.into()), self.line_ts));
            return;
        }
        let role = json::str_(o.get("role"));
        let blocks = json::objs(json::obj(o.get("message")).and_then(|m| m.get("content"))).unwrap_or_default();
        for (i, b) in blocks.iter().enumerate() {
            let id = format!("{bid}-{i}");
            match json::str_(b.get("type")) {
                Some("text") => {
                    let raw = json::str_(b.get("text")).unwrap_or("");
                    if role == Some("user") {
                        let Some(t) = cursor_query(raw) else { continue };
                        if t.is_empty() {
                            continue;
                        }
                        self.line_ts = cursor_time(raw).or(self.line_ts);
                        self.flush();
                        self.done.push(ConversationItem::new(&id, ItemKind::UserText(t), self.line_ts));
                    } else if role == Some("assistant") {
                        let t = text::trim(raw);
                        if t.is_empty() {
                            continue;
                        }
                        self.flush();
                        self.done.push(ConversationItem::new(&id, ItemKind::AssistantText(t.into()), None));
                    }
                }
                Some("tool_use") => {
                    if role != Some("assistant") {
                        continue;
                    }
                    let empty = Obj::new();
                    let input = json::obj(b.get("input")).unwrap_or(&empty);
                    let name = cursor_tool(json::str_(b.get("name")).unwrap_or("tool"), input);
                    if self.run.is_empty() {
                        self.run_key = id.clone();
                    }
                    self.run.push(RunEntry { tool: name.clone(), label: label(&name), id: id.clone() });
                    self.call_by_id
                        .insert(id.clone(), ToolCall::new(&id, &name, &summary(&name, input), &cut(&pretty_json(input))));
                }
                _ => {}
            }
        }
    }
}
