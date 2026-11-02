import Foundation

/// Allow / Deny buttons on the "needs you" system notification for a permission prompt.
///
/// Buttons only for a permission prompt on screen (`PermissionGate`): input is on and the agent is blocked. The id is the
/// log's pending tool call, or the prompt box on screen when there is none (a Claude without a transcript). Claude Code and
/// Grok Build only: their keys are verified (`ApprovalKeys`, `grokKey`). A question picker never gets buttons (the tab opens). An action re-reads
/// the pane first and answers only the prompt the notification was posted for; otherwise the tab opens.
public enum ApprovalNotice {
    public static let category = "approval"

    public enum Action: String, CaseIterable, Sendable {
        case allow = "approval.allow"
        case deny = "approval.deny"
        /// The `allow` argument of `HerdrClient.respond`, the call the approval card makes.
        public var allows: Bool { self == .allow }
    }

    /// The blocked pane at one moment: what the app knows plus the visible screen.
    public struct PaneState: Sendable, Hashable {
        public var agentKind: String
        public var blocked: Bool
        /// The tool call the log ends with, still without a result.
        public var pendingToolUseId: String?
        public var screen: String
        /// The pending call's tool and command, to match it with the prompt box on screen.
        public var pendingTool: String?
        public var pendingCommand: String?
        public init(agentKind: String, blocked: Bool, pendingToolUseId: String?, screen: String, pendingTool: String? = nil, pendingCommand: String? = nil) {
            self.agentKind = agentKind; self.blocked = blocked; self.pendingToolUseId = pendingToolUseId; self.screen = screen
            self.pendingTool = pendingTool; self.pendingCommand = pendingCommand
        }
    }

    /// The ids of the prompt the pane shows now. Empty: no buttons. Always the prompt box on screen; the log's pending
    /// tool call too, but only when it is that box (same tool, same command): a log that lags the screen still names
    /// the previous call, and its id must never answer the next prompt. The screen id covers a Claude without a transcript.
    static func promptIDs(_ s: PaneState, writes: Bool) -> [String] {
        if s.agentKind == "grok" {
            // Grok's log never names a prompt; its open call is the prompt when it runs the box's command.
            guard writes, s.blocked, let box = grokBox(s.screen) else { return [] }
            let onScreen = grokID(box)
            guard let logged = s.pendingToolUseId, !logged.isEmpty, !box.command.isEmpty,
                  sameCall(tool: s.pendingTool, command: s.pendingCommand, "Bash", box.command) else { return [onScreen] }
            return [logged, onScreen]
        }
        guard writes, s.blocked, s.agentKind == "claude",
              QuestionAdapter.claudeScreen(s.screen) == nil,
              let box = screenBox(s.screen), let onScreen = screenPrompt(s.screen) else { return [] }
        guard let logged = s.pendingToolUseId, !logged.isEmpty, sameCall(tool: s.pendingTool, command: s.pendingCommand, box.tool, box.command) else { return [onScreen] }
        return [logged, onScreen]
    }

    /// The log's call is the prompt box: same tool, same non-empty command. The box wraps a long command, mid-word or
    /// at a space, so each line boundary may stand for one space or none; inside a line, spaces must match (as runs).
    private static func sameCall(tool: String?, command: String?, _ boxTool: String, _ boxCommand: String) -> Bool {
        guard let tool, let command, !words(command).isEmpty, tool == boxTool else { return false }
        var rest = Substring(words(command))
        for (i, line) in boxCommand.components(separatedBy: "\n").map(words).filter({ !$0.isEmpty }).enumerated() {
            if i > 0, rest.hasPrefix(" ") { rest = rest.dropFirst() }
            guard rest.hasPrefix(line) else { return false }
            rest = rest.dropFirst(line.count)
        }
        return rest.isEmpty
    }

    /// Whitespace-insensitive: the box wraps and indents a long command.
    private static func words(_ s: String) -> String { s.split(whereSeparator: \.isWhitespace).joined(separator: " ") }

