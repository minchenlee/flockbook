//! Grok Build (`<GROK_HOME>/sessions/<cwd>/<id>/updates.jsonl`). Each line is an ACP session update:
//! `{timestamp, method: "session/update" | "_x.ai/session/update", params: {sessionId, update: {sessionUpdate, …}, _meta: {eventId, agentTimestampMs}}}`.
//! Text arrives as `*_chunk` updates; adjacent chunks of one kind are one message. Thinking is hidden. A `tool_call`
//! opens a call, `tool_call_update` with a final status gives its result, and `turn_completed` closes what is still open.
//! `ask_user_question` is a question card (`questions::grok`), answered by its final update.

use crate::util::json::{self, Obj};
use super::{cut, label, pretty_json, questions, summary, text, ConversationParser};
use crate::model::{ConversationItem, ItemKind, QuestionState, ToolCall};

#[derive(Clone, Debug, Default)]
pub struct GrokState {
    /// The text item the next chunk of the same kind continues: its kind, index in `done`, and raw text so far.
    open: Option<(String, usize, String)>,
}

static EMPTY: std::sync::OnceLock<Obj> = std::sync::OnceLock::new();

fn empty() -> &'static Obj {
    EMPTY.get_or_init(Obj::new)
}

pub fn grok_update(o: &Obj) -> Option<(&Obj, &Obj)> {
    let m = json::str_(o.get("method"))?;
    if m != "session/update" && m != "_x.ai/session/update" {
        return None;
    }
    let p = json::obj(o.get("params"))?;
    let u = json::obj(p.get("update"))?;
    json::str_(u.get("sessionUpdate"))?;
    Some((u, json::obj(p.get("_meta")).unwrap_or(empty())))
}

pub fn grok_text(kind: &str, raw: &str) -> ItemKind {
    let t = text::trim(raw).to_string();
    if kind == "user_message_chunk" {
        ItemKind::UserText(t)
    } else {
        ItemKind::AssistantText(t)
    }
}

/// Grok Build's tools under Claude Code's names, so chips read the same. Unknown names fall back on the tool kind.
pub fn grok_tool(name: &str, kind: Option<&str>, input: &Obj) -> String {
    match name {
        "run_terminal_command" | "monitor" | "get_command_or_subagent_output" => "Bash".into(),
        "read_file" => "Read".into(),
        "search_replace" => "Edit".into(),
        "write" => "Write".into(),
        "grep" => "Grep".into(),
        "list_dir" => "Glob".into(),
        "web_fetch" => "WebFetch".into(),
        "web_search" => "WebSearch".into(),
        "spawn_subagent" => "Agent".into(),
        "todo_write" => "TodoWrite".into(),
        "use_tool" => format!("mcp__{}", json::str_(input.get("tool_name")).unwrap_or("tool")),
        _ => match kind {
            Some("execute") => "Bash".into(),
            Some("read") => "Read".into(),
            Some("edit") => "Edit".into(),
            Some("search") => "Grep".into(),
            Some("fetch") => "WebFetch".into(),
            _ => name.into(),
        },
    }
}

