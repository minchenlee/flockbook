import Foundation

/// agy, the Antigravity CLI (`~/.gemini/antigravity-cli/brain/<id>/.system_generated/logs/transcript_full.jsonl`).
/// One step per line: `{step_index, source, type, status, created_at, content, thinking?, tool_calls?}`.
/// `USER_INPUT` is the prompt (inside `<USER_REQUEST>`), `PLANNER_RESPONSE` is the reply and its `tool_calls`
/// (`{name, args}`, no call id). Call i of step p gets its result in the `GENERIC` step p+1+i; lines can come out of
/// step order (a result line before its call), so results pair by `step_index`, not by line order.
/// `SYSTEM_MESSAGE` is injected text; `CHECKPOINT` marks truncated history. A call without a result is pending.
/// `ask_question` is a question card (result `A1: <answer>`). A denied call never gets a result (seen 2026-11-02):
/// a later step past its slot, or the next `USER_INPUT`, closes it as not run (a late result still fills it).
struct AgyState: Sendable {
    var slot: [Int: String] = [:]                    // result step -> call id, until the result arrives
    var slotOf: [String: Int] = [:]                  // call id -> result step, while the call is open
    var early: [Int: (text: String, error: Bool)] = [:]   // results whose call line has not come yet
}

extension ConversationParser {
    static func isAgy(_ o: [String: Any]) -> Bool {
        o["uuid"] == nil && o["step_index"] is NSNumber && o["type"] is String && o["source"] is String
    }

    mutating func feedAgy(_ o: [String: Any]) {
        let step = (o["step_index"] as? NSNumber)?.intValue ?? 0
        let bid = "ag\(step)"
        let content = (o["content"] as? String ?? "")
        lineTS = Self.parseTime(o["created_at"] as? String) ?? lineTS
        let type = o["type"] as? String
        if type != "GENERIC" { closeOpenAgyCalls(before: type == "USER_INPUT" ? nil : step) }
        switch type {
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
                let id = "\(bid)-\(i)"
                if c["name"] as? String == "ask_question", let q = QuestionAdapter.agy(id: id, input: args) {   // a card, never a permission
                    flush()
                    questionAt[id] = done.count
                    done.append(ConversationItem(id: "q-\(id)", kind: .question(q), timestamp: lineTS))
                    openAgySlot(id, at: step + 1 + i)
                    continue
                }
                let name = Self.agyTool(c["name"] as? String ?? "tool")
                if run.isEmpty { runKey = id }
                run.append((name, Self.label(for: name), id))
                callTS[id] = lineTS
                openToolUses[id] = Self.agySummary(args, fallback: name)
                callById[id] = ToolCall(id: id, toolName: name, summary: Self.agySummary(args, fallback: name),
                                        input: Self.cut(Self.prettyJSON(args)))
                openAgySlot(id, at: step + 1 + i)
            }
        case "GENERIC":
            let r = (text: Self.agyResult(content), error: o["status"] as? String == "ERROR")
            guard let id = agy.slot.removeValue(forKey: step) else { agy.early[step] = r; return }
            fillAgy(id, r)
        case "CHECKPOINT":
            flush()
            done.append(ConversationItem(id: bid, kind: .status("Context compacted"), timestamp: lineTS))
        default: break   // SYSTEM_MESSAGE and others: injected, not shown
        }
    }

    /// A call waits for its result in step `at`; a result line that came first fills it now.
    private mutating func openAgySlot(_ id: String, at: Int) {
        if let r = agy.early.removeValue(forKey: at) { fillAgy(id, r); return }
        agy.slot[at] = id
        agy.slotOf[id] = at
        openOrder.append(id)
    }

    /// The result of call `id` (also when it was closed as not run before).
    private mutating func fillAgy(_ id: String, _ r: (text: String, error: Bool)) {
        agy.slotOf[id] = nil
        openOrder.removeAll { $0 == id }
        if let at = questionAt[id], case .question(var q) = done[at].kind {
            questionAt[id] = nil
            q.state = QuestionAdapter.agyState(q, text: r.text)
            done[at].kind = .question(q)
            return
        }
        openToolUses[id] = nil
        callById[id]?.output = Self.cut(r.text)
        callById[id]?.isError = r.error
    }

    /// Open calls whose result step is before `before` (all for nil: a new prompt) were denied or interrupted: agy
    /// writes no result for them. Their slot stays, so a result line that comes late still fills them.
    private mutating func closeOpenAgyCalls(before: Int?) {
        for id in openOrder {
            guard let at = agy.slotOf[id], before.map({ at < $0 }) ?? true else { continue }
            agy.slotOf[id] = nil
            if let qi = questionAt[id], case .question(var q) = done[qi].kind {
                q.state = .declined
                done[qi].kind = .question(q)
            } else {
                callById[id]?.output = "Not run"
                callById[id]?.isError = true
                openToolUses[id] = nil
            }
        }
        openOrder.removeAll { agy.slotOf[$0] == nil }
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
