import Foundation

/// Cursor agent (`~/.cursor/projects/<slug>/agent-transcripts/<id>/<id>.jsonl`). Lines are `{role, message:{content}}`
/// and `{type:"turn_ended", status}`; no ids, no times, no tool results. The real prompt is inside `<user_query>`;
/// user lines without one are injected context. Tool calls become chips without output (results live only in
/// Cursor's store.db) and never a pending item.
extension ConversationParser {
    static func isCursor(_ o: [String: Any]) -> Bool {
        o["uuid"] == nil && ((o["type"] == nil && o["role"] is String && o["message"] != nil) || o["type"] as? String == "turn_ended")
    }

    mutating func feedCursor(_ o: [String: Any], offset: UInt64?) {
        codexSeq += 1
        let bid = offset.map { "cu@\($0)" } ?? "cu\(codexSeq)"
        if o["type"] as? String == "turn_ended" {
            let status = o["status"] as? String
            guard status == "error" || status == "aborted" else { return }
            flush()
            done.append(ConversationItem(id: bid, kind: .status(status == "error" ? "Error" : "Interrupted"), timestamp: lineTS))
            return
        }
        let role = o["role"] as? String
        let blocks = (o["message"] as? [String: Any])?["content"] as? [[String: Any]] ?? []
        for (i, b) in blocks.enumerated() {
            let id = "\(bid)-\(i)"
            switch b["type"] as? String {
            case "text":
                let raw = b["text"] as? String ?? ""
                if role == "user" {
                    guard let text = Self.cursorQuery(raw), !text.isEmpty else { continue }
                    lineTS = Self.cursorTime(raw) ?? lineTS
                    flush()
                    done.append(ConversationItem(id: id, kind: .userText(text), timestamp: lineTS))
                } else if role == "assistant" {
                    let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
                    if text.isEmpty { continue }
                    flush()
                    done.append(ConversationItem(id: id, kind: .assistantText(text), timestamp: nil))
                }
            case "tool_use":
                guard role == "assistant" else { continue }
                let input = b["input"] as? [String: Any] ?? [:]
                let name = Self.cursorTool(b["name"] as? String ?? "tool", input: input)
                if run.isEmpty { runKey = id }
                run.append((name, Self.label(for: name), id))
                callById[id] = ToolCall(id: id, toolName: name, summary: Self.summary(name: name, input: input), input: Self.cut(Self.prettyJSON(input)))
                callSeq += 1; callById[id]?.seq = callSeq
            default: break
            }
        }
    }

    /// The typed prompt: the `<user_query>` body; plain text without tags as is; other tagged text is injected context.
    static func cursorQuery(_ raw: String) -> String? {
        if let a = raw.range(of: "<user_query>") {
            let rest = raw[a.upperBound...]
            let body = rest.range(of: "</user_query>").map { rest[..<$0.lowerBound] } ?? rest
            return body.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        return t.hasPrefix("<") || t.hasPrefix("[") ? nil : t
    }

    /// `<timestamp>Saturday, Sep 12, 2026, 4:09 PM (UTC+8)</timestamp>` (minute precision).
    static func cursorTime(_ raw: String) -> Date? {
        guard let m = raw.firstMatch(of: /<timestamp>\w+, (\w{3}) (\d{1,2}), (\d{4}), (\d{1,2}):(\d{2}) (AM|PM) \(UTC(?:([+-])(\d{1,2})(?::(\d{2}))?)?\)<\/timestamp>/),
              let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].firstIndex(of: String(m.1)),
              let day = Int(m.2), let year = Int(m.3), var hour = Int(m.4), let minute = Int(m.5) else { return nil }
        if hour == 12 { hour = 0 }
        if m.6 == "PM" { hour += 12 }
        let hours: Int = m.8.flatMap { Int($0) } ?? 0
        let mins: Int = m.9.flatMap { Int($0) } ?? 0
        let sign: Int = m.7 == "-" ? -1 : 1
        let off: Int = (hours * 3600 + mins * 60) * sign
        var cal = Calendar(identifier: .gregorian)
        guard let tz = TimeZone(secondsFromGMT: off) else { return nil }
        cal.timeZone = tz
        return cal.date(from: DateComponents(year: year, month: month + 1, day: day, hour: hour, minute: minute))
    }

    /// Cursor's tools under Claude Code's names, so chips read the same.
    static func cursorTool(_ name: String, input: [String: Any]) -> String {
        switch name {
        case "Shell", "AwaitShell": "Bash"
        case "StrReplace": "Edit"
        case "Task": "Agent"
        case "CallMcpTool", "CallDynamicTool": "mcp__\(input["server"] as? String ?? input["namespace"] as? String ?? "cursor")__\(input["toolName"] as? String ?? "tool")"
        default: name
        }
    }
}