    /// The prompt a notification's buttons answer: the pending tool call's id, else the prompt on screen. Nil: no buttons.
    public static func prompt(_ s: PaneState, writes: Bool) -> String? { promptIDs(s, writes: writes).first }

    /// Claude's permission prompt box on screen. `tool` = the first word of its header ("Bash command" -> "Bash").
    /// `lines` = the whole box, trimmed: from below the box's top rule (or the screen top) down to the last option, without
    /// what changes while the prompt stays: blank lines, the rotating "Tip:" line, the auto-deny countdown and the cursor `❯`.
    struct ScreenBox: Sendable, Hashable {
        var header: String
        var tool: String
        var command: String
        var lines: [String]
        /// The top of the box is off screen: no header, tool or command (see `screenBox`).
        var clipped = false
    }

    /// "⚠ Claude Code will automatically deny this request in 1:23, …": its time changes every second.
    static func isCountdown(_ line: String) -> Bool {
        line.hasPrefix("⚠") && line.firstMatch(of: /\b\d{1,2}:\d{2}\b/) != nil
    }

    static func screenBox(_ screen: String) -> ScreenBox? {
        let raw = screen.components(separatedBy: "\n")
        guard let ask = raw.lastIndex(where: { $0.contains(PermissionGate.promptMarker) }),
              QuestionAdapter.claudeScreen(screen) == nil else { return nil }
        let trim: (String) -> String = { $0.trimmingCharacters(in: .whitespaces) }
        let rule = raw[..<ask].lastIndex { trim($0).hasPrefix("─") }
        let top = rule.map { $0 + 1 } ?? 0
        // Options: down to the first blank line or the key help line.
        var end = ask
        while end + 1 < raw.count, !trim(raw[end + 1]).isEmpty, !trim(raw[end + 1]).hasPrefix("Esc to cancel") { end += 1 }
        let box = raw[top...end].map { l -> String in
            let t = trim(l)
            return t.hasPrefix("❯") ? trim(String(t.dropFirst())) : t
        }.filter { !$0.isEmpty && !$0.hasPrefix("Tip:") && !isCountdown($0) }
        guard let header = box.first, header != box.last else { return nil }
        let isRule: (String) -> Bool = { $0.hasPrefix("╌") }
        let askAt = box.lastIndex { $0.contains(PermissionGate.promptMarker) } ?? box.count
        let body = Array(box[1..<askAt])
        let command: [String]
        if let r1 = body.firstIndex(where: isRule), let r2 = body[(r1 + 1)...].firstIndex(where: isRule) {
            command = Array(body[(r1 + 1)..<r2])
        } else {
            command = body.filter { !isRule($0) }
        }
        // A wrapped command has a "│" gutter on each line (Claude Code 2.1.296).
        let ungutter: (String) -> String = { $0.hasPrefix("│") ? $0.dropFirst().trimmingCharacters(in: .whitespaces) : $0 }
        // The box is taller than the pane and its top scrolled off into the command: no top rule, and the screen starts
        // with a "│" command line or keeps only the command's closing "╌" rule. The first line is then a line of the
        // command, not the header, and the command shown would be its end only: neither is read (`clipped`). `lines`
        // stays the visible box, so the prompt keeps its id and its buttons. (A box with no rule at all that starts at
        // its header, as older layouts drew it, is read as before.)
        if rule == nil, header.hasPrefix("│") || body.filter(isRule).count == 1 {
            return ScreenBox(header: "", tool: "", command: "", lines: box, clipped: true)
        }
        let tool = String(header.split(separator: " ").first ?? "")
        return ScreenBox(header: header, tool: tool, command: command.map(ungutter).joined(separator: "\n"), lines: box)
    }

    /// Id of the permission prompt on screen: a hash of the tool name and the whole box (header, command, question and
    /// options), so another tool, command or option list is another prompt. Stable across launches. Nil: no prompt.
    public static func screenPrompt(_ screen: String) -> String? {
        guard let box = screenBox(screen) else { return nil }
        return hashID("tool:\(box.tool)\n" + box.lines.joined(separator: "\n"))
    }

