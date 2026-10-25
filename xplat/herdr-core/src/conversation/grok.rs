use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joins_chunks_and_closes_tools_on_turn_end() {
        let mut p = ConversationParser::new();
        for (n, u) in [
            serde_json::json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}),
            serde_json::json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":" world"}}),
            serde_json::json!({"sessionUpdate":"tool_call","toolCallId":"call","title":"run_terminal_command","rawInput":{"command":"pwd"}}),
            serde_json::json!({"sessionUpdate":"turn_completed"}),
        ].into_iter().enumerate() {
            p.feed_at(&serde_json::json!({"method":"session/update","params":{"update":u}}).to_string(), Some(n as u64));
        }
        assert_eq!(
            p.items()[0].kind,
            ItemKind::AssistantText("hello world".into())
        );
        assert!(!p.has_open_tool_use());
    }
    #[test]
    fn question_result_matches_by_question_text() {
        let mut p = ConversationParser::new();
        for u in [
            serde_json::json!({"sessionUpdate":"tool_call","toolCallId":"q","_meta":{"x.ai/tool":{"name":"ask_user_question"}},"rawInput":{"questions":[{"question":"Pick?","options":[{"label":"A"}]}]}}),
            serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":"q","status":"completed","rawOutput":{"UserAnswered":{"answers":[{"question":"Pick?","picked":["A"]}]}}}),
        ] {
            p.feed(
                &serde_json::json!({"method":"_x.ai/session/update","params":{"update":u}})
                    .to_string(),
            );
        }
        let ItemKind::Question(q) = &p.items()[0].kind else {
            panic!("question")
        };
        assert_eq!(q.state, QuestionState::Answered(Some(vec!["A".into()])));
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct GrokState {
    open: Option<(String, usize, String)>,
}

pub(super) fn is_grok(o: &Obj) -> bool {
    matches!(
        s_of(o, "method"),
        Some("session/update" | "_x.ai/session/update")
    ) && o.pointer_update().is_some()
}
// Keep access to ACP envelopes separate from the normal Claude/Codex message envelopes.
trait Update {
    fn pointer_update(&self) -> Option<&Obj>;
}
impl Update for Obj {
    fn pointer_update(&self) -> Option<&Obj> {
        self.get("params")?.get("update")?.as_object()
    }
}
impl ConversationParser {
    pub(super) fn feed_grok(&mut self, top: &Obj, offset: Option<u64>) {
        let Some(u) = top.pointer_update() else {
            return;
        };
        let meta = top.get("params").and_then(|p| p.get("_meta"));
        self.line_ts = meta
            .and_then(|m| m.get("agentTimestampMs"))
            .and_then(Value::as_f64)
            .map(|t| t / 1000.0)
            .or(self.line_ts)
            .or_else(|| top.get("timestamp").and_then(Value::as_f64));
        self.codex_seq += 1;
        let bid = meta
            .and_then(|m| m.get("eventId"))
            .and_then(Value::as_str)
            .map(|s| format!("gk-{s}"))
            .or_else(|| offset.map(|n| format!("gk@{n}")))
            .unwrap_or_else(|| format!("gk{}", self.codex_seq));
        let kind = s_of(u, "sessionUpdate").unwrap_or("");
        match kind {
            "user_message_chunk" | "agent_message_chunk" => {
                let Some(c) = u.get("content") else { return };
                if c.get("type").and_then(Value::as_str) != Some("text") {
                    return;
                }
                let Some(piece) = c.get("text").and_then(Value::as_str) else {
                    return;
                };
                if let Some((k, at, text)) = &mut self.grok.open {
                    if k == kind && self.run.is_empty() && *at + 1 == self.done.len() {
                        text.push_str(piece);
                        self.done[*at].kind = grok_text(kind, text);
                        return;
                    }
                }
                if piece.trim().is_empty() {
                    return;
                }
                self.flush();
                self.done.push(ConversationItem {
                    id: bid,
                    kind: grok_text(kind, piece),
                    timestamp: self.line_ts,
                });
                self.grok.open = Some((kind.into(), self.done.len() - 1, piece.into()));
            }
            "tool_call" => {
                self.grok.open = None;
                let Some(id) = s_of(u, "toolCallId") else {
                    return;
                };
                let tool = u.get("_meta").and_then(|m| m.get("x.ai/tool"));
                let empty = Obj::new();
                let input = u
                    .get("rawInput")
                    .and_then(Value::as_object)
                    .unwrap_or(&empty);
                let name = tool
                    .and_then(|t| t.get("name"))
                    .and_then(Value::as_str)
                    .or_else(|| s_of(u, "title"))
                    .unwrap_or("tool");
                if name == "ask_user_question"
                    || tool.and_then(|t| t.get("kind")).and_then(Value::as_str) == Some("ask_user")
                {
                    if let Some(q) = question::grok(id, input) {
                        self.flush();
                        self.question_at.insert(id.into(), self.done.len());
                        self.done.push(ConversationItem {
                            id: format!("q-{id}"),
                            kind: ItemKind::Question(q),
                            timestamp: self.line_ts,
                        });
                        return;
                    }
                }
                let kind = tool.and_then(|t| t.get("kind")).and_then(Value::as_str);
                let name = match name {
                    "run_terminal_command" | "monitor" | "get_command_or_subagent_output" => "Bash",
                    "read_file" => "Read",
                    "search_replace" => "Edit",
                    "write" => "Write",
                    "grep" => "Grep",
                    "list_dir" => "Glob",
                    "web_fetch" => "WebFetch",
                    "web_search" => "WebSearch",
                    "spawn_subagent" => "Agent",
                    "todo_write" => "TodoWrite",
                    n => match kind {
                        Some("execute") => "Bash",
                        Some("read") => "Read",
                        Some("edit") => "Edit",
                        Some("search") => "Grep",
                        Some("fetch") => "WebFetch",
                        _ => n,
                    },
                };
                let summary = [
                    "command",
                    "target_file",
                    "file_path",
                    "pattern",
                    "target_directory",
                    "url",
                    "description",
                    "prompt",
                ]
                .iter()
                .find_map(|k| s_of(input, k).filter(|s| !s.is_empty()))
                .map(str::to_owned)
                .unwrap_or_else(|| summary(name, input));
                if self.run.is_empty() {
                    self.run_key = bid;
                }
                self.run.push(RunEntry {
                    tool: name.into(),
                    label: label_for(name),
                    id: id.into(),
                });
                if let Some(ts) = self.line_ts {
                    self.call_ts.insert(id.into(), ts);
                }
                self.open_tool_uses.insert(id.into(), summary.clone());
                self.open_order.push(id.into());
                self.call_by_id.insert(
                    id.into(),
                    ToolCall {
                        id: id.into(),
                        tool_name: name.into(),
                        summary,
                        input: cut(&pretty_json(input)),
                        output: None,
                        is_error: false,
                    },
                );
            }
            "tool_call_update" => {
                self.grok.open = None;
                let Some(id) = s_of(u, "toolCallId") else {
                    return;
                };
                let status = s_of(u, "status").unwrap_or("");
                if !matches!(status, "completed" | "failed") {
                    return;
                }
                let mut text = u
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.get("content")?.get("text")?.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.is_empty() {
                    text = u
                        .get("rawOutput")
                        .and_then(|v| v.get("output_for_prompt"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into();
                }
                if let Some(at) = self.question_at.remove(id) {
                    if let ItemKind::Question(q) = &mut self.done[at].kind {
                        q.state =
                            question::grok_state(q, status == "failed", u.get("rawOutput"), &text);
                    }
                    return;
                }
                if let Some(c) = self.call_by_id.get_mut(id) {
                    c.output = Some(cut(&text));
                    c.is_error = status == "failed";
                }
                self.open_tool_uses.remove(id);
                self.open_order.retain(|s| s != id);
            }
            "turn_completed" => {
                self.grok.open = None;
                self.open_tool_uses.clear();
                self.open_order.clear();
                for (_, at) in self.question_at.drain() {
                    if let ItemKind::Question(q) = &mut self.done[at].kind {
                        q.state = QuestionState::Declined;
                    }
                }
                if s_of(u, "stop_reason") == Some("cancelled") {
                    self.flush();
                    self.done.push(ConversationItem {
                        id: bid,
                        kind: ItemKind::Status("Interrupted".into()),
                        timestamp: self.line_ts,
                    });
                }
            }
            "agent_thought_chunk" => self.grok.open = None,
            _ => {}
        }
    }
}
fn grok_text(kind: &str, text: &str) -> ItemKind {
    if kind == "user_message_chunk" {
        ItemKind::UserText(text.trim().into())
    } else {
        ItemKind::AssistantText(text.trim().into())
    }
}
