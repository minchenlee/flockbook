//! pi (`~/.pi/agent/sessions/<cwd-slug>/<time>_<uuid>.jsonl`). Entries form a tree by `id`/`parentId`; the shown
//! conversation is the branch that ends at the newest entry. Lines are fed in file order; when an entry hangs off an
//! earlier entry (an edited or retried turn), the items are rebuilt from that branch. omp (oh-my-pi, a pi fork) writes
//! the same tree with more entry types, a `title` slot before the header, and `activeLeafId` on an entry written off
//! the shown branch (late usage, a background result): the shown branch stays at that leaf.

use std::collections::HashMap;

use serde_json::Value;

use crate::util::json::{self, Obj};
use super::{command_text, cut, label, pretty_json, result_text, summary, text, ConversationParser};
use crate::log_image;
use crate::model::{ConversationItem, ItemKind, ToolCall};

#[derive(Clone, Debug, Default)]
pub struct PiState {
    parent: HashMap<String, String>,       // entry id -> parent id ("" = root)
    lines: HashMap<String, String>,        // entry id -> line, only for entries that render
    offsets: HashMap<String, Option<u64>>, // entry id -> the line's byte offset (picture refs), when known
    last: Option<String>,
    replaying: bool,
}

/// pi's entry types, then omp's (session-entries.ts).
pub const PI_TYPES: [&str; 20] = [
    "session",
    "message",
    "model_change",
    "thinking_level_change",
    "custom",
    "custom_message",
    "session_info",
    "compaction",
    "context_edit",
    "branch_summary",
    "title",
    "title_change",
    "model_usage",
    "service_tier_change",
    "label",
    "session_init",
    "reset_boundary",
    "mode_change",
    "ttsr_injection",
    "credential_pin",
];

pub fn pi_renders(o: &Obj) -> bool {
    match json::str_(o.get("type")) {
        Some("message") => json::str_(json::obj(o.get("message")).and_then(|m| m.get("role"))) != Some("system"),
        Some("compaction") | Some("custom_message") => true,
        _ => false,
    }
}

/// pi's built-in tools under Claude Code's names, so chips and verbs read the same.
pub fn pi_tool(name: &str) -> &str {
    match name {
        "bash" => "Bash",
        "read" => "Read",
        "edit" => "Edit",
        "write" => "Write",
        "grep" => "Grep",
        "find" | "ls" => "Glob",
        "subagent" | "subagent_start" => "Agent",
        _ => name,
    }
}

impl ConversationParser {
    pub(super) fn feed_pi(&mut self, o: &Obj, line: &str, offset: Option<u64>) {
        if json::str_(o.get("type")) == Some("session") {
            return; // header: not in the tree
        }
        let Some(id) = json::str_(o.get("id")) else { return }; // omp's title slot has no id
        let parent = json::str_(o.get("parentId"));
        // omp: the leaf after this entry (absent: the entry; null: cleared). Replayed entries are on the branch.
        let leaf = match o.get("activeLeafId") {
            Some(Value::Null) => None,
            Some(v) => v.as_str(),
            None => Some(id),
        };
        if !self.pi.replaying && leaf != Some(id) {
            self.pi.parent.insert(id.into(), parent.unwrap_or("").into());
            if pi_renders(o) {
                self.pi.lines.insert(id.into(), line.into());
                self.pi.offsets.insert(id.into(), offset);
            }
            match leaf {
                None => self.clear_pi(),
                Some(l) if self.pi.last.as_deref() != Some(l) && self.pi.parent.contains_key(l) => self.rebuild_pi(l),
                Some(_) => {} // the shown leaf, or one omp no longer has: the branch stays
            }
            return;
        }
        if !self.pi.replaying {
            if let (Some(parent), Some(last)) = (parent, self.pi.last.as_deref()) {
                if parent != last && self.pi.parent.contains_key(parent) {
                    self.rebuild_pi(parent);
                }
            }
        }
        self.pi.parent.insert(id.into(), parent.unwrap_or("").into());
        self.pi.last = Some(id.into());
        if !self.pi.replaying && pi_renders(o) {
            self.pi.lines.insert(id.into(), line.into());
            self.pi.offsets.insert(id.into(), offset);
        }
        self.render_pi(o, id);
    }

    /// omp cleared the leaf: nothing is shown, the tree stays for a later branch.
    fn clear_pi(&mut self) {
        let mut fresh = ConversationParser::new();
        fresh.pi = std::mem::take(&mut self.pi);
        fresh.pi.last = None;
        *self = fresh;
    }

    /// Items become the branch from the root to `leaf` (replayed from the kept lines).
    fn rebuild_pi(&mut self, leaf: &str) {
        let mut chain: Vec<String> = vec![];
        let mut cur = leaf.to_string();
        while !cur.is_empty() && chain.len() <= self.pi.parent.len() {
            let Some(up) = self.pi.parent.get(&cur) else { break };
            let up = up.clone();
            chain.push(cur);
            cur = up;
        }
        let mut fresh = ConversationParser::new();
        fresh.pi = self.pi.clone();
        fresh.pi.replaying = true;
        for id in chain.iter().rev() {
            if let Some(l) = self.pi.lines.get(id) {
                fresh.feed_at(l, self.pi.offsets.get(id).copied().flatten());
            }
        }
        fresh.pi.replaying = false;
        fresh.pi.last = Some(leaf.into());
        *self = fresh;
    }

