import Foundation

/// agy, the Antigravity CLI (`~/.gemini/antigravity-cli/brain/<id>/.system_generated/logs/transcript_full.jsonl`).
/// One step per line: `{step_index, source, type, status, created_at, content, thinking?, tool_calls?}`.
/// `USER_INPUT` is the prompt (inside `<USER_REQUEST>`), `PLANNER_RESPONSE` is the reply and its `tool_calls`
/// (`{name, args}`, no call id), and the next `GENERIC` step is the result of the oldest open call.
/// `SYSTEM_MESSAGE` is injected text; `CHECKPOINT` marks truncated history. A call without a result is pending.
extension ConversationParser {
    static func isAgy(_ o: [String: Any]) -> Bool {
        o["uuid"] == nil && o["step_index"] is NSNumber && o["type"] is String && o["source"] is String
    }

    mutating func feedAgy(_ o: [String: Any]) {
        let step = (o["step_index"] as? NSNumber)?.intValue ?? 0
        let bid = "ag\(step)"
        let content = (o["content"] as? String ?? "")
        lineTS = Self.parseTime(o["created_at"] as? String) ?? lineTS
        switch o["type"] as? String {
        case "USER_INPUT":
            guard let text = Self.agyRequest(content), !text.isEmpty else { return }
            flush()
            done.append(ConversationItem(id: bid, kind: .userText(text), timestamp: lineTS))
        case "PLANNER_RESPONSE":
            let text = content.trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty {
                flush()
                done.append(ConversationItem(id: bid, kind: .assistantText(text), timestamp: lineTS))
            }
            for (i, c) in ((o["tool_calls"] as? [[String: Any]]) ?? []).enumerated() {
                let args = c["args"] as? [String: Any] ?? [:]
                let name = Self.agyTool(c["name"] as? String ?? "tool")
                let id = "\(bid)-\(i)"
                if run.isEmpty { runKey = id }
                run.append((name, Self.label(for: name), id))
                callTS[id] = lineTS
                openToolUses[id] = Self.agySummary(args, fallback: name)
                openOrder.append(id)
                callById[id] = ToolCall(id: id, toolName: name, summary: Self.agySummary(args, fallback: name),
                                        input: Self.cut(Self.prettyJSON(args)))
            }
        case "GENERIC":
            guard let id = openOrder.first else { return }
            openOrder.removeFirst()
            openToolUses[id] = nil
            callById[id]?.output = Self.cut(Self.agyResult(content))
            callById[id]?.isError = o["status"] as? String == "ERROR"
        case "CHECKPOINT":
            flush()
            done.append(ConversationItem(id: bid, kind: .status("Context compacted"), timestamp: lineTS))
        default: break   // SYSTEM_MESSAGE and others: injected, not shown
        }
    }

    /// The typed prompt: the `<USER_REQUEST>` body; plain text without tags as is; other tagged text is injected.
    static func agyRequest(_ raw: String) -> String? {
        if let a = raw.range(of: "<USER_REQUEST>") {
            let rest = raw[a.upperBound...]
            let body = rest.range(of: "</USER_REQUEST>").map { rest[..<$0.lowerBound] } ?? rest
            return body.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        return t.hasPrefix("<") ? nil : t
    }

    /// agy's tools under Claude Code's names, so chips and requests read the same.
    static func agyTool(_ name: String) -> String {
        switch name {
        case "run_command": "Bash"
        case "view_file", "read_file": "Read"
        case "write_to_file", "write_file": "Write"
        case "replace_file_content", "multi_replace_file_content": "Edit"
        case "grep_search", "find_by_name", "list_dir", "code_search": "Grep"
        case "read_url_content", "search_web", "execute_url": "WebFetch"
        case "invoke_subagent": "Agent"
        default: name
        }
    }

    /// What the call acts on: the command, the file, the query; else agy's own `toolSummary`.
    static func agySummary(_ args: [String: Any], fallback: String) -> String {
        for k in ["CommandLine", "AbsolutePath", "TargetFile", "Path", "DirectoryPath", "SearchPath", "Query", "Pattern", "Url", "toolSummary"] {
            if let s = args[k] as? String, !s.isEmpty { return s }
        }
        return fallback
    }

    /// Result text without the `Created At:` / `Completed At:` header lines and the tab indent.
    static func agyResult(_ raw: String) -> String {
        raw.split(separator: "\n", omittingEmptySubsequences: false)
            .filter { !$0.hasPrefix("Created At: ") && !$0.hasPrefix("Completed At: ") }
            .map { $0.drop { $0 == "\t" } }
            .joined(separator: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
