//! Incremental parser for agent session logs (Swift: `ConversationParser` and its extensions), ported 1:1 from
//! `rust/herdrkit`'s parser. Claude Code (with its queue log) and Codex here; pi, Grok Build, agy and Cursor in their
//! own files. The format is detected per line. Feed lines in order; read `items()`.
//!
//! Beyond the Swift port, a pending `PermissionRequest` carries `detail` (Claude's Bash `description`) and
//! `open_calls` (how many calls still wait for a result); neither changes what the Swift port emits otherwise.

mod agy;
mod cursor;
mod grok;
mod pi;

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::log_image;
use crate::model::{ConversationItem, Delivery, ItemKind, PermState, PermissionRequest, QuestionState, ToolCall, ToolChip};
use crate::question as questions;
use crate::util::json::{self, Obj};
use crate::util::{parse_time, pretty_json, text};
use agy::AgyState;
use grok::GrokState;
use pi::PiState;

pub use crate::util::{cut, MAX_DETAIL};

/// Parses all lines in one go.
pub fn parse_lines<S: AsRef<str>>(lines: &[S]) -> Vec<ConversationItem> {
    let mut p = ConversationParser::new();
    for l in lines {
        p.feed(l.as_ref());
    }
    p.items()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolLabel {
    pub symbol: String,
    pub singular: String,
    pub plural: Option<String>,
}

impl ToolLabel {
    fn new(symbol: &str, singular: &str, plural: Option<&str>) -> ToolLabel {
        ToolLabel { symbol: symbol.into(), singular: singular.into(), plural: plural.map(Into::into) }
    }
}

/// One call of an open tool run.
#[derive(Clone, Debug)]
struct RunEntry {
    tool: String,
    label: ToolLabel,
    id: String,
}

/// Claude Code's queue from its session log: `enqueue` adds (its `content`, or none during compaction), `remove`
/// takes the message the agent read mid-turn, `dequeue` the oldest at a turn's end, `popAll` the ones the owner pulled
/// back with Up.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ClaudeQueueLog {
    pub items: Vec<Option<String>>,
}

impl ClaudeQueueLog {
    pub fn new(items: Vec<Option<String>>) -> ClaudeQueueLog {
        ClaudeQueueLog { items }
    }

    pub fn feed(&mut self, o: &Obj) {
        if json::str_(o.get("type")) != Some("queue-operation") {
            return;
        }
        // As written: a pull compares it with the agent's input box character for character.
        let content = json::str_(o.get("content")).map(str::to_string);
        match json::str_(o.get("operation")) {
            Some("enqueue") => self.items.push(content),
            Some("dequeue") => {
                if !self.items.is_empty() {
                    self.items.remove(0);
                }
            }
            Some("remove") | Some("popAll") => {
                if let Some(i) =
                    self.items.iter().position(|x| *x == content).or_else(|| self.items.iter().position(Option::is_none))
                {
                    self.items.remove(i);
                } else if content.is_none() && !self.items.is_empty() {
                    self.items.remove(0);
                }
            }
            _ => {}
        }
    }
}

/// Incremental parser for Claude Code session JSONL. Feed lines in order; read `items`.
#[derive(Clone, Debug, Default)]
pub struct ConversationParser {
    done: Vec<ConversationItem>,
    run: Vec<RunEntry>, // open tool run
    run_key: String,
    open_tool_uses: HashMap<String, String>, // tool_use id -> command text, until a result arrives
    open_details: HashMap<String, String>,   // tool_use id -> the call's one-line description, when it has one
    open_order: Vec<String>,
    call_ts: HashMap<String, Option<f64>>,  // tool call id -> time of its line
    line_ts: Option<f64>,                   // time of the line being fed
    line_offset: Option<u64>,               // byte offset of the line being fed (picture refs point at it)
    call_by_id: HashMap<String, ToolCall>,  // call details; results patch these after the chip was emitted
    runs_by_key: HashMap<String, Vec<RunEntry>>,
    question_at: HashMap<String, usize>, // AskUserQuestion tool_use id -> index in `done`
    continued_in: Option<String>,
    thought: Option<(String, Option<String>, String)>, // Claude thinking with words, until its message's next block
    pi: PiState,     // pi entry tree (see pi.rs)
    agy: AgyState,   // agy result slots (see agy.rs)
    grok: GrokState, // Grok Build chunk merging (see grok.rs)
    queue: ClaudeQueueLog,
    codex_seq: u64,
}

