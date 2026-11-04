//! agy, the Antigravity CLI (`~/.gemini/antigravity-cli/brain/<id>/.system_generated/logs/transcript_full.jsonl`).
//! One step per line: `{step_index, source, type, status, created_at, content, thinking?, tool_calls?}`.
//! `USER_INPUT` is the prompt (inside `<USER_REQUEST>`), `PLANNER_RESPONSE` is the reply and its `tool_calls`
//! (`{name, args}`, no call id). Call i of step p gets its result in the i-th step after p that is not injected (p+1+i
//! when nothing is injected); lines can come out of step order (a result line before its call), so results pair by
//! `step_index`, not by line order. `SYSTEM_MESSAGE` is injected text; `CHECKPOINT` marks truncated history. An injected
//! step that comes in an open call's slot pushes that call's result one step on. A call without a result is pending.
//! `ask_question` is a question card (result `A1: <answer>`). A denied call never gets a result (seen 2026-11-02):
//! a later step past its slot, or the next `USER_INPUT`, closes it as not run (a late result still fills it).

use std::collections::{HashMap, HashSet};

use crate::util::json::{self, Obj};
use super::{cut, label, parse_time, pretty_json, questions, text, ConversationParser};
use crate::model::{ConversationItem, ItemKind, QuestionState, ToolCall};

#[derive(Clone, Debug, Default)]
pub struct AgyState {
    slot: HashMap<i64, String>,          // result step -> call id, until the result arrives
    slot_of: HashMap<String, i64>,       // call id -> result step, while the call is open
    early: HashMap<i64, (String, bool)>, // results whose call line has not come yet
    injected: HashSet<i64>,              // steps that are neither prompt, reply nor result
    call_step: HashMap<String, i64>,     // call id -> its PLANNER step, while the call is open
}

pub fn is_agy(o: &Obj) -> bool {
    !o.contains_key("uuid")
        && json::is_number(o.get("step_index"))
        && json::str_(o.get("type")).is_some()
        && json::str_(o.get("source")).is_some()
}

/// The typed prompt: the `<USER_REQUEST>` body; plain text without tags as is; other tagged text is injected.
pub fn agy_request(raw: &str) -> Option<String> {
    if let Some(a) = raw.find("<USER_REQUEST>") {
        let rest = &raw[a + "<USER_REQUEST>".len()..];
        let body = rest.find("</USER_REQUEST>").map(|e| &rest[..e]).unwrap_or(rest);
        return Some(text::trim(body).to_string());
    }
    let t = text::trim(raw);
    if text::has_prefix(t, "<") {
        None
    } else {
        Some(t.to_string())
    }
}

/// agy's tools under Claude Code's names, so chips and requests read the same.
pub fn agy_tool(name: &str) -> &str {
    match name {
        "run_command" => "Bash",
        "view_file" | "read_file" => "Read",
        "write_to_file" | "write_file" => "Write",
        "replace_file_content" | "multi_replace_file_content" => "Edit",
        "grep_search" | "find_by_name" | "list_dir" | "code_search" => "Grep",
        "read_url_content" | "search_web" | "execute_url" => "WebFetch",
        "invoke_subagent" => "Agent",
        _ => name,
    }
}