    static func hashID(_ text: String) -> String {
        var h: UInt64 = 0xcbf29ce484222325   // FNV-1a (Hasher is seeded per launch)
        for b in text.utf8 { h = (h ^ UInt64(b)) &* 0x100000001b3 }
        return "screen-" + String(h, radix: 16)
    }

    /// The card's title for a box whose top is off screen: the command is not shown, the terminal has it.
    public static let clippedTitle = "Command too long to show here. Read it in the terminal before you answer."

    /// The approval card's request read from the screen, for a prompt the log does not have (no transcript, or the log
    /// lags). Its `toolUseId` is `screenPrompt`.
    public static func screenRequest(_ screen: String, agent: String = "claude") -> PermissionRequest? {
        if agent == "grok" {
            guard let box = grokBox(screen) else { return nil }
            return PermissionRequest(title: box.title, command: box.command, toolName: box.command.isEmpty ? "" : "Bash", toolUseId: grokID(box), state: .pending)
        }
        guard let box = screenBox(screen), let id = screenPrompt(screen) else { return nil }
        return PermissionRequest(title: box.clipped ? clippedTitle : box.header, command: box.command, toolName: box.tool, toolUseId: id, state: .pending)
    }

    /// Grok Build 1.0.67's permission prompt (live): its last "┃" block holds "› <what it does>", the command on "│"
    /// lines, then rows "N (○) text", (●) on the row with focus; right below the block its key line "1/N:select …
    /// Ctrl+c:cancel". `lines`: the block with the focus marks dropped (the prompt stays the same while focus moves).
    struct GrokBox: Sendable, Hashable {
        var title: String
        var command: String
        var rows: [String]
        var focus: Int
        var lines: [String]
        var yes: Int? { rows.firstIndex(of: "Yes, proceed") }
        /// The row as Grok draws it while empty; typed feedback changes it, and that text is the user's.
        var no: Int? { rows.firstIndex(of: "No, reject (type to add feedback)") }
    }

