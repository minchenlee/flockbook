import Foundation
import HerdrKit

/// `card`, `approve`, `answer`, `send`: the approval and question cards and the composer, on herdr-core.
///
/// Reading is safe any time. Keys go out only on the owner's action (`approve`, `answer`), and only after a fresh read
/// shows the same request the mod showed: the mod passes the card's `request` id back, and a different card on screen
/// is refused with `changed`, nothing sent.
public enum FeedCards {
    public struct Card: Encodable, Sendable, Equatable {
        public var type = "card"
        /// "approval", "question", or nil (no card).
        public var kind: String?
        /// The id to pass back to `approve` / `answer`.
        public var request: String?
        public var title: String?
        public var command: String?
        public var tool: String?
        public var question: FeedQuestion?
        /// Questions known only by their header (Claude shows one at a time). `answer` reads them first.
        public var unread: Int?
    }

    /// As the hub gates a paired device's keys: before each key the pane must still show the agent read now (same kind
    /// and session, or for hc the same program), and the picker the expected question. No agent: fail closed.
    static func gate(_ pane: Pane) -> KeyGate { KeyGate(agent: pane.agent, allowed: { pane.agent != nil }) }
    static let agentChanged = Done(result: "changed", message: "another agent is in this pane now; nothing was sent")

    enum Current {
        case approval(id: String, title: String, command: String, tool: String)
        case question(QuestionRequest)
    }

    /// The prompt's own lines: the "Do you want …?" line and the 4 lines after it. The id of an approval the log does
    /// not name is their hash, so a spinner elsewhere on screen does not change it.
    static func promptLines(_ screen: String) -> [String] {
        let lines = screen.components(separatedBy: "\n").map { $0.trimmingCharacters(in: CharacterSet(charactersIn: "│|┃ \t")) }
        guard let i = lines.lastIndex(where: { $0.hasPrefix(PermissionGate.promptMarker) }) else { return [] }
        return Array(lines[i...].filter { !$0.isEmpty }.prefix(5))
    }

    /// The lines above the prompt (what it asks about), for an approval the log does not name.
    static func promptContext(_ screen: String) -> String {
        let lines = screen.components(separatedBy: "\n").map { $0.trimmingCharacters(in: CharacterSet(charactersIn: "│|┃ \t")) }
        guard let i = lines.lastIndex(where: { $0.hasPrefix(PermissionGate.promptMarker) }) else { return "" }
        return lines[max(0, i - 6)..<i].filter { !$0.isEmpty && !$0.allSatisfy { "─━═╭╮╰╯".contains($0) } }.joined(separator: "\n")
    }

    static func current(pane: Pane, screen: any PaneScreen, log: URL?) async -> Current? {
        guard let agent = pane.agent else { return nil }
        let items = log.flatMap { try? FeedTail.read($0) }.map { FeedTail.items($0.lines) } ?? []
        var perm: PermissionRequest?, question: QuestionRequest?
        for it in items.reversed() {
            if perm == nil, case .permissionRequest(let p) = it.kind, p.state == .pending { perm = p }
            if question == nil, case .question(let q) = it.kind, q.state == .pending { question = q }
        }
        if PermissionGate.checkApproval(agent, pendingTool: perm != nil), let text = try? await screen.visibleText(paneId: pane.id),
           PermissionGate.screenShowsPrompt(text) {
            let id = perm.map(\.toolUseId).flatMap { $0.isEmpty ? nil : $0 } ?? "screen-" + fnv1a(promptLines(text).joined(separator: "\n"))
            let ctx = promptContext(text)
            return .approval(id: id, title: perm?.title ?? "Approval needed", command: perm?.command ?? ctx, tool: perm?.toolName ?? "")
        }
        // A question in the log: answered by a message (Codex async) any time, or by keys while the agent waits.
        if let q = question, q.delivery == .message || agent.status == .blocked { return .question(q) }
        if PermissionGate.readQuestion(agent), let q = try? await screen.pendingQuestion(paneId: pane.id) { return .question(q) }
        return nil
    }

