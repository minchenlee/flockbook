import Foundation

/// Asked right before every key of an input that a remote device started: the pane must still show the agent the
/// checks saw, and `allowed` (the device's level and Allow input now) must still say yes. A refusal stops the
/// input there; nothing more is sent. herdr has no compare-and-send, so the moment between the last read and the
/// key stays; `PaneScreen.passes` keeps it to one screen read and a main-actor hop.
public struct KeyGate: Sendable {
    public var agent: Agent?
    public var allowed: (@MainActor @Sendable () -> Bool)?
    public init(agent: Agent?, allowed: (@MainActor @Sendable () -> Bool)?) { self.agent = agent; self.allowed = allowed }
}

/// A `KeyGate` said no: the input stopped, the keys after it were not sent.
public struct KeyGateStop: Error, Sendable, Hashable, CustomStringConvertible {
    public init() {}
    public var description: String { "stopped: the pane, its agent or this device's input level changed" }
}

extension PaneScreen {
    /// True when the next key may go: the pane still shows the gate's agent (same kind and session; fresh snapshot)
    /// and `agentOK` holds for it as it is now, then `ok` holds on a fresh screen read, then `allowed()`. The screen
    /// is read last, right before the answer. No gate: true, nothing read.
    public func passes(_ gate: KeyGate?, paneId: String, screen ok: (@Sendable (String) -> Bool)? = nil,
                       agent agentOK: (@Sendable (Agent) -> Bool)? = nil) async -> Bool {
        guard let gate else { return true }
        if let seen = gate.agent {
            guard let now = await freshAgent(seen, paneId: paneId), agentOK?(now) ?? true else { return false }
        }
        if let ok { guard let s = try? await visibleText(paneId: paneId), ok(s) else { return false } }
        if let allowed = gate.allowed { guard await allowed() else { return false } }
        return true
    }

    /// Throws `KeyGateStop` when `passes` says no.
    func gated(_ gate: KeyGate?, paneId: String, screen ok: (@Sendable (String) -> Bool)? = nil) async throws {
        guard await passes(gate, paneId: paneId, screen: ok) else { throw KeyGateStop() }
    }

    /// Named keys for a paired device: one gated call per key, so no committing key (Enter, Tab) rides in a batch
    /// past a check. No gate: one call, as before.
    func gatedKeys(_ keys: [String], gate: KeyGate?, paneId: String, screen ok: (@Sendable (String) -> Bool)?) async throws {
        guard gate != nil else { try await sendNamedKeys(keys, to: paneId); return }
        for k in keys {
            try await gated(gate, paneId: paneId, screen: ok)
            try await sendNamedKeys([k], to: paneId)
        }
    }
}

extension HerdrBackend {
    /// The pane's agent now (fresh snapshot), when it is `seen`: same kind and the same session id. Without a session
    /// id there is nothing stable to bind to (herdr gives no other agent identity), so nil: paired keys fail closed.
    /// hc: the same kind and the same program in the terminal (`Agent.process`); none reported: nil.
    func freshAgent(_ seen: Agent, paneId: String) async -> Agent? {
        // hc: no session id from hc (the store finds it in the log); the same program in the terminal instead.
        if seen.backend == .hc {
            guard let process = seen.process, let ws = try? await snapshot(),
                  let a = ws.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == paneId })?.agent,
                  a.backend == .hc, a.kind == seen.kind, a.process == process else { return nil }
            return a
        }
        guard let sid = seen.sessionId, !sid.isEmpty,
              let ws = try? await snapshot(), let a = ws.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == paneId })?.agent,
              a.kind == seen.kind, a.sessionId == sid else { return nil }
        return a
    }
}