    static func grokBox(_ screen: String) -> GrokBox? {
        let raw = screen.components(separatedBy: "\n")
        guard let block = QuestionAdapter.grokBlock(raw),
              let key = raw[block.upperBound...].first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty })?.trimmingCharacters(in: .whitespaces),
              key.firstMatch(of: /^1\/\d+:select\s/) != nil, key.contains("Ctrl+c:cancel") else { return nil }
        var title: String?, command: [String] = [], rows: [String] = [], focus: [Int] = [], lines: [String] = []
        for t in QuestionAdapter.grokLines(raw[block]) {
            if let m = t.wholeMatch(of: /^(\d+) \(([○●])\) (.*)$/) {
                guard Int(m.1) == rows.count + 1 else { return nil }
                if m.2 == "●" { focus.append(rows.count) }
                rows.append(String(m.3))
                lines.append("\(m.1) \(m.3)")
            } else {
                guard rows.isEmpty else { return nil }   // text below the rows: not a layout this reads
                if title == nil, t.hasPrefix("›") { title = t.dropFirst().trimmingCharacters(in: .whitespaces) }
                else if t.hasPrefix("│") { command.append(t.dropFirst().trimmingCharacters(in: .whitespaces)) }
                lines.append(t)
            }
        }
        guard let title, focus.count == 1, rows.contains("Yes, proceed"), rows.contains(where: { $0.hasPrefix("No, reject") }) else { return nil }
        return GrokBox(title: title, command: command.joined(separator: "\n"), rows: rows, focus: focus[0], lines: lines)
    }

    static func grokID(_ box: GrokBox) -> String { hashID("tool:grok\n" + box.lines.joined(separator: "\n")) }

    /// The next key for Allow or Deny on Grok's prompt, or nil: send nothing (answer in the terminal). Enter answers
    /// the row with focus (live): Allow only from "Yes, proceed"; Deny from "No, reject", or Down first when that row
    /// is right below "Yes, proceed". The always-approve and never-allow rows change the user's settings: never chosen.
    static func grokKey(_ screen: String, allow: Bool) -> String? {
        guard let b = grokBox(screen), let yes = b.yes else { return nil }
        if allow { return b.focus == yes ? "Enter" : nil }
        guard let no = b.no else { return nil }
        if b.focus == no { return "Enter" }
        return b.focus == yes && no == yes + 1 ? "Down" : nil
    }

    /// The request the in-app card shows and binds its buttons to: the log's call when it is the box on screen, else
    /// the box (a log that lags the screen still names the previous call). No box read: the log's call; its keys then
    /// wait for the box (`respondIfOpen`).
    public static func cardRequest(logged: PermissionRequest?, onScreen: PermissionRequest?) -> PermissionRequest? {
        guard let onScreen else { return logged }
        guard let logged, sameCall(tool: logged.toolName, command: logged.command, onScreen.toolName, onScreen.command) else { return onScreen }
        return logged
    }

    /// Where the in-app approval card with live buttons stands. One card per pane, bound to `cardRequest`'s id, the same
    /// id the tab's card and the notification answer.
    public enum CardPlace: Sendable, Hashable {
        /// The log's pending call, in the conversation (it is the card's request).
        case conversation(toolUseId: String)
        /// The card read from the screen, in the composer slot. The log's call shows no buttons: its id is not the prompt.
        case composer(PermissionRequest)
        case none
    }

    /// `pending`: the log's pending call. `card`: `cardRequest(logged: pending, onScreen:)`.
    public static func cardPlace(pending: PermissionRequest?, card: PermissionRequest?) -> CardPlace {
        guard let card else { return .none }
        if let pending, pending.toolUseId == card.toolUseId { return .conversation(toolUseId: pending.toolUseId) }
        return .composer(card)
    }

    /// A notification's text.
    public struct Text: Sendable, Hashable {
        public var title: String, subtitle: String, body: String
    }

    /// The "needs you" notification. With the approval it answers: what is approved ("claude wants to run Bash", the
    /// command's first two lines), the workspace and tab in the subtitle. Without one: as before.
    public static func text(agent: String, workspace: String, tab: String, request: PermissionRequest?) -> Text {
        let who = agent.isEmpty ? tab : agent
        guard let request else { return Text(title: who, subtitle: "", body: "Needs you · \(workspace) · \(tab)") }
        let tool = request.toolName.isEmpty ? "a tool" : request.toolName
        // Escape sequences out first (colour codes), then any other control character.
        let clean: (String) -> String = { s in
            let plain = s.replacing(/\x{1b}\[[0-9;?]*[ -\/]*[@-~]/, with: "")
            return String(String.UnicodeScalarView(plain.unicodeScalars.filter { $0.value >= 0x20 && !(0x7f...0x9f).contains($0.value) }))
                .trimmingCharacters(in: .whitespaces)
        }
        let lines = request.command.components(separatedBy: .newlines).map(clean).filter { !$0.isEmpty }
        let cut: (String) -> String = { $0.count > 120 ? String($0.prefix(119)) + "…" : $0 }
        var body = lines.prefix(2).map(cut).joined(separator: "\n")
        if lines.count > 2 { body += " …" }
        if body.isEmpty { body = cut(clean(request.title)) }
        if body.isEmpty { body = "Needs you" }
        return Text(title: "\(who) wants to run \(tool)", subtitle: "\(workspace) · \(tab)", body: body)
    }

    public enum Outcome: Sendable, Hashable {
        /// Send the approval card's input: `HerdrClient.respond(to:allow:)`.
        case send(allow: Bool)
        /// Send nothing; open the tab.
        case open
    }

    /// `now` is read after the action arrives (nil: the pane is gone). Anything but the same prompt still on screen opens the tab.
    public static func outcome(actionIdentifier: String, boundPrompt: String?, now: PaneState?, writes: Bool) -> Outcome {
        guard let action = Action(rawValue: actionIdentifier), let boundPrompt, let now,
              promptIDs(now, writes: writes).contains(boundPrompt) else { return .open }
        return .send(allow: action.allows)
    }
}