impl ConversationParser {
    pub fn new() -> ConversationParser {
        ConversationParser::default()
    }

    /// The session the last line fed says the conversation went on in (Claude Code's `continued-in`), else nil.
    pub fn continued_in(&self) -> Option<&str> {
        self.continued_in.as_deref()
    }

    /// Claude Code's queue of messages typed while it works (`queue-operation` lines).
    pub fn queue(&self) -> &ClaudeQueueLog {
        &self.queue
    }

    /// True while the newest tool call has no result yet. Also true while the tool runs, so it is not proof of a prompt.
    pub fn has_open_tool_use(&self) -> bool {
        !self.open_order.is_empty()
    }

    fn ts(&self, id: Option<&RunEntry>) -> Option<f64> {
        id.and_then(|r| self.call_ts.get(&r.id).copied().flatten())
    }

    /// Items so far. The newest tool call without a result is emitted as a `pending` permissionRequest
    /// (replacing its chip); once the result arrives it goes back into the chips.
    pub fn items(&self) -> Vec<ConversationItem> {
        let mut out: Vec<ConversationItem> = self
            .done
            .iter()
            .map(|item| {
                if let Some(key) = item.id.strip_prefix("tools-") {
                    if let Some(r) = self.runs_by_key.get(key) {
                        return chips(r, key, &self.call_by_id, self.ts(r.first()));
                    }
                }
                item.clone()
            })
            .collect();
        let mut run = self.run.clone();
        let mut perm = None;
        if let Some(last) = self.open_order.last() {
            if let Some(cmd) = self.open_tool_uses.get(last) {
                if let Some(i) = run.iter().rposition(|r| &r.id == last) {
                    let t = run.remove(i);
                    perm = Some(ConversationItem::new(
                        &format!("perm-{last}"),
                        ItemKind::PermissionRequest(PermissionRequest {
                            title: format!("Wants to {}", verb(&t.tool)),
                            command: cmd.clone(),
                            tool_name: t.tool,
                            tool_use_id: last.clone(),
                            state: PermState::Pending,
                            detail: self.open_details.get(last).cloned().unwrap_or_default(),
                            open_calls: self.open_order.len(),
                        }),
                        self.call_ts.get(last).copied().flatten(),
                    ));
                }
            }
        }
        if !run.is_empty() {
            out.push(chips(&run, &self.run_key, &self.call_by_id, self.ts(run.first())));
        }
        if let Some(p) = perm {
            out.push(p);
        }
        out
    }

    /// Swift `feed(line:)`: a line without its offset.
    pub fn feed(&mut self, line: &str) {
        self.feed_at(line, None);
    }

