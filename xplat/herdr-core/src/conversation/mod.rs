//! Incremental parser for agent session logs (port of ConversationParser*.swift).
//!
//! Feed lines in order, read `items()`. Understands Claude Code, Codex rollouts, Cursor transcripts and pi sessions;
//! the format is detected per line.

mod grok;
mod cursor;
mod pi;

use crate::model::*;
use crate::question;
use crate::util::{parse_time, pretty_json};
pub use crate::util::cut;
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
struct ToolLabel {
    symbol: &'static str,
    singular: String,
    plural: Option<&'static str>,
}

#[derive(Debug, Clone)]
struct RunEntry {
    tool: String,
    label: ToolLabel,
    id: String,
}

#[derive(Debug, Clone, Default)]
pub struct ConversationParser {
    done: Vec<ConversationItem>,
    run: Vec<RunEntry>,
    run_key: String,
    /// tool_use id -> command text, until a result arrives
    open_tool_uses: HashMap<String, String>,
    /// tool_use id -> the call's `description`, when it has one
    open_details: HashMap<String, String>,
    open_order: Vec<String>,
    /// tool call id -> time of its line
    call_ts: HashMap<String, f64>,
    /// time of the line being fed
    line_ts: Option<f64>,
    /// call details; results patch these after the chip was emitted
    call_by_id: HashMap<String, ToolCall>,
    runs_by_key: HashMap<String, Vec<RunEntry>>,
    /// AskUserQuestion tool_use id -> index in `done`
    question_at: HashMap<String, usize>,
    pi: pi::PiState,
    grok: grok::GrokState,
    /// Counter that gives Codex/Cursor lines an id when no byte offset is known.
    codex_seq: u64,
    /// Counter for Claude lines that carry no uuid.
    anon_seq: u64,
}

/// Parses all lines in one go.
pub fn parse_lines<S: AsRef<str>>(lines: &[S]) -> Vec<ConversationItem> {
    let mut p = ConversationParser::new();
    for l in lines {
        p.feed(l.as_ref());
    }
    p.items()
}

type Obj = Map<String, Value>;

fn s_of<'a>(o: &'a Obj, k: &str) -> Option<&'a str> {
    o.get(k).and_then(Value::as_str)
}

fn obj_array<'a>(v: Option<&'a Value>) -> Option<Vec<&'a Obj>> {
    v?.as_array()?.iter().map(Value::as_object).collect()
}