pub fn grok_summary(name: &str, input: &Obj) -> String {
    let keys: &[&str] = match name {
        "Bash" => &["command"],
        "Read" => &["target_file", "file_path"],
        "Edit" | "Write" => &["file_path", "target_file"],
        "Grep" => &["pattern"],
        "Glob" => &["target_directory"],
        "WebFetch" => &["url"],
        "Agent" => &["description", "prompt"],
        _ => &[],
    };
    for k in keys {
        if let Some(s) = json::str_(input.get(*k)) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    summary(name, input)
}

/// The text of an update's `content` blocks (`[{content:{text}}]`).
fn content_text(u: &Obj) -> String {
    json::objs(u.get("content"))
        .unwrap_or_default()
        .iter()
        .filter_map(|c| json::str_(json::obj(c.get("content")).and_then(|x| x.get("text"))))
        .collect::<Vec<_>>()
        .join("\n")
}

impl ConversationParser {
    pub(super) fn feed_grok(&mut self, u: &Obj, meta: &Obj, top: &Obj, offset: Option<u64>) {
        if let Some(ms) = json::f64_(meta.get("agentTimestampMs")) {
            self.line_ts = Some(ms / 1000.0).filter(|t| t.is_finite());
        } else if let Some(s) = json::f64_(top.get("timestamp")) {
            self.line_ts = Some(s).filter(|t| t.is_finite());
        }
        self.codex_seq += 1;
        let bid = json::str_(meta.get("eventId"))
            .map(|e| format!("gk-{e}"))
            .or_else(|| offset.map(|o| format!("gk@{o}")))
            .unwrap_or_else(|| format!("gk{}", self.codex_seq));
        let kind = json::str_(u.get("sessionUpdate")).unwrap_or("");
        match kind {
            "user_message_chunk" | "agent_message_chunk" => {
                let content = json::obj(u.get("content"));
                if json::str_(content.and_then(|c| c.get("type"))) != Some("text") {
                    return;
                }
                let Some(piece) = json::str_(content.and_then(|c| c.get("text"))) else { return };
                if let Some((okind, at, otext)) = self.grok.open.clone() {
                    if okind == kind && self.run.is_empty() && at + 1 == self.done.len() {
                        let t = format!("{otext}{piece}");
                        self.done[at].kind = grok_text(kind, &t);
                        self.grok.open = Some((kind.into(), at, t));
                        return;
                    }
                }
                if text::trim(piece).is_empty() {
                    return;
                }
                self.flush();
                self.done.push(ConversationItem::new(&bid, grok_text(kind, piece), self.line_ts));
                self.grok.open = Some((kind.into(), self.done.len() - 1, piece.into()));
            }
            "tool_call" => {
                self.grok.open = None;
                let Some(id) = json::str_(u.get("toolCallId")) else { return };
                let tool = json::obj(u.get("_meta")).and_then(|m| json::obj(m.get("x.ai/tool")));
                let input = json::obj(u.get("rawInput")).unwrap_or(empty());
                let tname = json::str_(tool.and_then(|t| t.get("name")));
                let tkind = json::str_(tool.and_then(|t| t.get("kind")));
                if tname == Some("ask_user_question") || tkind == Some("ask_user") {
                    if let Some(q) = questions::grok(id, input) {
                        // a question card, not a tool chip
                        self.flush();
                        self.question_at.insert(id.into(), self.done.len());
                        self.done.push(ConversationItem::new(&format!("q-{id}"), ItemKind::Question(q), self.line_ts));
                        return;
                    }
                }
                let name = grok_tool(tname.or(json::str_(u.get("title"))).unwrap_or("tool"), tkind, input);
                let sum = grok_summary(&name, input);
                if self.run.is_empty() {
                    self.run_key = bid.clone();
                }
                let call = ToolCall::new(id, &name, &sum, &cut(&pretty_json(input)));
                self.open_call(id, &name, label(&name), Some(sum), call);
            }
            "tool_call_update" => {
                self.grok.open = None;
                let Some(id) = json::str_(u.get("toolCallId")) else { return };
                let status = json::str_(u.get("status"));
                let finished = status == Some("completed") || status == Some("failed");
                if let Some(&at) = self.question_at.get(id) {
                    if let ItemKind::Question(q) = &self.done[at].kind {
                        if !finished {
                            return;
                        }
                        let state = questions::grok_state(q, status == Some("failed"), u.get("rawOutput"), &content_text(u));
                        self.set_question_state(at, state);
                        self.question_at.remove(id);
                        return;
                    }
                }
                if !finished {
                    return;
                }
                let mut out = content_text(u);
                if out.is_empty() {
                    if let Some(s) = json::obj(u.get("rawOutput")).and_then(|r| json::str_(r.get("output_for_prompt"))) {
                        out = s.into();
                    }
                }
                if let Some(c) = self.call_by_id.get_mut(id) {
                    c.output = Some(cut(&out));
                    c.is_error = status == Some("failed");
                }
                self.close_call(id);
            }
            "turn_completed" => {
                self.grok.open = None;
                for id in std::mem::take(&mut self.open_order) {
                    self.open_tool_uses.remove(&id);
                }
                // A question still open when the turn ends was never answered.
                for (_, at) in std::mem::take(&mut self.question_at) {
                    self.set_question_state(at, QuestionState::Declined);
                }
                if json::str_(u.get("stop_reason")) == Some("cancelled") {
                    self.flush();
                    self.done.push(ConversationItem::new(&bid, ItemKind::Status("Interrupted".into()), self.line_ts));
                }
            }
            "agent_thought_chunk" => self.grok.open = None,
            _ => {} // hook runs and other notices: not part of the conversation
        }
    }
}