/// What the call acts on: the command, the file, the query; else agy's own `toolSummary`.
pub fn agy_summary(args: &Obj, fallback: &str) -> String {
    for k in ["CommandLine", "AbsolutePath", "TargetFile", "Path", "DirectoryPath", "SearchPath", "Query", "Pattern", "Url", "toolSummary"] {
        if let Some(s) = json::str_(args.get(k)) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}

/// Result text without the `Created At:` / `Completed At:` header lines and the tab indent.
pub fn agy_result(raw: &str) -> String {
    let lines: Vec<&str> = text::split_lf(raw, false)
        .into_iter()
        .filter(|l| !text::has_prefix(l, "Created At: ") && !text::has_prefix(l, "Completed At: "))
        .map(|l| l.trim_start_matches('\t'))
        .collect();
    text::trim(&lines.join("\n")).to_string()
}

impl ConversationParser {
    pub(super) fn feed_agy(&mut self, o: &Obj) {
        let step = json::int_value(o.get("step_index")).unwrap_or(0);
        let bid = format!("ag{step}");
        let content = json::str_(o.get("content")).unwrap_or("");
        self.line_ts = json::str_(o.get("created_at")).and_then(parse_time).or(self.line_ts);
        let ty = json::str_(o.get("type"));
        if ty != Some("GENERIC") {
            self.close_open_agy_calls(if ty == Some("USER_INPUT") { None } else { Some(step) });
        }
        if ty != Some("GENERIC") && ty != Some("USER_INPUT") && ty != Some("PLANNER_RESPONSE") {
            self.inject_agy_step(step);
        }
        match ty {
            Some("USER_INPUT") => {
                let Some(t) = agy_request(content) else { return };
                if t.is_empty() {
                    return;
                }
                self.flush();
                self.done.push(ConversationItem::new(&bid, ItemKind::UserText(t), self.line_ts));
            }
            Some("PLANNER_RESPONSE") => {
                let t = text::trim(content);
                if !t.is_empty() {
                    self.flush();
                    self.done.push(ConversationItem::new(&bid, ItemKind::AssistantText(t.into()), self.line_ts));
                }
                let mut at = step;
                for (i, c) in json::objs(o.get("tool_calls")).unwrap_or_default().iter().enumerate() {
                    at += 1;
                    while self.agy.injected.contains(&at) {
                        at += 1;
                    }
                    let empty = Obj::new();
                    let args = json::obj(c.get("args")).unwrap_or(&empty);
                    let id = format!("{bid}-{i}");
                    if json::str_(c.get("name")) == Some("ask_question") {
                        if let Some(q) = questions::agy(&id, args) {
                            // a card, never a permission
                            self.flush();
                            self.question_at.insert(id.clone(), self.done.len());
                            self.done.push(ConversationItem::new(&format!("q-{id}"), ItemKind::Question(q), self.line_ts));
                            self.open_agy_slot(&id, at, step);
                            continue;
                        }
                    }
                    let name = agy_tool(json::str_(c.get("name")).unwrap_or("tool")).to_string();
                    if self.run.is_empty() {
                        self.run_key = id.clone();
                    }
                    let sum = agy_summary(args, &name);
                    let call = ToolCall::new(&id, &name, &sum, &cut(&pretty_json(args)));
                    // The slot opens the call (`open_agy_slot`), not the run.
                    self.open_call(&id, &name, label(&name), None, call);
                    self.open_tool_uses.insert(id.clone(), sum);
                    self.open_agy_slot(&id, at, step);
                }
            }
            Some("GENERIC") => {
                let r = (agy_result(content), json::str_(o.get("status")) == Some("ERROR"));
                let Some(id) = self.agy.slot.remove(&step) else {
                    self.agy.early.insert(step, r);
                    return;
                };
                self.fill_agy(&id, r);
            }
            Some("CHECKPOINT") => {
                self.flush();
                self.done.push(ConversationItem::new(&bid, ItemKind::Status("Context compacted".into()), self.line_ts));
            }
            _ => {} // SYSTEM_MESSAGE and others: injected, not shown
        }
    }

    /// A call waits for its result in step `at`; a result line that came first fills it now.
    fn open_agy_slot(&mut self, id: &str, at: i64, from: i64) {
        if let Some(r) = self.agy.early.remove(&at) {
            self.fill_agy(id, r);
            return;
        }
        self.agy.slot.insert(at, id.into());
        self.agy.slot_of.insert(id.into(), at);
        self.agy.call_step.insert(id.into(), from);
        self.open_order.push(id.into());
    }

    /// Injected step `s`: an open call of an earlier step whose slot is `s` or later gets its result one step on
    /// (past other injected steps). Calls move from the last slot down, so no slot is taken twice.
    fn inject_agy_step(&mut self, s: i64) {
        self.agy.injected.insert(s);
        let mut moving: Vec<(String, i64)> = self
            .agy
            .slot_of
            .iter()
            .filter(|(id, at)| **at >= s && self.agy.call_step.get(*id).copied().unwrap_or(s) < s)
            .map(|(id, at)| (id.clone(), *at))
            .collect();
        moving.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (id, at) in moving {
            if self.agy.slot.get(&at) == Some(&id) {
                self.agy.slot.remove(&at);
            }
            let mut next = at + 1;
            while self.agy.injected.contains(&next) {
                next += 1;
            }
            if let Some(r) = self.agy.early.remove(&next) {
                self.fill_agy(&id, r);
                continue;
            }
            self.agy.slot.insert(next, id.clone());
            self.agy.slot_of.insert(id, next);
        }
    }

    /// The result of call `id` (also when it was closed as not run before).
    fn fill_agy(&mut self, id: &str, r: (String, bool)) {
        self.agy.slot_of.remove(id);
        self.agy.call_step.remove(id);
        self.open_order.retain(|x| x != id);
        if let Some(&at) = self.question_at.get(id) {
            if let ItemKind::Question(q) = &self.done[at].kind {
                let state = questions::agy_state(q, &r.0);
                self.question_at.remove(id);
                self.set_question_state(at, state);
                return;
            }
        }
        self.open_tool_uses.remove(id);
        if let Some(c) = self.call_by_id.get_mut(id) {
            c.output = Some(cut(&r.0));
            c.is_error = r.1;
        }
    }

    /// Open calls whose result step is before `before` (all for nil: a new prompt) were denied or interrupted: agy
    /// writes no result for them. Their slot stays, so a result line that comes late still fills them.
    fn close_open_agy_calls(&mut self, before: Option<i64>) {
        for id in self.open_order.clone() {
            let Some(&at) = self.agy.slot_of.get(&id) else { continue };
            if !before.map(|b| at < b).unwrap_or(true) {
                continue;
            }
            self.agy.slot_of.remove(&id);
            self.agy.call_step.remove(&id);
            let question = self.question_at.get(&id).copied().filter(|&qi| matches!(self.done[qi].kind, ItemKind::Question(_)));
            if let Some(qi) = question {
                self.set_question_state(qi, QuestionState::Declined);
            } else {
                if let Some(c) = self.call_by_id.get_mut(&id) {
                    c.output = Some("Not run".into());
                    c.is_error = true;
                }
                self.open_tool_uses.remove(&id);
            }
        }
        let slot_of = &self.agy.slot_of;
        self.open_order.retain(|x| slot_of.contains_key(x));
    }
}