    /// `offset`: byte offset of the line in its file. Codex lines have no uuid, so it gives them ids that stay stable
    /// when earlier lines are prepended.
    pub fn feed_at(&mut self, line: &str, offset: Option<u64>) {
        let Some(o) = json::parse_obj(line) else { return };
        self.line_ts = json::str_(o.get("timestamp")).and_then(parse_time);
        self.line_offset = offset;
        if let Some((update, meta)) = grok::grok_update(&o) {
            self.feed_grok(update, meta, &o, offset);
            return;
        }
        if let Some(payload) = json::obj(o.get("payload")) {
            self.feed_codex(json::str_(o.get("type")), payload, offset);
            return;
        }
        if agy::is_agy(&o) {
            self.feed_agy(&o);
            return;
        }
        if !o.contains_key("uuid") {
            if let Some(t) = json::str_(o.get("type")) {
                if pi::PI_TYPES.contains(&t) {
                    self.feed_pi(&o, line, offset);
                    return;
                }
            }
        }
        if cursor::is_cursor(&o) {
            self.feed_cursor(&o, offset);
            return;
        }
        let ty = json::str_(o.get("type"));
        self.continued_in =
            if ty == Some("continued-in") { json::str_(o.get("continuedInSessionId")).map(str::to_string) } else { None };
        if json::bool_(o.get("isSidechain")) == Some(true) {
            return;
        }
        if ty == Some("attachment") {
            self.feed_queued(&o);
            return;
        }
        if ty == Some("queue-operation") {
            self.queue.feed(&o);
            return;
        }
        if ty != Some("user") && ty != Some("assistant") {
            return;
        }
        let Some(msg) = json::obj(o.get("message")) else { return };
        let is_user = ty == Some("user");
        let uuid = json::str_(o.get("uuid")).map(str::to_string).unwrap_or_else(random_uuid);
        let meta = json::bool_(o.get("isMeta")).unwrap_or(false);
        let compact_flag = json::bool_(o.get("isCompactSummary")).unwrap_or(false);

        let string_block: Obj;
        let blocks: Vec<&Obj> = if let Some(s) = json::str_(msg.get("content")) {
            let mut b = Obj::new();
            b.insert("type".into(), Value::from("text"));
            b.insert("text".into(), Value::from(s));
            string_block = b;
            vec![&string_block]
        } else {
            json::objs(msg.get("content")).unwrap_or_default()
        };
        let pics = if is_user && !meta && !compact_flag {
            log_image::refs(&log_image::found_message(msg), offset)
        } else {
            vec![]
        };
        let first_new = self.done.len();

        let message_id = json::str_(msg.get("id")).map(str::to_string);
        for (i, b) in blocks.iter().enumerate() {
            let bid = format!("{uuid}-{i}");
            let kind = json::str_(b.get("type"));
            // Thinking stays hidden, except words logged as thinking right before a tool call of the same message: the
            // terminal shows those as the reply's text.
            if kind == Some("thinking") || kind == Some("redacted_thinking") {
                let words = text::trim(json::str_(b.get("thinking")).unwrap_or(""));
                if !is_user && !words.is_empty() {
                    self.thought = Some((bid, message_id.clone(), words.to_string()));
                }
                continue;
            }
            if let Some((tid, tmsg, ttext)) = self.thought.take() {
                if kind == Some("tool_use") && !is_user && tmsg == message_id {
                    self.flush();
                    self.done.push(ConversationItem::new(&tid, ItemKind::AssistantText(ttext), self.line_ts));
                }
            }
            match kind {
                Some("text") => {
                    let t = text::trim(json::str_(b.get("text")).unwrap_or(""));
                    if t.is_empty() {
                        continue;
                    }
                    if is_user {
                        if compact_flag || text::has_prefix(t, "This session is being continued") {
                            self.flush();
                            self.done.push(ConversationItem::new(
                                &bid,
                                ItemKind::Status("Context compacted".into()),
                                self.line_ts,
                            ));
                            continue;
                        }
                        if meta {
                            continue;
                        }
                        let Some(shown) = user_visible(t) else { continue };
                        self.flush();
                        self.done.push(ConversationItem::new(&bid, ItemKind::UserText(shown), self.line_ts));
                    } else {
                        self.flush();
                        self.done.push(ConversationItem::new(&bid, ItemKind::AssistantText(t.to_string()), self.line_ts));
                    }
                }
                Some("tool_use") => {
                    if is_user {
                        continue;
                    }
                    let name = json::str_(b.get("name")).unwrap_or("tool");
                    let empty = Obj::new();
                    let input = json::obj(b.get("input")).unwrap_or(&empty);
                    let id = json::str_(b.get("id")).map(str::to_string).unwrap_or_else(|| bid.clone());
                    if name == "AskUserQuestion" {
                        if let Some(q) = questions::claude(&id, input) {
                            // a question card, never a permission
                            self.flush();
                            self.question_at.insert(id.clone(), self.done.len());
                            self.done.push(ConversationItem::new(&format!("q-{id}"), ItemKind::Question(q), self.line_ts));
                            continue;
                        }
                    }
                    if self.run.is_empty() {
                        self.run_key = bid.clone();
                    }
                    self.run.push(RunEntry { tool: name.into(), label: label(name), id: id.clone() });
                    self.call_ts.insert(id.clone(), self.line_ts);
                    self.open_tool_uses.insert(id.clone(), command_text(name, input));
                    if name == "Bash" {
                        if let Some(d) = json::str_(input.get("description")).filter(|d| !d.is_empty()) {
                            self.open_details.insert(id.clone(), d.to_string());
                        }
                    }
                    self.open_order.push(id.clone());
                    self.call_by_id.insert(
                        id.clone(),
                        ToolCall::new(&id, name, &summary(name, input), &cut(&pretty_json(input))),
                    );
                }
                Some("tool_result") => {
                    let Some(id) = json::str_(b.get("tool_use_id")) else { continue };
                    if let Some(&at) = self.question_at.get(id) {
                        if let ItemKind::Question(q) = &self.done[at].kind {
                            let state = questions::claude_state(
                                q,
                                json::bool_(b.get("is_error")).unwrap_or(false),
                                o.get("toolUseResult"),
                                &result_text(b.get("content")),
                            );
                            self.set_question_state(at, state);
                            self.question_at.remove(id);
                            continue;
                        }
                    }
                    if let Some(c) = self.call_by_id.get_mut(id) {
                        c.output = Some(cut(&result_text(b.get("content"))));
                        c.is_error = json::bool_(b.get("is_error")).unwrap_or(false);
                        c.images = pics.iter().filter(|p| p.0.as_deref() == Some(id)).map(|p| p.1.clone()).collect();
                    }
                    self.close_call(id);
                }
                _ => {}
            }
        }
        // The owner's pictures go on the user item from this line; a line of pictures alone makes one.
        let own: Vec<_> = pics.iter().filter(|p| p.0.is_none()).map(|p| p.1.clone()).collect();
        if own.is_empty() {
            return;
        }
        if let Some(at) = (first_new..self.done.len()).find(|&j| matches!(self.done[j].kind, ItemKind::UserText(_))) {
            self.done[at].images = own;
        } else {
            self.flush();
            let mut item = ConversationItem::new(&format!("{uuid}-img"), ItemKind::UserText(String::new()), self.line_ts);
            item.images = own;
            self.done.push(item);
        }
    }

