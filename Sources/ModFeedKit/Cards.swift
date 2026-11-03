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
        /// An approval this sidecar already answered (`FeedAnswered`) that is still on screen: no buttons.
        public var answered: Bool?
    }

    /// As the hub gates a paired device's keys: before each key the pane must still show the agent read now (same kind
    /// and session, or for hc the same program), and the picker the expected question. No agent: fail closed.
    static func gate(_ pane: Pane) -> KeyGate { KeyGate(agent: pane.agent, allowed: { pane.agent != nil }) }
    static let agentChanged = Done(result: "changed", message: "another agent is in this pane now; nothing was sent")
    static let alreadyAnswered = Done(result: "answered", message: "this approval was already answered; waiting for the agent to move on")

    enum Current {
        /// `pending`: the log's pending call, passed to `respondIfOpen` (which matches it with the box again).
        case approval(id: String, title: String, command: String, tool: String, pending: PermissionRequest?)
        case question(QuestionRequest)
    }

    static func current(pane: Pane, screen: any PaneScreen, log: URL?) async -> Current? {
        guard let agent = pane.agent else { return nil }
        let items = log.flatMap { try? FeedTail.read($0) }.map { FeedTail.items($0.lines) } ?? []
        var perm: PermissionRequest?, question: QuestionRequest?
        for it in items.reversed() {
            if perm == nil, case .permissionRequest(let p) = it.kind, p.state == .pending { perm = p }
            if question == nil, case .question(let q) = it.kind, q.state == .pending { question = q }
        }
        // The app's rules (`ApprovalNotice`): the card is the prompt box on screen; the log's call only when it is
        // that box (same tool and command). A log that lags the screen still names the call before, and its id must
        // never answer the next prompt. The box's id covers its command (Claude, Grok and agy boxes alike).
        if PermissionGate.checkApproval(agent, pendingTool: perm != nil), let text = try? await screen.visibleText(paneId: pane.id) {
            let state = ApprovalNotice.PaneState(agentKind: agent.kind, blocked: agent.status == .blocked, pendingToolUseId: perm?.toolUseId,
                                                 screen: text, pendingTool: perm?.toolName, pendingCommand: perm?.command)
            if ApprovalNotice.prompt(state, writes: true) != nil,
               let card = ApprovalNotice.cardRequest(logged: perm, onScreen: ApprovalNotice.screenRequest(text, agent: agent.kind)) {
                return .approval(id: card.toolUseId, title: card.title.isEmpty ? "Approval needed" : card.title, command: card.command,
                                 tool: card.toolName, pending: perm)
            }
        }
        // A question in the log: answered by a message (Codex async) any time, or by keys while the agent waits.
        if let q = question, q.delivery == .message || agent.status == .blocked { return .question(q) }
        if PermissionGate.readQuestion(agent), let q = try? await screen.pendingQuestion(paneId: pane.id) { return .question(q) }
        return nil
    }

    /// `all`: for a question with more behind it, walk the picker's tabs first (keys, on the owner's click) so the
    /// card holds every question and `unread` is 0.
    public static func card(pane: Pane, screen: any PaneScreen, log: URL?, all: Bool = false, cache: URL? = nil) async -> Card {
        var now = await current(pane: pane, screen: screen, log: log)
        if all, case .question(let q)? = now, q.unread > 0,
           let full = try? await screen.loadAllQuestions(paneId: pane.id, gate: gate(pane), expect: q) { now = .question(full) }
        // The claims of prompts no longer on screen go (`FeedAnswered.seen`); one still there shows as answered.
        let answered = FeedAnswered(cache: cache)
        if case .approval(let id, _, _, _, _)? = now { answered.seen(terminal: pane.id, current: id) } else { answered.seen(terminal: pane.id, current: nil) }
        switch now {
        case .approval(let id, let title, let command, let tool, _)?:
            return Card(kind: "approval", request: id, title: title, command: FeedRows.cut(command, FeedRows.textCap), tool: tool,
                        answered: answered.isClaimed(terminal: pane.id, id: id) ? true : nil)
        case .question(let q)?:
            let row = FeedRows.row(ConversationItem(id: q.toolUseId, kind: .question(q)))
            return Card(kind: "question", request: q.toolUseId, question: row.question, unread: q.unread)
        case nil:
            return Card()
        }
    }

    public struct Done: Encodable, Sendable, Equatable {
        public var type = "done"
        /// "sent"; "unconfirmed" (no answer in time: the text or key may still arrive); "changed" (another card, or
        /// none, is on screen now: nothing sent); "answered" (approve: this prompt was answered already, nothing
        /// sent); "failed".
        public var result: String
        public var message: String?
    }

    public static func approve(pane: Pane, screen: any PaneScreen, log: URL?, request: String, allow: Bool, cache: URL? = nil) async -> Done {
        guard let agent = pane.agent,
              case .approval(let id, _, _, _, let pending)? = await current(pane: pane, screen: screen, log: log), id == request else {
            return Done(result: "changed", message: "the approval on screen is not the one shown")
        }
        guard await screen.passes(gate(pane), paneId: pane.id) else { return agentChanged }
        // One answer per prompt, across runs and views: claimed before the key; kept when the key went out or may
        // still land (hc held it); given back when nothing went out.
        let answered = FeedAnswered(cache: cache)
        guard answered.claim(terminal: pane.id, id: id) else { return alreadyAnswered }
        // As the app's buttons: the screen is read again right before the key, and the key goes only while the box on
        // screen is still this id (Grok: its own keys, each checked against the focus).
        do {
            let sent = try await screen.respondIfOpen(paneId: pane.id, prompt: id, allow: allow, agentKind: agent.kind,
                                                      blocked: agent.status == .blocked, pending: pending, gate: gate(pane))
            if sent { return Done(result: "sent") }
            answered.release(terminal: pane.id, id: id)
            return Done(result: "changed", message: "the prompt on screen is not the one shown")
        } catch HerdrError.timeout {
            // hc may still hold the key for its typing-quiet rule: not "nothing sent". The claim stays.
            return Done(result: "unconfirmed", message: "no answer in time; the key may still arrive")
        } catch {
            answered.release(terminal: pane.id, id: id)
            return Done(result: "failed", message: "herdr-core did not take the keys")
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
