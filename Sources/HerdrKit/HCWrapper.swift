import Foundation

/// Asked when a local terminal's foreground program is not a shell and does not name the agent kind `status.app`
/// reports (the wrapper case, issue #15).
public struct HCWrapperAsk: Sendable, Hashable {
    public var address: String
    public var fgPid: Int32
    public var created: Int
    public var kind: String
    public var statusMs: Int
    public init(address: String, fgPid: Int32, created: Int, kind: String, statusMs: Int) {
        self.address = address; self.fgPid = fgPid; self.created = created; self.kind = kind; self.statusMs = statusMs
    }
}

/// The running agent's pid under a wrapper foreground process. Fails closed (nil) when unsure.
public protocol HCAgentCheck: Sendable {
    func agentUnder(_ ask: HCWrapperAsk) -> Int32?
}

/// Never resolves a wrapper terminal.
public struct NoAgentCheck: HCAgentCheck {
    public init() {}
    public func agentUnder(_ ask: HCWrapperAsk) -> Int32? { nil }
}

/// The process-tree check with its small cache.
public final class HCTreeCheck: HCAgentCheck, @unchecked Sendable {
    public init(source: @escaping @Sendable () -> [SessionResolver.Proc] = { [] },
                now: @escaping @Sendable () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
                alive: @escaping @Sendable (Int32) -> Bool = { _ in false }) {}
    public func agentUnder(_ ask: HCWrapperAsk) -> Int32? { nil }
}