    fn render_pi(&mut self, o: &Obj, id: &str) {
        match json::str_(o.get("type")) {
            Some("compaction") => {
                self.flush();
                self.done.push(ConversationItem::new(id, ItemKind::Status("Context compacted".into()), self.line_ts));
            }
            Some("custom_message") => {
                // extension notices shown in pi's TUI (e.g. a background subagent finished)
                if json::bool_(o.get("display")) != Some(true) {
                    return;
                }
                let all = result_text(o.get("content"));
                // omp: a background result handed to the model, not a notice
                if text::trim(&all).starts_with("<system-notice>") {
                    return;
                }
                let Some(first) = text::first_line(text::trim(&all)) else { return };
                self.flush();
                self.done.push(ConversationItem::new(id, ItemKind::Status(text::prefix(first, 160).into()), self.line_ts));
            }
            Some("message") => {
                let Some(m) = json::obj(o.get("message")) else { return };
                self.render_pi_message(m, id);
            }
            _ => {}
        }
    }

    fn render_pi_message(&mut self, m: &Obj, id: &str) {
        let string_block: Obj;
        let blocks: Vec<&Obj> = if let Some(b) = json::objs(m.get("content")) {
            b
        } else if let Some(s) = json::str_(m.get("content")) {
            let mut b = Obj::new();
            b.insert("type".into(), Value::from("text"));
            b.insert("text".into(), Value::from(s));
            string_block = b;
            vec![&string_block]
        } else {
            vec![]
        };
        match json::str_(m.get("role")) {
            Some("user") => {
                for (i, b) in blocks.iter().enumerate() {
                    if json::str_(b.get("type")) != Some("text") {
                        continue;
                    }
                    let t = text::trim(json::str_(b.get("text")).unwrap_or(""));
                    if t.is_empty() {
                        continue;
                    }
                    self.flush();
                    self.done.push(ConversationItem::new(&format!("{id}-{i}"), ItemKind::UserText(t.into()), self.line_ts));
                }
            }
            Some("assistant") => {
                for (i, b) in blocks.iter().enumerate() {
                    match json::str_(b.get("type")) {
                        Some("text") => {
                            let t = text::trim(json::str_(b.get("text")).unwrap_or(""));
                            if t.is_empty() {
                                continue;
                            }
                            self.flush();
                            self.done.push(ConversationItem::new(
                                &format!("{id}-{i}"),
                                ItemKind::AssistantText(t.into()),
                                self.line_ts,
                            ));
                        }
                        Some("toolCall") => {
                            let name = pi_tool(json::str_(b.get("name")).unwrap_or("tool")).to_string();
                            let empty = Obj::new();
                            let input = json::obj(b.get("arguments")).unwrap_or(&empty);
                            let key = format!("{id}-{i}");
                            let call = json::str_(b.get("id")).map(str::to_string).unwrap_or_else(|| key.clone());
                            self.open_pi_call(&call, &name, input, &key);
                        }
                        _ => {} // thinking: hidden, as for Claude Code
                    }
                }
                match json::str_(m.get("stopReason")) {
                    Some("error") => {
                        let msg = text::first_line(json::str_(m.get("errorMessage")).unwrap_or("")).unwrap_or("");
                        let s = if msg.is_empty() { "Error".to_string() } else { format!("Error: {}", text::prefix(msg, 160)) };
                        self.flush();
                        self.done.push(ConversationItem::new(&format!("{id}-err"), ItemKind::Status(s), self.line_ts));
                    }
                    Some("aborted") => {
                        self.flush();
                        self.done.push(ConversationItem::new(
                            &format!("{id}-err"),
                            ItemKind::Status("Interrupted".into()),
                            self.line_ts,
                        ));
                    }
                    _ => {}
                }
            }
            Some("toolResult") => {
                let Some(call) = json::str_(m.get("toolCallId")) else { return };
                let offset = self.line_offset;
                if let Some(c) = self.call_by_id.get_mut(call) {
                    c.output = Some(cut(&result_text(m.get("content"))));
                    c.is_error = json::bool_(m.get("isError")).unwrap_or(false);
                    c.images = log_image::refs(&log_image::found_message(m), offset).into_iter().map(|p| p.1).collect();
                }
                self.close_call(call);
            }
            Some("bashExecution") => {
                // the user ran `!command` in pi
                let cmd = json::str_(m.get("command")).unwrap_or("");
                let mut input = Obj::new();
                input.insert("command".into(), Value::from(cmd));
                self.open_pi_call(id, "Bash", &input, id);
                if let Some(c) = self.call_by_id.get_mut(id) {
                    c.output = Some(cut(json::str_(m.get("output")).unwrap_or("")));
                    c.is_error = json::int(m.get("exitCode")).unwrap_or(0) != 0;
                }
                self.close_call(id);
            }
            _ => {} // system prompt
        }
    }

    fn open_pi_call(&mut self, id: &str, name: &str, input: &Obj, key: &str) {
        if self.run.is_empty() {
            self.run_key = key.into();
        }
        let call = ToolCall::new(id, name, &summary(name, input), &cut(&pretty_json(input)));
        self.open_call(id, name, label(name), Some(command_text(name, input)), call);
    }
}
