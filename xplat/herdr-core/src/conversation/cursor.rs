//! Cursor agent (`~/.cursor/projects/<slug>/agent-transcripts/<id>/<id>.jsonl`). Lines are `{role, message:{content}}`
//! and `{type:"turn_ended", status}`; no ids, no times, no tool results. The real prompt is inside `<user_query>`;
//! user lines without one are injected context. Tool calls become chips without output and never a pending item.

use super::*;
use crate::util::days_from_civil;

pub(super) fn is_cursor(o: &Obj) -> bool {
    !o.contains_key("uuid")
        && ((!o.contains_key("type") && o.get("role").is_some_and(Value::is_string) && o.contains_key("message")) || s_of(o, "type") == Some("turn_ended"))
}

impl ConversationParser {
    pub(super) fn feed_cursor(&mut self, o: &Obj, offset: Option<u64>) {
        self.codex_seq += 1;
        let bid = offset.map(|x| format!("cu@{x}")).unwrap_or_else(|| format!("cu{}", self.codex_seq));
        if s_of(o, "type") == Some("turn_ended") {
            let status = s_of(o, "status");
            if status != Some("error") && status != Some("aborted") {
                return;
            }
            self.flush();
            let text = if status == Some("error") { "Error" } else { "Interrupted" };
            self.done.push(ConversationItem { id: bid, kind: ItemKind::Status(text.into()), timestamp: self.line_ts, ..Default::default() });
            return;
        }
        let role = s_of(o, "role");
        let blocks = obj_array(o.get("message").and_then(|m| m.get("content"))).unwrap_or_default();
        for (i, b) in blocks.iter().enumerate() {
            let id = format!("{bid}-{i}");
            match s_of(b, "type") {
                Some("text") => {
                    let raw = s_of(b, "text").unwrap_or("");
                    if role == Some("user") {
                        let Some(text) = cursor_query(raw).filter(|t| !t.is_empty()) else { continue };
                        self.line_ts = cursor_time(raw).or(self.line_ts);
                        self.flush();
                        self.done.push(ConversationItem { id, kind: ItemKind::UserText(text), timestamp: self.line_ts, ..Default::default() });
                    } else if role == Some("assistant") {
                        let text = raw.trim();
                        if text.is_empty() {
                            continue;
                        }
                        self.flush();
                        self.done.push(ConversationItem { id, kind: ItemKind::AssistantText(text.to_string()), timestamp: None, ..Default::default() });
                    }
                }
                Some("tool_use") => {
                    if role != Some("assistant") {
                        continue;
                    }
                    let input = b.get("input").and_then(Value::as_object).cloned().unwrap_or_default();
                    let name = cursor_tool(s_of(b, "name").unwrap_or("tool"), &input);
                    if self.run.is_empty() {
                        self.run_key = id.clone();
                    }
                    self.run.push(RunEntry { label: label_for(&name), tool: name.clone(), id: id.clone() });
                    let call = ToolCall { id: id.clone(), tool_name: name.clone(), summary: summary(&name, &input), input: cut(&pretty_json(&input)), output: None, is_error: false, ..Default::default() };
                    self.call_by_id.insert(id, call);
                }
                _ => {}
            }
        }
    }
}

/// The typed prompt: the `<user_query>` body; plain text without tags as is; other tagged text is injected context.
fn cursor_query(raw: &str) -> Option<String> {
    if let Some(a) = raw.find("<user_query>") {
        let rest = &raw[a + "<user_query>".len()..];
        let body = rest.find("</user_query>").map_or(rest, |c| &rest[..c]);
        return Some(body.trim().to_string());
    }
    let t = raw.trim();
    (!(t.starts_with('<') || t.starts_with('['))).then(|| t.to_string())
}

/// `<timestamp>Saturday, Sep 12, 2026, 4:09 PM (UTC+8)</timestamp>` (minute precision).
fn cursor_time(raw: &str) -> Option<f64> {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| {
        Regex::new(r"<timestamp>\w+, (\w{3}) (\d{1,2}), (\d{4}), (\d{1,2}):(\d{2}) (AM|PM) \(UTC(?:([+-])(\d{1,2})(?::(\d{2}))?)?\)</timestamp>").unwrap()
    });
    let m = re.captures(raw)?;
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let month = MONTHS.iter().position(|x| *x == &m[1])? as i64 + 1;
    let (day, year): (i64, i64) = (m[2].parse().ok()?, m[3].parse().ok()?);
    let (mut hour, minute): (i64, i64) = (m[4].parse().ok()?, m[5].parse().ok()?);
    if hour == 12 {
        hour = 0;
    }
    if &m[6] == "PM" {
        hour += 12;
    }
    let hours: i64 = m.get(8).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
    let mins: i64 = m.get(9).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
    let sign = if m.get(7).map(|x| x.as_str()) == Some("-") { -1 } else { 1 };
    let off = (hours * 3600 + mins * 60) * sign;
    Some((days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 - off) as f64)
}

/// Cursor's tools under Claude Code's names, so chips read the same.
fn cursor_tool(name: &str, input: &Obj) -> String {
    match name {
        "Shell" | "AwaitShell" => "Bash".into(),
        "StrReplace" => "Edit".into(),
        "Task" => "Agent".into(),
        "CallMcpTool" | "CallDynamicTool" => {
            let server = s_of(input, "server").or_else(|| s_of(input, "namespace")).unwrap_or("cursor");
            format!("mcp__{server}__{}", s_of(input, "toolName").unwrap_or("tool"))
        }
        n => n.into(),
    }
}
