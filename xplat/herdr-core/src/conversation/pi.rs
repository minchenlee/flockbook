//! pi (`~/.pi/agent/sessions/<cwd-slug>/<time>_<uuid>.jsonl`). Entries form a tree by `id`/`parentId`; the shown
//! conversation is the branch that ends at the newest entry. Lines are fed in file order; when an entry hangs off an
//! earlier entry (an edited or retried turn), the items are rebuilt from that branch.

use super::*;

#[derive(Debug, Clone, Default)]
pub(super) struct PiState {
    /// entry id -> parent id ("" = root)
    parent: HashMap<String, String>,
    /// entry id -> line, only for entries that render
    lines: HashMap<String, String>,
    last: Option<String>,
    replaying: bool,
}

pub(super) const PI_TYPES: [&str; 10] = [
    "session", "message", "model_change", "thinking_level_change", "custom", "custom_message", "session_info", "compaction", "context_edit", "branch_summary",
];

fn renders(o: &Obj) -> bool {
    match s_of(o, "type") {
        Some("message") => o.get("message").and_then(|m| m.get("role")).and_then(Value::as_str) != Some("system"),
        Some("compaction" | "custom_message") => true,
        _ => false,
    }
}

/// pi's built-in tools under Claude Code's names, so chips and verbs read the same.
fn pi_tool(name: &str) -> String {
    match name {
        "bash" => "Bash",
        "read" => "Read",
        "edit" => "Edit",
        "write" => "Write",
        "grep" => "Grep",
        "find" | "ls" => "Glob",
        "subagent" | "subagent_start" => "Agent",
        n => n,
    }
    .into()
}

impl ConversationParser {
    pub(super) fn feed_pi(&mut self, o: &Obj, line: &str) {
        if s_of(o, "type") == Some("session") {
            return; // header: not in the tree
        }
        let Some(id) = s_of(o, "id") else { return };
        let parent = s_of(o, "parentId");
        if !self.pi.replaying {
            if let (Some(parent), Some(last)) = (parent, self.pi.last.as_deref()) {
                if parent != last && self.pi.parent.contains_key(parent) {
                    let parent = parent.to_string();
                    self.rebuild_pi(&parent);
                }
            }
        }
        self.pi.parent.insert(id.into(), parent.unwrap_or("").into());
        self.pi.last = Some(id.into());
        if !self.pi.replaying && renders(o) {
            self.pi.lines.insert(id.into(), line.into());
        }
        self.render_pi(o, id);
    }

    /// Items become the branch from the root to `leaf` (replayed from the kept lines).
    fn rebuild_pi(&mut self, leaf: &str) {
        let mut chain: Vec<&str> = Vec::new();
        let mut cur = leaf;
        while !cur.is_empty() && chain.len() <= self.pi.parent.len() {
            let Some(up) = self.pi.parent.get(cur) else { break };
            chain.push(cur);
            cur = up;
        }
        let mut fresh = ConversationParser::new();
        fresh.pi = self.pi.clone();
        fresh.pi.replaying = true;
        for id in chain.iter().rev() {
            if let Some(l) = self.pi.lines.get(*id) {
                fresh.feed(l);
            }
        }
        fresh.pi.replaying = false;
        fresh.pi.last = Some(leaf.into());
        *self = fresh;
    }

    fn render_pi(&mut self, o: &Obj, id: &str) {
        match s_of(o, "type") {
            Some("compaction") => {
                self.flush();
                self.done.push(ConversationItem { id: id.into(), kind: ItemKind::Status("Context compacted".into()), timestamp: self.line_ts, ..Default::default() });
            }
            Some("custom_message") => {
                // extension notices shown in pi's TUI (e.g. a background subagent finished)
                if o.get("display").and_then(Value::as_bool) != Some(true) {
                    return;
                }
                let text = result_text(o.get("content"));
                let Some(first) = text.trim().lines().find(|l| !l.is_empty()) else { return };
                self.flush();
                self.done.push(ConversationItem { id: id.into(), kind: ItemKind::Status(first.chars().take(160).collect()), timestamp: self.line_ts, ..Default::default() });
            }
            Some("message") => {
                if let Some(Value::Object(m)) = o.get("message") {
                    self.render_pi_message(m, id);
                }
            }
            _ => {}
        }
    }