    fn set_question_state(&mut self, at: usize, state: QuestionState) {
        if let ItemKind::Question(q) = &mut self.done[at].kind {
            q.state = state;
        }
    }

    /// The call has its result: no longer open.
    fn close_call(&mut self, id: &str) {
        self.open_tool_uses.remove(id);
        self.open_details.remove(id);
        self.open_order.retain(|x| x != id);
    }

    /// Opens a call in the run (Claude Code's names and labels).
    fn open_call(&mut self, id: &str, name: &str, label: ToolLabel, open: Option<String>, call: ToolCall) {
        self.run.push(RunEntry { tool: name.into(), label, id: id.into() });
        self.call_ts.insert(id.into(), self.line_ts);
        if let Some(cmd) = open {
            self.open_tool_uses.insert(id.into(), cmd);
            self.open_order.push(id.into());
        }
        self.call_by_id.insert(id.into(), call);
    }

    /// A message the owner typed while the agent worked. Claude Code hands it over mid-turn as a `queued_command`
    /// attachment instead of a user line, so it shows here, where the agent took it. Task notices (`commandMode`
    /// other than "prompt") and subagent hand-backs (an origin other than "human") are not the owner's words.
    fn feed_queued(&mut self, o: &Obj) {
        let Some(a) = json::obj(o.get("attachment")) else { return };
        if json::str_(a.get("type")) != Some("queued_command") || json::str_(a.get("commandMode")) != Some("prompt") {
            return;
        }
        // Older clients log no origin: the owner's. An origin of another kind (peer, task-notification) is not.
        if let Some(origin) = a.get("origin") {
            if json::str_(json::obj(Some(origin)).and_then(|x| x.get("kind"))) != Some("human") {
                return;
            }
        }
        let t: String = if let Some(s) = json::str_(a.get("prompt")) {
            s.to_string()
        } else if let Some(blocks) = json::objs(a.get("prompt")) {
            blocks
                .iter()
                .filter(|b| json::str_(b.get("type")) == Some("text"))
                .filter_map(|b| json::str_(b.get("text")))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            return;
        };
        let trimmed = text::trim(&t);
        if trimmed.is_empty() {
            return;
        }
        let Some(shown) = user_visible(trimmed) else { return };
        self.flush();
        let uuid = json::str_(o.get("uuid")).map(str::to_string).unwrap_or_else(random_uuid);
        self.done.push(ConversationItem::new(&format!("{uuid}-0"), ItemKind::UserText(shown), self.line_ts));
    }