    /// `all`: for a question with more behind it, walk the picker's tabs first (keys, on the owner's click) so the
    /// card holds every question and `unread` is 0.
    public static func card(pane: Pane, screen: any PaneScreen, log: URL?, all: Bool = false) async -> Card {
        var now = await current(pane: pane, screen: screen, log: log)
        if all, case .question(let q)? = now, q.unread > 0,
           let full = try? await screen.loadAllQuestions(paneId: pane.id, gate: gate(pane), expect: q) { now = .question(full) }
        switch now {
        case .approval(let id, let title, let command, let tool)?:
            return Card(kind: "approval", request: id, title: title, command: FeedRows.cut(command, FeedRows.textCap), tool: tool)
        case .question(let q)?:
            let row = FeedRows.row(ConversationItem(id: q.toolUseId, kind: .question(q)))
            return Card(kind: "question", request: q.toolUseId, question: row.question, unread: q.unread)
        case nil:
            return Card()
        }
    }

    public struct Done: Encodable, Sendable, Equatable {
        public var type = "done"
        /// "sent"; "unconfirmed" (send: no answer in time, the text may still arrive); "changed" (another card, or
        /// none, is on screen now: nothing sent); "failed".
        public var result: String
        public var message: String?
    }

    public static func approve(pane: Pane, screen: any PaneScreen, log: URL?, request: String, allow: Bool) async -> Done {
        guard case .approval(let id, _, _, _)? = await current(pane: pane, screen: screen, log: log), id == request else {
            return Done(result: "changed", message: "the approval on screen is not the one shown")
        }
        // The agent first, then answerPrompt reads the screen last, right before the key.
        guard await screen.passes(gate(pane), paneId: pane.id) else { return agentChanged }
        switch await screen.answerPrompt(paneId: pane.id, allow: allow) {
        case .sent: return Done(result: "sent")
        case .promptGone: return Done(result: "changed", message: "the prompt went away")
        case .failed: return Done(result: "failed", message: "herdr-core did not take the keys")
        }
    }

    /// `[{"picks":[0]}, {"picks":[], "other":"text"}]`: one per question, option indexes from 0.
    public static func parseAnswers(_ json: String) -> [QuestionAnswer]? {
        guard let a = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [[String: Any]] else { return nil }
        var out: [QuestionAnswer] = []
        for o in a {
            guard let picks = (o["picks"] ?? [Int]()) as? [Int] else { return nil }
            let other = o["other"] as? String
            out.append(QuestionAnswer(picks: picks, other: other))
        }
        return out
    }

    public static func answer(pane: Pane, screen: any PaneScreen, log: URL?, request: String, answers: [QuestionAnswer]) async -> Done {
        guard case .question(var q)? = await current(pane: pane, screen: screen, log: log), q.toolUseId == request else {
            return Done(result: "changed", message: "the question on screen is not the one shown")
        }
        do {
            if q.unread > 0 {
                guard let all = try await screen.loadAllQuestions(paneId: pane.id, gate: gate(pane), expect: q) else {
                    return Done(result: "changed", message: "the question went away")
                }
                q = all
            }
            guard answers.count == q.questions.count else {
                return Done(result: "failed", message: "\(answers.count) answers for \(q.questions.count) questions")
            }
            try await screen.answerQuestion(paneId: pane.id, request: q, answers: answers, gate: gate(pane))
            return Done(result: "sent")
        } catch is KeyGateStop {
            return agentChanged
        } catch QuestionAdapter.Failure.notOnScreen {
            return Done(result: "changed", message: "the question is not on screen")
        } catch {
            return Done(result: "failed", message: "\(error)")
        }
    }

    /// The composer: the text, with each file in `files` named in it (`HCClient.deliver`).
    public static func send(pane: Pane, client: HCClient, text: String, files: [String]) async -> Done {
        guard await client.passes(gate(pane), paneId: pane.id) else { return agentChanged }
        let r = files.isEmpty ? await client.deliver(text: text, to: pane.id) : await client.deliver(text: text, to: pane.id, files: files)
        switch r {
        case .sent: return Done(result: "sent")
        case .unconfirmed: return Done(result: "unconfirmed", message: "no answer in time; the text may still arrive")
        default: return Done(result: "failed", message: "herdr-core did not take the text")
        }
    }
}
