import Foundation
import Observation

/// One answer per permission prompt. A card press claims its pane (`begin`) right away, on the main actor, so a second
/// press (Allow twice, or Allow then Deny) is ignored until the caller has re-checked the screen and calls `end`.
/// Without it a second "1" could approve the next tool call the user never saw, or an Esc could land in the running turn.
@Observable @MainActor public final class ApprovalSender {
    public private(set) var inFlight: Set<String> = []
    public init() {}

    /// False while an answer for this pane is on its way.
    public func begin(_ paneId: String) -> Bool { inFlight.insert(paneId).inserted }
    public func end(_ paneId: String) { inFlight.remove(paneId) }
    public func isAnswering(_ paneId: String) -> Bool { inFlight.contains(paneId) }

    /// The one way an approval goes out, for the card, the notification buttons and the hub alike: claims the pane,
    /// then `respondIfOpen` (the prompt re-read right before the keys). After keys went out the pane stays claimed
    /// until `prompt` has left the screen (read every `poll`, at most `maxWait`), so a redraw lag never lets a second
    /// "1" approve the next call. On hc a `terminal.keys` past its timeout may still land (the typing-quiet rule holds
    /// keys): `unconfirmed`, claimed the same way but for up to `heldWait` (about hc's send timeout), since the key
    /// may still be held after the short `maxWait` meant for a redraw.
    public func answer(paneId: String, via screen: any PaneScreen, prompt: String, allow: Bool, agentKind: String, blocked: Bool,
                       pending: PermissionRequest?, gate: KeyGate? = nil, poll: Duration = .seconds(1), maxWait: Duration = .seconds(10),
                       heldWait: Duration = .seconds(60)) async -> ApprovalAnswer {
        guard begin(paneId) else { return .busy }
        let result: ApprovalAnswer
        do {
            let sent = try await screen.respondIfOpen(paneId: paneId, prompt: prompt, allow: allow, agentKind: agentKind, blocked: blocked,
                                                      pending: pending, gate: gate)
            result = sent ? .sent : .notSent
        } catch HerdrError.timeout where screen.backend == .hc {
            result = .unconfirmed
        } catch {
            result = .failed
        }
        guard result == .sent || result == .unconfirmed else { end(paneId); return result }
        Task { [weak self] in
            await Self.waitGone(paneId: paneId, via: screen, prompt: prompt, agentKind: agentKind, pending: pending, poll: poll,
                                maxWait: result == .unconfirmed ? heldWait : maxWait)
            self?.end(paneId)
        }
        return result
    }

    /// Returns once the pane's screen no longer shows `prompt` (or after `maxWait`). Read-only.
    static func waitGone(paneId: String, via screen: any PaneScreen, prompt: String, agentKind: String, pending: PermissionRequest?,
                         poll: Duration, maxWait: Duration) async {
        let until = ContinuousClock.now + maxWait
        while ContinuousClock.now < until {
            try? await Task.sleep(for: poll)
            guard let now = await screen.approvalState(paneId: paneId, agentKind: agentKind, blocked: true, pending: pending) else { continue }
            if !ApprovalNotice.promptIDs(now, writes: true).contains(prompt) { return }
        }
    }
}

/// What became of an approval (`ApprovalSender.answer`).
public enum ApprovalAnswer: Sendable, Equatable {
    /// The keys went out while the prompt was on screen.
    case sent
    /// The prompt was no longer the one the card showed: nothing sent.
    case notSent
    /// Another answer for this pane is on its way: nothing sent.
    case busy
    /// hc: the keys went out but no answer came in time; they may still land.
    case unconfirmed
    /// The keys could not be sent.
    case failed
}
