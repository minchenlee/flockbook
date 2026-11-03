import Foundation

/// Permission-prompt detection. The kit reports unanswered tool calls (`PermissionRequest.state == .pending`);
/// only the screen tells a prompt from a running tool.
public enum PermissionGate {
    /// Text Claude Code draws on its permission prompt.
    public static let promptMarker = "Do you want"

    /// The whole prompt shape, not just the words: a line "Do you want …?" followed within a few lines by the option
    /// "1. Yes", and no question-picker footer ("Type something", "Chat about this"). "1" answers either, so a question
    /// menu read as an approval would pick its first option. Grok Build's prompt is its own box
    /// (`ApprovalNotice.grokBox`).
    public static func screenShowsPrompt(_ screenText: String) -> Bool {
        if ApprovalNotice.grokBox(screenText) != nil { return true }
        if screenText.contains("Chat about this") || screenText.contains("Type something") { return false }
        let lines = screenText.components(separatedBy: "\n").map { $0.trimmingCharacters(in: CharacterSet(charactersIn: "│|┃ \t")) }
        let yes = /^(?:❯\s*)?1\.\s+Yes\b.*/
        for (i, l) in lines.enumerated() where l.hasPrefix(promptMarker) {
            // A long file name wraps the question: it ends with "?" within three lines.
            guard let end = (i..<min(i + 3, lines.count)).first(where: { lines[$0].hasSuffix("?") }) else { continue }
            let next = lines[(end + 1)...].filter { !$0.isEmpty }.prefix(3)
            if next.contains(where: { $0.wholeMatch(of: yes) != nil }) { return true }
        }
        return false
    }

    /// Whether to read the screen for a permission prompt. hc: its 7501 status is the source: only `blocked`, and
    /// never when the kind says `question`; no kind lets the screen decide. TUI: blocked, or a tool call pending in the log.
    public static func checkApproval(_ agent: Agent?, pendingTool: Bool) -> Bool {
        guard let agent else { return pendingTool }
        if agent.backend == .hc { return agent.status == .blocked && agent.statusKind != "question" }
        return agent.status == .blocked || pendingTool
    }

    /// The pane an approval card belongs to: a blocked pane whose screen shows the prompt (`promptVisible`). The card's
    /// text comes from this pane's log and its buttons answer this pane, never another blocked pane of the same tab.
    public static func cardPane(_ panes: [Pane], promptVisible: Set<String>) -> Pane? {
        panes.first { $0.agent?.status == .blocked && promptVisible.contains($0.id) }
    }

    /// Whether to read the screen for Claude's question picker: a blocked Claude agent; on hc not when 7501 says `permission`.
    public static func readQuestion(_ agent: Agent?) -> Bool {
        guard let agent, agent.status == .blocked, agent.kind == "claude" else { return false }
        return agent.backend == .tui || agent.statusKind != "permission"
    }
}

extension PaneScreen {
    /// Read-only: does the pane's visible screen show a permission prompt right now?
    /// Show the Approval card + Allow/Deny only when true; otherwise show a "Running <tool>…" chip.
    /// Send approval keys only when true.
    public func checkPrompt(paneId: String) async -> Bool {
        guard let text = try? await visibleText(paneId: paneId) else { return false }
        return PermissionGate.screenShowsPrompt(text)
    }

    /// Answers a permission prompt only if it is still on screen: a stored "prompt visible" flag can be seconds old, and
    /// hc may hold the keys for its typing-quiet rule, so the keys could land in a different prompt. Reads the screen
    /// again right before the keys; `.promptGone` means nothing was sent.
    public func answerPrompt(paneId: String, allow: Bool) async -> PromptAnswer {
        guard await checkPrompt(paneId: paneId) else { return .promptGone }
        do { try await respond(to: paneId, allow: allow); return .sent } catch { return .failed }
    }
}

public enum PromptAnswer: Sendable, Equatable { case sent, promptGone, failed }