impl ConversationParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the newest tool call has no result yet. Also true while the tool runs, so it is not proof of a prompt.
    pub fn has_open_tool_use(&self) -> bool {
        !self.open_order.is_empty()
    }

    /// Items so far. The newest tool call without a result is emitted as a `Pending` permission request
    /// (replacing its chip); once the result arrives it goes back into the chips.
    pub fn items(&self) -> Vec<ConversationItem> {
        let mut out: Vec<ConversationItem> = self
            .done
            .iter()
            .map(|item| {
                if let Some(key) = item.id.strip_prefix("tools-") {
                    if let Some(r) = self.runs_by_key.get(key) {
                        return self.chips(r, key, r.first().and_then(|e| self.call_ts.get(&e.id).copied()));
                    }
                }
                item.clone()
            })
            .collect();
        let mut run = self.run.clone();
        let mut perm = None;
        if let Some(last) = self.open_order.last() {
            if let (Some(cmd), Some(i)) = (self.open_tool_uses.get(last), run.iter().rposition(|e| &e.id == last)) {
                let t = run.remove(i);
                perm = Some(ConversationItem {
                    id: format!("perm-{last}"),
                    kind: ItemKind::PermissionRequest(PermissionRequest {
                        title: format!("Wants to {}", verb(&t.tool)),
                        command: cmd.clone(),
                        tool_name: t.tool,
                        tool_use_id: last.clone(),
                        state: PermState::Pending,
                        detail: self.open_details.get(last).cloned().unwrap_or_default(),
                        open_calls: self.open_order.len(),
                    }),
                    timestamp: self.call_ts.get(last).copied(),
                });
            }
        }
        if !run.is_empty() {
            let ts = run.first().and_then(|e| self.call_ts.get(&e.id).copied());
            out.push(self.chips(&run, &self.run_key, ts));
        }
        out.extend(perm);
        out
    }

    pub fn feed(&mut self, line: &str) {
        self.feed_at(line, None);
    }

    /// `offset`: byte offset of the line in its file. Codex and Cursor lines have no uuid, so it gives them ids that
    /// stay stable when earlier lines are prepended.
    pub fn feed_at(&mut self, line: &str, offset: Option<u64>) {
        let Ok(Value::Object(o)) = serde_json::from_str::<Value>(line) else { return };
        self.line_ts = s_of(&o, "timestamp").and_then(parse_time);
        if o.get("isSidechain").and_then(Value::as_bool) == Some(true) { return; }
        if o.get("isMeta").and_then(Value::as_bool) != Some(true) {
            let refs = crate::log_image::refs(&Value::Object(o.clone()), offset);
            if !refs.is_empty() {
                self.flush();
                let id = offset.map(|n| format!("images@{n}")).unwrap_or_else(|| {
                    self.anon_seq += 1;
                    format!("images-{}", self.anon_seq)
                });
                self.done.push(ConversationItem { id, kind: ItemKind::Images(refs.into_iter().map(|(_, r)| r).collect()), timestamp: self.line_ts });
            }
        }
        if grok::is_grok(&o) { self.feed_grok(&o, offset); return; }
        if let Some(Value::Object(payload)) = o.get("payload") {
            self.feed_codex(s_of(&o, "type"), payload, offset);
            return;
        }
        if !o.contains_key("uuid") {
            if let Some(t) = s_of(&o, "type") {
                if pi::PI_TYPES.contains(&t) {
                    self.feed_pi(&o, line);
                    return;
                }
            }
        }
        if cursor::is_cursor(&o) {
            self.feed_cursor(&o, offset);
            return;
        }
        if o.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            return;
        }
        self.feed_claude(&o);
    }

    fn feed_claude(&mut self, o: &Obj) {
        let ty = s_of(o, "type");
        let (true, Some(Value::Object(msg))) = (ty == Some("user") || ty == Some("assistant"), o.get("message")) else { return };
        let uuid = s_of(o, "uuid").map(String::from).unwrap_or_else(|| {
            self.anon_seq += 1;
            format!("line{}", self.anon_seq)
        });
        let meta = o.get("isMeta").and_then(Value::as_bool).unwrap_or(false);
        let compact_flag = o.get("isCompactSummary").and_then(Value::as_bool).unwrap_or(false);

        let blocks: Vec<Obj> = match msg.get("content") {
            Some(Value::String(s)) => {
                let mut b = Obj::new();
                b.insert("type".into(), "text".into());
                b.insert("text".into(), s.clone().into());
                vec![b]
            }
            c => obj_array(c).map(|v| v.into_iter().cloned().collect()).unwrap_or_default(),
        };

        for (i, b) in blocks.iter().enumerate() {
            let bid = format!("{uuid}-{i}");
            match s_of(b, "type") {
                Some("text") => {
                    let text = s_of(b, "text").unwrap_or("").trim();
                    if text.is_empty() {
                        continue;
                    }
                    if ty == Some("user") {
                        if compact_flag || text.starts_with("This session is being continued") {
                            self.flush();
                            self.done.push(ConversationItem { id: bid, kind: ItemKind::Status("Context compacted".into()), timestamp: self.line_ts });
                            continue;
                        }
                        if meta {
                            continue;
                        }
                        let Some(shown) = user_visible(text) else { continue };
                        self.flush();
                        self.done.push(ConversationItem { id: bid, kind: ItemKind::UserText(shown), timestamp: self.line_ts });
                    } else {
                        self.flush();
                        self.done.push(ConversationItem { id: bid, kind: ItemKind::AssistantText(text.to_string()), timestamp: self.line_ts });
                    }
                }
                Some("tool_use") => {
                    if ty != Some("assistant") {
                        continue;
                    }
                    let name = s_of(b, "name").unwrap_or("tool").to_string();
                    let input = b.get("input").and_then(Value::as_object).cloned().unwrap_or_default();
                    let id = s_of(b, "id").map(String::from).unwrap_or_else(|| bid.clone());
                    if name == "AskUserQuestion" {
                        if let Some(q) = question::claude(&id, &input) {
                            // a question card, never a permission
                            self.flush();
                            self.question_at.insert(id.clone(), self.done.len());
                            self.done.push(ConversationItem { id: format!("q-{id}"), kind: ItemKind::Question(q), timestamp: self.line_ts });
                            continue;
                        }
                    }
                    if self.run.is_empty() {
                        self.run_key = bid.clone();
                    }
                    self.run.push(RunEntry { label: label_for(&name), tool: name.clone(), id: id.clone() });
                    self.set_call_ts(&id);
                    self.open_tool_uses.insert(id.clone(), command_text(&name, &input));
                    if name == "Bash" {
                        if let Some(d) = s_of(&input, "description").filter(|d| !d.is_empty()) {
                            self.open_details.insert(id.clone(), d.to_string());
                        }
                    }
                    self.open_order.push(id.clone());
                    self.call_by_id.insert(
                        id.clone(),
                        ToolCall { id, tool_name: name.clone(), summary: summary(&name, &input), input: cut(&pretty_json(&input)), output: None, is_error: false },
                    );
                }
                Some("tool_result") => {
                    let Some(id) = s_of(b, "tool_use_id") else { continue };
                    let is_error = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    if let Some(at) = self.question_at.get(id).copied() {
                        if let ItemKind::Question(q) = &self.done[at].kind {
                            let mut q = q.clone();
                            q.state = question::claude_state(&q, is_error, o.get("toolUseResult"), &result_text(b.get("content")));
                            self.done[at].kind = ItemKind::Question(q);
                            self.question_at.remove(id);
                            continue;
                        }
                    }
                    self.close_call(id, &result_text(b.get("content")), is_error);
                }
                _ => {}
            }
        }
    }

    fn set_call_ts(&mut self, id: &str) {
        match self.line_ts {
            Some(t) => self.call_ts.insert(id.to_string(), t),
            None => self.call_ts.remove(id),
        };
    }

    fn close_call(&mut self, id: &str, output: &str, is_error: bool) {
        if let Some(c) = self.call_by_id.get_mut(id) {
            c.output = Some(cut(output));
            c.is_error = is_error;
        }
        self.open_tool_uses.remove(id);
        self.open_details.remove(id);
        self.open_order.retain(|x| x != id);
    }

    // MARK: Codex rollout lines (`~/.codex/sessions/**/rollout-*.jsonl`)

    fn feed_codex(&mut self, ty: Option<&str>, payload: &Obj, offset: Option<u64>) {
        if ty != Some("response_item") {
            return;
        }
        self.codex_seq += 1;
        let bid = offset.map(|o| format!("cx@{o}")).unwrap_or_else(|| format!("cx{}", self.codex_seq));
        match s_of(payload, "type") {
            Some("message") => {
                let role = s_of(payload, "role");
                if role != Some("user") && role != Some("assistant") {
                    return;
                }
                let parts: Vec<&str> = payload
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|c| c.get("text").and_then(Value::as_str)).collect())
                    .unwrap_or_default();
                let text = parts.join("\n");
                let text = text.trim();
                if text.is_empty() {
                    return;
                }
                self.flush();
                if role == Some("user") {
                    if text.starts_with("# AGENTS.md") {
                        return;
                    }
                    let Some(shown) = user_visible(text) else { return };
                    self.answer_async_questions();
                    self.done.push(ConversationItem { id: bid, kind: ItemKind::UserText(shown), timestamp: self.line_ts });
                } else {
                    self.done.push(ConversationItem { id: bid, kind: ItemKind::AssistantText(text.to_string()), timestamp: self.line_ts });
                }
            }
            Some(pt @ ("custom_tool_call" | "function_call" | "local_shell_call")) => {
                let name = s_of(payload, "name").map(String::from).unwrap_or_else(|| if pt == "local_shell_call" { "shell".into() } else { "tool".into() });
                let id = s_of(payload, "call_id").map(String::from).unwrap_or_else(|| bid.clone());
                if let Some(q) = question::codex(&name, &id, s_of(payload, "arguments").unwrap_or("")) {
                    self.flush();
                    self.question_at.insert(id.clone(), self.done.len());
                    self.done.push(ConversationItem { id: format!("q-{id}"), kind: ItemKind::Question(q), timestamp: self.line_ts });
                    return;
                }
                if self.run.is_empty() {
                    self.run_key = bid.clone();
                }
                self.run.push(RunEntry { label: codex_label(&name), tool: name.clone(), id: id.clone() });
                self.set_call_ts(&id);
                let cmd = codex_command(payload);
                self.open_tool_uses.insert(id.clone(), cmd.clone());
                self.open_order.push(id.clone());
                let raw = s_of(payload, "arguments").or_else(|| s_of(payload, "input")).unwrap_or("");
                self.call_by_id.insert(id.clone(), ToolCall { id, tool_name: name, summary: cmd, input: cut(raw), output: None, is_error: false });
            }
            Some("custom_tool_call_output" | "function_call_output" | "local_shell_call_output") => {
                let Some(id) = s_of(payload, "call_id") else { return };
                let output = result_text(payload.get("output"));
                if let Some(at) = self.question_at.get(id).copied() {
                    if let ItemKind::Question(q) = &self.done[at].kind {
                        let mut q = q.clone();
                        if let Some(state) = question::codex_state(&q, &output) {
                            q.state = state;
                        }
                        let release = q.state != QuestionState::Pending || q.delivery == Delivery::Picker;
                        self.done[at].kind = ItemKind::Question(q);
                        if release {
                            self.question_at.remove(id);
                        }
                        return;
                    }
                }
                self.close_call(id, &output, false);
            }
            _ => {}
        }
    }

    /// A user message answers every open `request_user_input_async` question (its answer is free text).
    fn answer_async_questions(&mut self) {
        let ids: Vec<(String, usize)> = self.question_at.iter().map(|(k, v)| (k.clone(), *v)).collect();
        for (id, at) in ids {
            if let ItemKind::Question(q) = &self.done[at].kind {
                if q.delivery == Delivery::Message {
                    let mut q = q.clone();
                    q.state = QuestionState::Answered(None);
                    self.done[at].kind = ItemKind::Question(q);
                    self.question_at.remove(&id);
                }
            }
        }
    }

    fn flush(&mut self) {
        if self.run.is_empty() {
            return;
        }
        let run = std::mem::take(&mut self.run);
        let ts = run.first().and_then(|e| self.call_ts.get(&e.id).copied());
        let item = self.chips(&run, &self.run_key.clone(), ts);
        self.runs_by_key.insert(self.run_key.clone(), run);
        self.done.push(item);
    }

    fn chips(&self, run: &[RunEntry], key: &str, at: Option<f64>) -> ConversationItem {
        let mut order: Vec<&str> = Vec::new();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        let mut labels: HashMap<&str, &ToolLabel> = HashMap::new();
        let mut groups: HashMap<&str, Vec<ToolCall>> = HashMap::new();
        for r in run {
            let k = r.label.singular.as_str();
            if !counts.contains_key(k) {
                order.push(k);
                labels.insert(k, &r.label);
            }
            *counts.entry(k).or_default() += 1;
            if let Some(c) = self.call_by_id.get(&r.id) {
                groups.entry(k).or_default().push(c.clone());
            }
        }
        let chips = order
            .into_iter()
            .map(|k| {
                let (l, n) = (labels[k], counts[k]);
                let text = match (n > 1, l.plural) {
                    (true, Some(p)) => p.replace("%d", &n.to_string()),
                    _ => l.singular.clone(),
                };
                ToolChip { id: format!("{key}|{k}"), symbol: l.symbol.into(), text, calls: groups.remove(k).unwrap_or_default() }
            })
            .collect();
        ConversationItem { id: format!("tools-{key}"), kind: ItemKind::ToolChips(chips), timestamp: at }
    }
}

