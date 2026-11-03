import Foundation

/// Which Herdr runtime an agent lives on.
public enum BackendKind: String, Sendable, Hashable, Codable {
    /// The `herdr` 0.9.x TUI server (`HerdrClient`): workspaces, tabs, panes, layout actions.
    case tui
    /// herdr-core (`hc`, `HCClient`): terminals grouped by the official app's labels and its `herdr-app` document.
    case hc
}

extension Pane {
    /// The host a composer uploads this pane's attachments to (and runs its hook setup and git on): the herdr
    /// session's remote host for a TUI pane; none for an hc pane, whose files go beside its terminal (`terminal.put`).
    public func composerRemote(_ sessionRemote: RemoteHost?) -> RemoteHost? { backend == .tui ? sessionRemote : nil }
}

/// What became of a message the composer handed to a backend.
public enum DeliveryOutcome: Sendable, Equatable {
    /// The backend took it.
    case sent
    /// The request went out but no answer came in time: the backend may still deliver it, so the text must not come
    /// back into the composer (a retry would send it twice).
    case unconfirmed
    /// It never reached the backend (not connected, pane gone, refused): give the text back.
    case failed
}

/// What the app layer needs from a backend. `HerdrClient` (TUI) and `HCClient` (herdr-core) implement it.
/// Pane ids are the backend's own (`w1:p1` on TUI, `hc:t7` on hc), so callers route by `Agent.backend`/`Pane.backend`.
public protocol HerdrBackend: Actor {
    nonisolated var backend: BackendKind { get }
    /// Whether the layout writes of `LayoutBackend` work (both backends now).
    nonisolated var supportsLayout: Bool { get }
    func snapshot() async throws -> [Workspace]
    /// Yields once per change worth a new snapshot. Finishes when the connection drops.
    nonisolated func events() -> AsyncThrowingStream<Void, Error>
    /// Types `text` and presses Enter.
    func send(text: String, to paneId: String) async throws
    /// Raw bytes (control keys, escape sequences).
    func sendKeys(_ bytes: [UInt8], to paneId: String) async throws
    /// Named keys in herdr's spelling ("Enter", "Down", "Escape", "C-c").
    func sendNamedKeys(_ keys: [String], to paneId: String) async throws
}

/// Screen reads and keys for the approval and question cards (`checkPrompt`, `pendingQuestion`, `loadAllQuestions`,
/// `answerQuestion` are written once on top of these). TUI: `pane.read` / `pane.send_text`; hc: `terminal.read` /
/// `terminal.keys`. Reads are safe any time; everything that sends keys runs only on a user action.
public protocol PaneScreen: HerdrBackend {
    /// The pane's visible screen as plain text. Read-only.
    func visibleText(paneId: String) async throws -> String
    /// Answers a permission prompt: allow once, or deny. Call only when the prompt is on screen.
    func respond(to paneId: String, allow: Bool) async throws
}

extension HerdrClient: PaneScreen {}
extension HCClient: PaneScreen {}

extension HerdrClient: HerdrBackend {
    nonisolated public var backend: BackendKind { .tui }
    nonisolated public var supportsLayout: Bool { true }
}
