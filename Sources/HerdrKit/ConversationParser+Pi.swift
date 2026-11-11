import Foundation

/// pi (`~/.pi/agent/sessions/<cwd-slug>/<time>_<uuid>.jsonl`). Entries form a tree by `id`/`parentId`; the shown
/// conversation is the branch that ends at the newest entry. Lines are fed in file order; when an entry hangs off an
/// earlier entry (an edited or retried turn), the items are rebuilt from that branch. omp (oh-my-pi, a pi fork) writes
/// the same tree with more entry types, a `title` slot before the header, and `activeLeafId` on an entry written off
/// the shown branch (late usage, a background result): the shown branch stays at that leaf.
struct PiState: Sendable {
    var parent: [String: String] = [:]   // entry id -> parent id ("" = root)
    var lines: [String: String] = [:]    // entry id -> line, only for entries that render
    var offsets: [String: UInt64] = [:]  // entry id -> the line's byte offset (picture refs), when known
    var last: String?
    var replaying = false
}

extension ConversationParser {
    /// pi's entry types, then omp's (session-entries.ts).
    static let piTypes: Set<String> = ["session", "message", "model_change", "thinking_level_change", "custom", "custom_message",
                                       "session_info", "compaction", "context_edit", "branch_summary",
                                       "title", "title_change", "model_usage", "service_tier_change", "label", "session_init",
                                       "reset_boundary", "mode_change", "ttsr_injection", "credential_pin"]

    mutating func feedPi(_ o: [String: Any], line: String, offset: UInt64?) {
        // The header is not in the tree; omp's title slot has no id.
        guard o["type"] as? String != "session", let id = o["id"] as? String else { return }
        let parent = o["parentId"] as? String
        // omp: the leaf after this entry (absent: the entry; null: cleared). Replayed entries are on the branch.
        let leaf: String? = o["activeLeafId"].map { $0 as? String } ?? id
        if !pi.replaying, leaf != id {
            pi.parent[id] = parent ?? ""
            if Self.piRenders(o) { pi.lines[id] = line; pi.offsets[id] = offset }
            if let leaf {
                // Else the shown leaf, or one omp no longer has: the branch stays.
                if pi.last != leaf, pi.parent[leaf] != nil { rebuildPi(upTo: leaf) }
            } else {
                clearPi()
            }
            return
        }
        // An entry off the shown leaf (an edited turn, or the first one after omp cleared the leaf): its branch.
        if !pi.replaying, let parent, pi.last != parent, pi.parent[parent] != nil { rebuildPi(upTo: parent) }
        pi.parent[id] = parent ?? ""
        pi.last = id
        if !pi.replaying, Self.piRenders(o) { pi.lines[id] = line; pi.offsets[id] = offset }
        renderPi(o, id: id)
    }

    static func piRenders(_ o: [String: Any]) -> Bool {
        switch o["type"] as? String {
        case "message": (o["message"] as? [String: Any])?["role"] as? String != "system"
        case "compaction", "custom_message": true
        default: false
        }
    }

    /// omp cleared the leaf: nothing is shown, the tree stays for a later branch.
    private mutating func clearPi() {
        var fresh = ConversationParser()
        fresh.pi = pi
        fresh.pi.last = nil
        self = fresh
    }

    /// Items become the branch from the root to `leaf` (replayed from the kept lines).
    private mutating func rebuildPi(upTo leaf: String) {
        var chain: [String] = []
        var cur = leaf
        while !cur.isEmpty, chain.count <= pi.parent.count, let up = pi.parent[cur] { chain.append(cur); cur = up }
        var fresh = ConversationParser()
        fresh.pi = pi
        fresh.pi.replaying = true
        for id in chain.reversed() { if let l = pi.lines[id] { fresh.feed(line: l, offset: pi.offsets[id]) } }
        fresh.pi.replaying = false
        fresh.pi.last = leaf
        self = fresh
    }