// MARK: helpers

fn lbl(symbol: &'static str, singular: &str, plural: Option<&'static str>) -> ToolLabel {
    ToolLabel { symbol, singular: singular.into(), plural }
}

fn codex_label(tool: &str) -> ToolLabel {
    match tool {
        "exec" | "exec_command" | "shell" | "local_shell" | "container.exec" | "write_stdin" => lbl("terminal", "Ran a command", Some("Ran %d commands")),
        "apply_patch" => lbl("pencil", "Edited files", Some("Edited files %d times")),
        "web_search" | "web.run" => lbl("globe", "Searched the web", Some("Searched the web %d times")),
        "update_plan" => lbl("checklist", "Updated the plan", None),
        _ => lbl("wrench", &format!("Used {tool}"), None),
    }
}

fn label_for(tool: &str) -> ToolLabel {
    match tool {
        "Read" => lbl("doc.text", "Read a file", Some("Read %d files")),
        "Edit" | "MultiEdit" | "NotebookEdit" => lbl("pencil", "Edited a file", Some("Edited %d files")),
        "Write" => lbl("square.and.pencil", "Wrote a file", Some("Wrote %d files")),
        "Bash" | "BashOutput" | "Monitor" => lbl("terminal", "Ran a command", Some("Ran %d commands")),
        "Grep" | "Glob" => lbl("magnifyingglass", "Searched the code", Some("Searched the code %d times")),
        "WebFetch" | "WebSearch" => lbl("globe", "Searched the web", Some("Searched the web %d times")),
        "Agent" | "Task" => lbl("person.2", "Delegated a task", Some("Delegated %d tasks")),
        "TodoWrite" => lbl("checklist", "Updated the to-do list", None),
        t if t.starts_with("mcp__") => lbl("puzzlepiece.extension", "Used a connector", Some("Used connectors %d times")),
        t => lbl("wrench", &format!("Used {t}"), None),
    }
}