    // MARK: Codex rollout lines (`~/.codex/sessions/**/rollout-*.jsonl`)

    fn feed_codex(&mut self, ty: Option<&str>, payload: &Obj, offset: Option<u64>) {
        if ty != Some("response_item") {
            return;
        }
        self.codex_seq += 1;
        let bid = offset.map(|o| format!("cx@{o}")).unwrap_or_else(|| format!("cx{}", self.codex_seq));
        let ptype = json::str_(payload.get("type"));
        match ptype {
            Some("message") => {
                let role = json::str_(payload.get("role"));
                if role != Some("user") && role != Some("assistant") {
                    return;
                }
                let parts: Vec<&str> = json::objs(payload.get("content"))
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|c| json::str_(c.get("text")))
                    .collect();
                let joined = parts.join("\n");
                let t = text::trim(&joined);
                let pics: Vec<_> = if role == Some("user") {
                    log_image::refs(&log_image::found_codex(payload), offset).into_iter().map(|p| p.1).collect()
                } else {
                    vec![]
                };
                if t.is_empty() && pics.is_empty() {
                    return;
                }
                self.flush();
                if role == Some("user") {
                    let mut shown = String::new();
                    if !t.is_empty() {
                        if text::has_prefix(t, "# AGENTS.md") {
                            return;
                        }
                        let Some(s) = user_visible(t) else { return };
                        shown = s;
                    }
                    self.answer_async_questions();
                    let mut item = ConversationItem::new(&bid, ItemKind::UserText(shown), self.line_ts);
                    item.images = pics;
                    self.done.push(item);
                } else {
                    self.done.push(ConversationItem::new(&bid, ItemKind::AssistantText(t.to_string()), self.line_ts));
                }
            }
            Some("custom_tool_call") | Some("function_call") | Some("local_shell_call") => {
                let name = json::str_(payload.get("name"))
                    .unwrap_or(if ptype == Some("local_shell_call") { "shell" } else { "tool" })
                    .to_string();
                let id = json::str_(payload.get("call_id")).map(str::to_string).unwrap_or_else(|| bid.clone());
                if let Some(q) =
                    questions::codex(&name, &id, json::str_(payload.get("arguments")).unwrap_or(""))
                {
                    self.flush();
                    self.question_at.insert(id.clone(), self.done.len());
                    self.done.push(ConversationItem::new(&format!("q-{id}"), ItemKind::Question(q), self.line_ts));
                    return;
                }
                if self.run.is_empty() {
                    self.run_key = bid.clone();
                }
                let raw = json::str_(payload.get("arguments")).or(json::str_(payload.get("input"))).unwrap_or("");
                let cmd = codex_command(payload);
                let call = ToolCall::new(&id, &name, &cmd, &cut(raw));
                self.open_call(&id, &name, codex_label(&name), Some(cmd), call);
            }
            Some("custom_tool_call_output") | Some("function_call_output") | Some("local_shell_call_output") => {
                let Some(id) = json::str_(payload.get("call_id")) else { return };
                if let Some(&at) = self.question_at.get(id) {
                    if let ItemKind::Question(q) = &self.done[at].kind {
                        let mut q = q.clone();
                        if let Some(state) = questions::codex_state(&q, &result_text(payload.get("output"))) {
                            q.state = state.clone();
                            self.set_question_state(at, state);
                        }
                        if q.state != QuestionState::Pending || q.delivery == Delivery::Picker {
                            self.question_at.remove(id);
                        }
                        return;
                    }
                }
                if let Some(c) = self.call_by_id.get_mut(id) {
                    c.output = Some(cut(&result_text(payload.get("output"))));
                    c.images = log_image::refs(&log_image::found_codex(payload), offset).into_iter().map(|p| p.1).collect();
                }
                self.close_call(id);
            }
            _ => {}
        }
    }

    /// A user message answers every open `request_user_input_async` question (its answer is free text).
    fn answer_async_questions(&mut self) {
        let open: Vec<(String, usize)> = self.question_at.iter().map(|(k, v)| (k.clone(), *v)).collect();
        for (id, at) in open {
            let ItemKind::Question(q) = &self.done[at].kind else { continue };
            if q.delivery != Delivery::Message {
                continue;
            }
            self.set_question_state(at, QuestionState::Answered(None));
            self.question_at.remove(&id);
        }
    }

    fn flush(&mut self) {
        if self.run.is_empty() {
            return;
        }
        let run = std::mem::take(&mut self.run);
        let item = chips(&run, &self.run_key, &self.call_by_id, self.ts(run.first()));
        self.runs_by_key.insert(self.run_key.clone(), run);
        self.done.push(item);
    }
}