/// On both backends (TUI `pane.read` / `pane.send_text`, hc `terminal.read` / `terminal.keys`).
extension PaneScreen {
    /// The approval card's input, sent only if `prompt` is still open: the pane is read again right before the keys.
    /// False: nothing sent. The notification buttons and the hub both answer through this.
    public func respondIfOpen(paneId: String, prompt: String, allow: Bool, agentKind: String, blocked: Bool, pending: PermissionRequest?, gate: KeyGate? = nil) async throws -> Bool {
        let action = (allow ? ApprovalNotice.Action.allow : .deny).rawValue
        // `gate` (a remote device): the same agent first, its status as it is now (not the one from the check), then
        // the screen read is the last call before the key.
        var blocked = blocked
        if let seen = gate?.agent {
            guard let now = await freshAgent(seen, paneId: paneId) else { return false }
            blocked = now.status == .blocked
        }
        guard let now = await approvalState(paneId: paneId, agentKind: agentKind, blocked: blocked, pending: pending),
              ApprovalNotice.outcome(actionIdentifier: action, boundPrompt: prompt, now: now, writes: true) == .send(allow: allow) else { return false }
        if let allowed = gate?.allowed { guard await allowed() else { return false } }
        if agentKind == "grok" {
            guard let onScreen = ApprovalNotice.promptIDs(now, writes: true).last else { return false }
            return try await respondGrok(paneId: paneId, screenPrompt: onScreen, allow: allow, gate: gate)
        }
        try await respond(to: paneId, allow: allow)
        return true
    }

    /// Grok's prompt `screenPrompt`: each key only while that prompt is on screen with the focus where the key needs it
    /// (`grokKey`), read right before the key. Deny: Down, wait for the focus on "No, reject", Enter. False: no answer
    /// was given (a Down may have moved the focus only).
    func respondGrok(paneId: String, screenPrompt: String, allow: Bool, gate: KeyGate?) async throws -> Bool {
        var moved = false
        for _ in 0..<20 {
            let screen = try await visibleText(paneId: paneId)
            guard let box = ApprovalNotice.grokBox(screen), ApprovalNotice.grokID(box) == screenPrompt else { return false }
            let key = ApprovalNotice.grokKey(screen, allow: allow)
            if key == "Down", moved { try await Task.sleep(for: .milliseconds(150)); continue }   // the move not drawn yet
            guard let key else { return false }
            if let allowed = gate?.allowed { guard await allowed() else { return false } }
            try await sendNamedKeys([key], to: paneId)
            if key == "Enter" { return true }
            moved = true
            try await Task.sleep(for: .milliseconds(150))
        }
        return false
    }

    /// Read-only: the pane's state for `ApprovalNotice`, with its visible screen. Nil when the screen cannot be read.
    public func approvalState(paneId: String, agentKind: String, blocked: Bool, pendingToolUseId: String?) async -> ApprovalNotice.PaneState? {
        guard let screen = try? await visibleText(paneId: paneId) else { return nil }
        return .init(agentKind: agentKind, blocked: blocked, pendingToolUseId: pendingToolUseId, screen: screen)
    }
    /// The same, with the log's pending call (its tool and command are matched with the prompt box).
    public func approvalState(paneId: String, agentKind: String, blocked: Bool, pending: PermissionRequest?) async -> ApprovalNotice.PaneState? {
        guard let screen = try? await visibleText(paneId: paneId) else { return nil }
        return .init(agentKind: agentKind, blocked: blocked, pendingToolUseId: pending?.toolUseId, screen: screen,
                     pendingTool: pending?.toolName, pendingCommand: pending?.command)
    }
}