    fn render_pi_message(&mut self, m: &Obj, id: &str) {
        let blocks: Vec<Obj> = match m.get("content") {
            Some(Value::String(s)) => {
                let mut b = Obj::new();
                b.insert("type".into(), "text".into());
                b.insert("text".into(), s.clone().into());
                vec![b]
            }
            c => obj_array(c).map(|v| v.into_iter().cloned().collect()).unwrap_or_default(),
        };
        match s_of(m, "role") {
            Some("user") => {
                for (i, b) in blocks.iter().enumerate().filter(|(_, b)| s_of(b, "type") == Some("text")) {
                    let text = s_of(b, "text").unwrap_or("").trim();
                    if text.is_empty() {
                        continue;
                    }
                    self.flush();
                    self.done.push(ConversationItem { id: format!("{id}-{i}"), kind: ItemKind::UserText(text.into()), timestamp: self.line_ts, ..Default::default() });
                }
            }
            Some("assistant") => {
                for (i, b) in blocks.iter().enumerate() {
                    match s_of(b, "type") {
                        Some("text") => {
                            let text = s_of(b, "text").unwrap_or("").trim();
                            if text.is_empty() {
                                continue;
                            }
                            self.flush();
                            self.done.push(ConversationItem { id: format!("{id}-{i}"), kind: ItemKind::AssistantText(text.into()), timestamp: self.line_ts, ..Default::default() });
                        }
                        Some("toolCall") => {
                            let name = pi_tool(s_of(b, "name").unwrap_or("tool"));
                            let input = b.get("arguments").and_then(Value::as_object).cloned().unwrap_or_default();
                            let call_id = s_of(b, "id").map(String::from).unwrap_or_else(|| format!("{id}-{i}"));
                            self.open_pi_call(&call_id, &name, &input, &format!("{id}-{i}"));
                        }
                        _ => {} // thinking: hidden, as for Claude Code
                    }
                }
                match s_of(m, "stopReason") {
                    Some("error") => {
                        let msg = s_of(m, "errorMessage").unwrap_or("").lines().find(|l| !l.is_empty()).unwrap_or("");
                        let text = if msg.is_empty() { "Error".to_string() } else { format!("Error: {}", msg.chars().take(160).collect::<String>()) };
                        self.flush();
                        self.done.push(ConversationItem { id: format!("{id}-err"), kind: ItemKind::Status(text), timestamp: self.line_ts, ..Default::default() });
                    }
                    Some("aborted") => {
                        self.flush();
                        self.done.push(ConversationItem { id: format!("{id}-err"), kind: ItemKind::Status("Interrupted".into()), timestamp: self.line_ts, ..Default::default() });
                    }
                    _ => {}
                }
            }
            Some("toolResult") => {
                let Some(call) = s_of(m, "toolCallId") else { return };
                let is_error = m.get("isError").and_then(Value::as_bool).unwrap_or(false);
                self.close_call(call, &result_text(m.get("content")), is_error);
            }
            Some("bashExecution") => {
                // the user ran `!command` in pi
                let cmd = s_of(m, "command").unwrap_or("");
                let mut input = Obj::new();
                input.insert("command".into(), cmd.into());
                self.open_pi_call(id, "Bash", &input, id);
                let failed = m.get("exitCode").and_then(Value::as_i64).unwrap_or(0) != 0;
                self.close_call(id, s_of(m, "output").unwrap_or(""), failed);
            }
            _ => {} // system prompt
        }
    }

    fn open_pi_call(&mut self, id: &str, name: &str, input: &Obj, key: &str) {
        if self.run.is_empty() {
            self.run_key = key.into();
        }
        self.run.push(RunEntry { label: label_for(name), tool: name.into(), id: id.into() });
        self.set_call_ts(id);
        self.open_tool_uses.insert(id.into(), command_text(name, input));
        self.open_order.push(id.into());
        self.call_by_id.insert(
            id.into(),
            ToolCall { id: id.into(), tool_name: name.into(), summary: summary(name, input), input: cut(&pretty_json(input)), output: None, is_error: false, ..Default::default() },
        );
    }
}