pub fn codex_label(tool: &str) -> ToolLabel {
    match tool {
        "exec" | "exec_command" | "shell" | "local_shell" | "container.exec" | "write_stdin" => {
            ToolLabel::new("terminal", "Ran a command", Some("Ran %d commands"))
        }
        "apply_patch" => ToolLabel::new("pencil", "Edited files", Some("Edited files %d times")),
        "web_search" | "web.run" => ToolLabel::new("globe", "Searched the web", Some("Searched the web %d times")),
        "update_plan" => ToolLabel::new("checklist", "Updated the plan", None),
        _ => ToolLabel::new("wrench", &format!("Used {tool}"), None),
    }
}

pub fn codex_command(payload: &Obj) -> String {
    if let Some(o) = json::str_(payload.get("arguments")).and_then(json::parse_obj) {
        for k in ["cmd", "command"] {
            if let Some(s) = json::str_(o.get(k)) {
                if !s.is_empty() {
                    return s.to_string();
                }
            }
            if let Some(a) = json::strs(o.get(k)) {
                if !a.is_empty() {
                    return a.join(" ");
                }
            }
        }
    }
    if let Some(input) = json::str_(payload.get("input")) {
        if !input.is_empty() {
            let first = text::split_lf(input, true).first().copied().unwrap_or("");
            return text::prefix(first, 200).to_string();
        }
    }
    json::str_(payload.get("name")).unwrap_or("tool").to_string()
}

// MARK: helpers