    private mutating func renderPi(_ o: [String: Any], id: String) {
        switch o["type"] as? String {
        case "compaction":
            flush()
            done.append(ConversationItem(id: id, kind: .status("Context compacted"), timestamp: lineTS))
        case "custom_message":   // extension notices shown in pi's TUI (e.g. a background subagent finished)
            guard o["display"] as? Bool == true else { return }
            let text = Self.resultText(o["content"]).trimmingCharacters(in: .whitespacesAndNewlines)
            if text.hasPrefix("<system-notice>") { return }   // omp: a background result handed to the model, not a notice
            guard let first = text.split(whereSeparator: \.isNewline).first else { return }
            flush()
            done.append(ConversationItem(id: id, kind: .status(String(first.prefix(160))), timestamp: lineTS))
        case "message":
            guard let m = o["message"] as? [String: Any] else { return }
            renderPiMessage(m, id: id)
        default: break
        }
    }

    private mutating func renderPiMessage(_ m: [String: Any], id: String) {
        let blocks = m["content"] as? [[String: Any]] ?? (m["content"] as? String).map { [["type": "text", "text": $0]] } ?? []
        switch m["role"] as? String {
        case "user":
            for (i, b) in blocks.enumerated() where b["type"] as? String == "text" {
                let text = (b["text"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                if text.isEmpty { continue }
                flush()
                done.append(ConversationItem(id: "\(id)-\(i)", kind: .userText(text), timestamp: lineTS))
            }
        case "assistant":
            for (i, b) in blocks.enumerated() {
                switch b["type"] as? String {
                case "text":
                    let text = (b["text"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                    if text.isEmpty { continue }
                    flush()
                    done.append(ConversationItem(id: "\(id)-\(i)", kind: .assistantText(text), timestamp: lineTS))
                case "toolCall":
                    let name = Self.piTool(b["name"] as? String ?? "tool")
                    let input = b["arguments"] as? [String: Any] ?? [:]
                    openPiCall(id: b["id"] as? String ?? "\(id)-\(i)", name: name, input: input, key: "\(id)-\(i)")
                default: break   // thinking: hidden, as for Claude Code
                }
            }
            switch m["stopReason"] as? String {
            case "error":
                let msg = (m["errorMessage"] as? String ?? "").split(whereSeparator: \.isNewline).first.map(String.init) ?? ""
                flush()
                done.append(ConversationItem(id: "\(id)-err", kind: .status(msg.isEmpty ? "Error" : "Error: \(msg.prefix(160))"), timestamp: lineTS))
            case "aborted":
                flush()
                done.append(ConversationItem(id: "\(id)-err", kind: .status("Interrupted"), timestamp: lineTS))
            default: break
            }
        case "toolResult":
            guard let call = m["toolCallId"] as? String else { return }
            callById[call]?.output = Self.cut(Self.resultText(m["content"]))
            callById[call]?.isError = m["isError"] as? Bool ?? false
            callById[call]?.images = LogImage.refs(LogImage.found(message: m), offset: lineOffset).map(\.ref)
            openToolUses[call] = nil
            openOrder.removeAll { $0 == call }
        case "bashExecution":   // the user ran `!command` in pi
            let cmd = m["command"] as? String ?? ""
            openPiCall(id: id, name: "Bash", input: ["command": cmd], key: id)
            callById[id]?.output = Self.cut(m["output"] as? String ?? "")
            callById[id]?.isError = (m["exitCode"] as? Int ?? 0) != 0
            openToolUses[id] = nil
            openOrder.removeAll { $0 == id }
        default: break   // system prompt
        }
    }

    private mutating func openPiCall(id: String, name: String, input: [String: Any], key: String) {
        if run.isEmpty { runKey = key }
        run.append((name, Self.label(for: name), id))
        callTS[id] = lineTS
        openToolUses[id] = Self.commandText(name: name, input: input)
        openOrder.append(id)
        callById[id] = ToolCall(id: id, toolName: name, summary: Self.summary(name: name, input: input), input: Self.cut(Self.prettyJSON(input)))
    }

    /// pi's built-in tools under Claude Code's names, so chips and verbs read the same.
    static func piTool(_ name: String) -> String {
        switch name {
        case "bash": "Bash"
        case "read": "Read"
        case "edit": "Edit"
        case "write": "Write"
        case "grep": "Grep"
        case "find", "ls": "Glob"
        case "subagent", "subagent_start": "Agent"
        default: name
        }
    }
}
