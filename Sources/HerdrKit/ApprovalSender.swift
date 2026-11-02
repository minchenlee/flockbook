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
}