fn chips(run: &[RunEntry], key: &str, calls: &HashMap<String, ToolCall>, at: Option<f64>) -> ConversationItem {
    let mut order: Vec<&str> = vec![];
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut labels: HashMap<&str, &ToolLabel> = HashMap::new();
    let mut groups: HashMap<&str, Vec<ToolCall>> = HashMap::new();
    for r in run {
        let k = r.label.singular.as_str();
        if !counts.contains_key(k) {
            order.push(k);
            labels.insert(k, &r.label);
        }
        *counts.entry(k).or_insert(0) += 1;
        if let Some(c) = calls.get(&r.id) {
            groups.entry(k).or_default().push(c.clone());
        }
    }
    let chips = order
        .iter()
        .map(|k| {
            let l = labels[k];
            let n = counts[k];
            let text = if n > 1 { l.plural.as_deref().map(|p| text::format_d(p, n)) } else { None }
                .unwrap_or_else(|| l.singular.clone());
            ToolChip {
                id: stable_uuid(&format!("{key}|{k}")),
                symbol: l.symbol.clone(),
                text,
                calls: groups.remove(k).unwrap_or_default(),
            }
        })
        .collect();
    ConversationItem::new(&format!("tools-{key}"), ItemKind::ToolChips(chips), at)
}

pub fn label(tool: &str) -> ToolLabel {
    match tool {
        "Read" => ToolLabel::new("doc.text", "Read a file", Some("Read %d files")),
        "Edit" | "MultiEdit" | "NotebookEdit" => ToolLabel::new("pencil", "Edited a file", Some("Edited %d files")),
        "Write" => ToolLabel::new("square.and.pencil", "Wrote a file", Some("Wrote %d files")),
        "Bash" | "BashOutput" | "Monitor" => ToolLabel::new("terminal", "Ran a command", Some("Ran %d commands")),
        "Grep" | "Glob" => ToolLabel::new("magnifyingglass", "Searched the code", Some("Searched the code %d times")),
        "WebFetch" | "WebSearch" => ToolLabel::new("globe", "Searched the web", Some("Searched the web %d times")),
        "Agent" | "Task" => ToolLabel::new("person.2", "Delegated a task", Some("Delegated %d tasks")),
        "TodoWrite" => ToolLabel::new("checklist", "Updated the to-do list", None),
        _ => {
            if text::has_prefix(tool, "mcp__") {
                return ToolLabel::new("puzzlepiece.extension", "Used a connector", Some("Used connectors %d times"));
            }
            ToolLabel::new("wrench", &format!("Used {tool}"), None)
        }
    }
}

pub fn verb(tool: &str) -> String {
    match tool {
        "Bash" => "run a command".into(),
        "Edit" | "MultiEdit" | "NotebookEdit" => "edit a file".into(),
        "Write" => "write a file".into(),
        "Read" => "read a file".into(),
        "WebFetch" | "WebSearch" => "use the web".into(),
        _ => format!("use {tool}"),
    }
}