fn verb(tool: &str) -> String {
    match tool {
        "Bash" => "run a command".into(),
        "Edit" | "MultiEdit" | "NotebookEdit" => "edit a file".into(),
        "Write" => "write a file".into(),
        "Read" => "read a file".into(),
        "WebFetch" | "WebSearch" => "use the web".into(),
        t => format!("use {t}"),
    }
}

fn codex_command(payload: &Obj) -> String {
    if let Some(Ok(Value::Object(o))) = s_of(payload, "arguments").map(serde_json::from_str::<Value>) {
        for k in ["cmd", "command"] {
            match o.get(k) {
                Some(Value::String(s)) if !s.is_empty() => return s.clone(),
                Some(Value::Array(a)) if !a.is_empty() => {
                    if let Some(parts) = a.iter().map(|v| v.as_str()).collect::<Option<Vec<_>>>() {
                        return parts.join(" ");
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(input) = s_of(payload, "input").filter(|s| !s.is_empty()) {
        return input.split('\n').find(|l| !l.is_empty()).unwrap_or("").chars().take(200).collect();
    }
    s_of(payload, "name").unwrap_or("tool").to_string()
}

/// tool_result content: a string, or an array of `{type:text,text}` blocks.
pub(crate) fn result_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

pub(crate) fn summary(name: &str, input: &Obj) -> String {
    let keys: &[&str] = match name {
        "Bash" => &["command"],
        "Read" | "Write" | "Edit" | "MultiEdit" => &["file_path", "path"],
        "NotebookEdit" => &["notebook_path", "file_path"],
        "Task" | "Agent" => &["description", "prompt"],
        _ => &[],
    };
    for k in keys {
        if let Some(s) = s_of(input, k).filter(|s| !s.is_empty()) {
            return s.to_string();
        }
    }
    // serde_json's map is sorted by key, like Swift's `keys.sorted()`.
    for v in input.values() {
        if let Some(s) = v.as_str().filter(|s| !s.is_empty()) {
            return s.to_string();
        }
    }
    name.to_string()
}

pub(crate) fn command_text(name: &str, input: &Obj) -> String {
    for k in ["command", "file_path", "path", "url", "query", "pattern", "description"] {
        if let Some(s) = s_of(input, k).filter(|s| !s.is_empty()) {
            return s.to_string();
        }
    }
    name.to_string()
}

/// Tags whose text is the user's own. Claude Code wraps pasted text in `<pasted_content id="…">`. Any other text
/// that starts with a tag (`<system-reminder>`, `<command-name>`, `<task-notification>`, …) is injected context.
const VISIBLE_TAGS: [&str; 1] = ["pasted_content"];

/// The text to show for a user message, or `None` when it is injected context. Pasted blocks lose their wrapper
/// tags; typed text and pasted blocks keep their order, separated by a blank line.
pub fn user_visible(text: &str) -> Option<String> {
    static TAG: OnceLock<Regex> = OnceLock::new();
    if text.starts_with('<') {
        let tag = TAG.get_or_init(|| Regex::new(r"^<([A-Za-z_][\w-]*)").unwrap());
        let m = tag.captures(text)?;
        if !VISIBLE_TAGS.contains(&&m[1]) {
            return None;
        }
    } else if is_injected(text) {
        return None;
    }
    if !text.contains("<pasted_content") {
        return Some(text.to_string());
    }
    let out = unwrap_pasted(text);
    (!out.is_empty()).then_some(out)
}

/// Splits text into typed parts and pasted blocks. A block ends at the closing tag with the same id (or the
/// plain closing tag without an id), so pasted text that itself quotes these tags stays as it is.
fn unwrap_pasted(text: &str) -> String {
    static OPEN: OnceLock<Regex> = OnceLock::new();
    let open_re = OPEN.get_or_init(|| Regex::new(r#"<pasted_content(?: id="([^"]*)")?>"#).unwrap());
    let mut parts: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(open) = open_re.captures(rest) {
        let whole = open.get(0).unwrap();
        parts.push(&rest[..whole.start()]);
        let close = match open.get(1) {
            Some(id) => format!("</pasted_content id=\"{}\">", id.as_str()),
            None => "</pasted_content>".to_string(),
        };
        let body = &rest[whole.end()..];
        if let Some(c) = body.find(&close) {
            parts.push(&body[..c]);
            rest = &body[c + close.len()..];
        } else {
            parts.push(body);
            rest = "";
        }
    }
    parts.push(rest);
    parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n\n")
}

fn is_injected(text: &str) -> bool {
    text.starts_with('<') || text.starts_with("[Request interrupted") || text.starts_with("Caveat:")
}
