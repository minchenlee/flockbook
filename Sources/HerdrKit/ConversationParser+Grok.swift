import Foundation

/// Grok Build (`<GROK_HOME>/sessions/<cwd>/<id>/updates.jsonl`). Each line is an ACP session update:
/// `{timestamp, method: "session/update" | "_x.ai/session/update", params: {sessionId, update: {sessionUpdate, …}, _meta: {eventId, agentTimestampMs}}}`.
/// Text arrives as `*_chunk` updates; adjacent chunks of one kind are one message. Thinking is hidden. A `tool_call`
/// opens a call, `tool_call_update` with a final status gives its result, and `turn_completed` closes what is still open.
/// `ask_user_question` is a question card (`QuestionAdapter.grok`), answered by its final update.
extension ConversationParser {
    struct GrokState {
        /// The text item the next chunk of the same kind continues: its kind, index in `done`, and raw text so far.
        var open: (kind: String, at: Int, text: String)?
    }

    static func grokUpdate(_ o: [String: Any]) -> (update: [String: Any], meta: [String: Any])? {
        guard let m = o["method"] as? String, m == "session/update" || m == "_x.ai/session/update",
              let p = o["params"] as? [String: Any], let u = p["update"] as? [String: Any], u["sessionUpdate"] is String else { return nil }
        return (u, p["_meta"] as? [String: Any] ?? [:])
    }

    mutating func feedGrok(_ u: [String: Any], meta: [String: Any], top: [String: Any], offset: UInt64?) {
        if let ms = meta["agentTimestampMs"] as? Double { lineTS = Date(timeIntervalSince1970: ms / 1000) }
        else if let s = top["timestamp"] as? Double { lineTS = Date(timeIntervalSince1970: s) }
        codexSeq += 1
        let bid = (meta["eventId"] as? String).map { "gk-\($0)" } ?? offset.map { "gk@\($0)" } ?? "gk\(codexSeq)"
        let kind = u["sessionUpdate"] as? String ?? ""
        switch kind {
        case "user_message_chunk", "agent_message_chunk":
            let content = u["content"] as? [String: Any]
            guard content?["type"] as? String == "text", let piece = content?["text"] as? String else { return }
            if let o = grok.open, o.kind == kind, run.isEmpty, o.at == done.count - 1 {
                let text = o.text + piece
                grok.open = (kind, o.at, text)
                done[o.at].kind = Self.grokText(kind, text)
                return
            }
            guard !piece.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
            flush()
            done.append(ConversationItem(id: bid, kind: Self.grokText(kind, piece), timestamp: lineTS))
            grok.open = (kind, done.count - 1, piece)
        case "tool_call":
            grok.open = nil
            guard let id = u["toolCallId"] as? String else { return }
            let tool = (u["_meta"] as? [String: Any])?["x.ai/tool"] as? [String: Any]
            let input = u["rawInput"] as? [String: Any] ?? [:]
            if tool?["name"] as? String == "ask_user_question" || tool?["kind"] as? String == "ask_user",
               let q = QuestionAdapter.grok(callId: id, input: input) {   // a question card, not a tool chip
                flush()
                questionAt[id] = done.count
                done.append(ConversationItem(id: "q-\(id)", kind: .question(q), timestamp: lineTS))
                return
            }
            let name = Self.grokTool(tool?["name"] as? String ?? u["title"] as? String ?? "tool", kind: tool?["kind"] as? String, input: input)
            let summary = Self.grokSummary(name: name, input: input)
            if run.isEmpty { runKey = bid }
            run.append((name, Self.label(for: name), id))
            callTS[id] = lineTS
            openToolUses[id] = summary
            openOrder.append(id)
            callById[id] = ToolCall(id: id, toolName: name, summary: summary, input: Self.cut(Self.prettyJSON(input)))
            callSeq += 1; callById[id]?.seq = callSeq
        case "tool_call_update":
            grok.open = nil
            guard let id = u["toolCallId"] as? String else { return }
            let status = u["status"] as? String
            if let at = questionAt[id], case .question(var q) = done[at].kind {
                guard status == "completed" || status == "failed" else { return }
                let text = (u["content"] as? [[String: Any]] ?? []).compactMap { ($0["content"] as? [String: Any])?["text"] as? String }.joined(separator: "\n")
                q.state = QuestionAdapter.grokState(q, failed: status == "failed", output: u["rawOutput"], text: text)
                done[at].kind = .question(q)
                questionAt[id] = nil
                return
            }
            guard let status, status == "completed" || status == "failed" else { return }
            var out = (u["content"] as? [[String: Any]] ?? []).compactMap { ($0["content"] as? [String: Any])?["text"] as? String }.joined(separator: "\n")
            if out.isEmpty, let raw = u["rawOutput"] as? [String: Any], let s = raw["output_for_prompt"] as? String { out = s }
            callById[id]?.output = Self.cut(out)
            callById[id]?.isError = status == "failed"
            openToolUses[id] = nil
            openOrder.removeAll { $0 == id }
        case "turn_completed":
            grok.open = nil
            for id in openOrder { openToolUses[id] = nil }
            openOrder = []
            // A question still open when the turn ends was never answered.
            for (id, at) in questionAt {
                if case .question(var q) = done[at].kind { q.state = .declined; done[at].kind = .question(q) }
                questionAt[id] = nil
            }
            if u["stop_reason"] as? String == "cancelled" {
                flush()
                done.append(ConversationItem(id: bid, kind: .status("Interrupted"), timestamp: lineTS))
            }
        case "agent_thought_chunk":
            grok.open = nil
        default: break   // hook runs and other notices: not part of the conversation
        }
    }

    static func grokText(_ kind: String, _ raw: String) -> ConversationItem.Kind {
        let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        return kind == "user_message_chunk" ? .userText(t) : .assistantText(t)
    }

    /// Grok Build's tools under Claude Code's names, so chips read the same. Unknown names fall back on the tool kind.
    static func grokTool(_ name: String, kind: String?, input: [String: Any]) -> String {
        switch name {
        case "run_terminal_command", "monitor", "get_command_or_subagent_output": "Bash"
        case "read_file": "Read"
        case "search_replace": "Edit"
        case "write": "Write"
        case "grep": "Grep"
        case "list_dir": "Glob"
        case "web_fetch": "WebFetch"
        case "web_search": "WebSearch"
        case "spawn_subagent": "Agent"
        case "todo_write": "TodoWrite"
        case "use_tool": "mcp__\(input["tool_name"] as? String ?? "tool")"
        default:
            switch kind {
            case "execute": "Bash"
            case "read": "Read"
            case "edit": "Edit"
            case "search": "Grep"
            case "fetch": "WebFetch"
            default: name
            }
        }
    }

    static func grokSummary(name: String, input: [String: Any]) -> String {
        let keys: [String]
        switch name {
        case "Bash": keys = ["command"]
        case "Read": keys = ["target_file", "file_path"]
        case "Edit", "Write": keys = ["file_path", "target_file"]
        case "Grep": keys = ["pattern"]
        case "Glob": keys = ["target_directory"]
        case "WebFetch": keys = ["url"]
        case "Agent": keys = ["description", "prompt"]
        default: keys = []
        }
        for k in keys { if let s = input[k] as? String, !s.isEmpty { return s } }
        return summary(name: name, input: input)
    }
}