/// A UUID's `uuidString` (uppercase).
fn uuid_string(b: &[u8; 16]) -> String {
    let h: String = b.iter().map(|x| format!("{x:02X}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Same text, same UUID: chip identity must not change on every `items()` call, or the UI diffs every row each time.
pub fn stable_uuid(s: &str) -> String {
    let mut a: u64 = 0xcbf29ce484222325;
    let mut b: u64 = 0x84222325cbf29ce4;
    for u in s.bytes() {
        a = (a ^ u as u64).wrapping_mul(0x100000001b3);
        b = b.wrapping_add(u as u64).wrapping_mul(0x100000001b3) ^ a;
    }
    let mut bytes = [0u8; 16];
    for i in 0..8 {
        bytes[i] = (a >> (8 * i)) as u8;
        bytes[8 + i] = (b >> (8 * i)) as u8;
    }
    uuid_string(&bytes)
}

/// `UUID().uuidString`: a random (version 4) UUID.
pub fn random_uuid() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut bytes = [0u8; 16];
    for (half, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(n);
        h.write_u128(now);
        h.write_usize(half);
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid_string(&bytes)
}

/// tool_result content: a string, or an array of `{type:text,text}` blocks.
pub fn result_text(v: Option<&Value>) -> String {
    if let Some(s) = json::str_(v) {
        return s.to_string();
    }
    if let Some(a) = json::objs(v) {
        return a.iter().filter_map(|b| json::str_(b.get("text"))).collect::<Vec<_>>().join("\n");
    }
    String::new()
}

pub fn summary(name: &str, input: &Obj) -> String {
    let keys: &[&str] = match name {
        "Bash" => &["command"],
        "Read" | "Write" | "Edit" | "MultiEdit" => &["file_path", "path"],
        "NotebookEdit" => &["notebook_path", "file_path"],
        "Task" | "Agent" => &["description", "prompt"],
        _ => &[],
    };
    for k in keys {
        if let Some(s) = json::str_(input.get(*k)) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    let mut sorted: Vec<&String> = input.keys().collect();
    sorted.sort();
    for k in sorted {
        if let Some(s) = json::str_(input.get(k)) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    name.to_string()
}

pub fn command_text(name: &str, input: &Obj) -> String {
    for k in ["command", "file_path", "path", "url", "query", "pattern", "description"] {
        if let Some(s) = json::str_(input.get(k)) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    name.to_string()
}

/// Tags whose text is the user's own. Claude Code wraps pasted text in `<pasted_content id="…">`. Any other text
/// that starts with a tag (`<system-reminder>`, `<command-name>`, `<task-notification>`, …) is injected context.
/// `hc`: a note another agent sent with `hc send` (see `hc_note`).
pub const VISIBLE_TAGS: [&str; 2] = ["pasted_content", "hc"];

fn tag_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^<([A-Za-z_][\w-]*)").unwrap())
}

fn pasted_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"<pasted_content(?: id="([^"]*)")?>"#).unwrap())
}

fn hc_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?s)^<hc(\s[^>]*)?>(.*)</hc>$").unwrap())
}

fn hc_from_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"\sfrom="([^"]*)""#).unwrap())
}

/// A note from another agent, as herdr-core's `hc send` types it: the whole message is one
/// `<hc from="t2 name" …>text</hc>`. Returns the sender as written ("t2 name", or "" without `from`) and the text.
pub fn hc_note(t: &str) -> Option<(String, String)> {
    let m = hc_re().captures(text::trim(t))?;
    let body = m.get(2).map_or("", |b| b.as_str());
    if body.contains("</hc>") {
        return None;
    }
    let attrs = m.get(1).map_or("", |a| a.as_str());
    let from = hc_from_re().captures(attrs).map_or("", |f| f.get(1).map_or("", |x| x.as_str()));
    Some((from.to_string(), text::trim(body).to_string()))
}

/// The text to show for a user message, or nil when it is injected context. Pasted blocks lose their wrapper
/// tags; typed text and pasted blocks keep their order, separated by a blank line.
pub fn user_visible(t: &str) -> Option<String> {
    if text::has_prefix(t, "<") {
        let m = tag_re().captures(t)?;
        if !VISIBLE_TAGS.contains(&&m[1]) {
            return None;
        }
    } else if is_injected(t) {
        return None;
    }
    if !t.contains("<pasted_content") {
        return Some(t.to_string());
    }
    let out = unwrap_pasted(t);
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Splits text into typed parts and pasted blocks. A block ends at the closing tag with the same id (or the
/// plain closing tag without an id), so pasted text that itself quotes these tags stays as it is.
pub fn unwrap_pasted(t: &str) -> String {
    let mut parts: Vec<&str> = vec![];
    let mut rest = t;
    while let Some(open) = pasted_re().captures(rest) {
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
    parts.iter().map(|p| text::trim(p)).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n\n")
}

pub fn is_injected(t: &str) -> bool {
    if text::has_prefix(t, "<") {
        return true; // tagged text; `user_visible` lets the visible tags through
    }
    if text::has_prefix(t, "[Request interrupted") {
        return true;
    }
    if text::has_prefix(t, "Caveat:") {
        return true;
    }
    false
}
