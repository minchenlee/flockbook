import Foundation
import Darwin

/// Asked when a local terminal's foreground program is not a shell and does not name the agent kind `status.app`
/// reports (the wrapper case, issue #15: `secretspec run -- claude`). Twin of herdr-core `hc::mapper::WrapperAsk`.
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
    /// A check scoped to one snapshot, so all wrapper terminals in it share one process-table read.
    func snapshot() -> HCAgentCheck?
    /// Drops the state of terminals no longer in the snapshot.
    func prune(live: Set<String>)
}

public extension HCAgentCheck {
    func snapshot() -> HCAgentCheck? { nil }
    func prune(live: Set<String>) {}
}

/// Never resolves a wrapper terminal.
public struct NoAgentCheck: HCAgentCheck {
    public init() {}
    public func agentUnder(_ ask: HCWrapperAsk) -> Int32? { nil }
}

/// Checks that a wrapper terminal's foreground tree runs the agent (W5: exactly one nearest agent process; W6: the status
/// changed at or after it started), with a small cache per terminal address. Twin of herdr-core `hc::wrapper::TreeCheck`.
///
/// A Yes holds 10 s and only while `kill(pid, 0)` says the agent lives; a No holds 30 s. Both are keyed on the foreground
/// pid, the terminal's creation, the kind and `status_ms`: a new status report reads the table again at once.
public final class HCTreeCheck: HCAgentCheck, @unchecked Sendable {
    static let yesTTL: TimeInterval = 10
    static let noTTL: TimeInterval = 30

    private struct Key: Equatable {
        var fgPid: Int32, created: Int, kind: String, statusMs: Int
        init(_ a: HCWrapperAsk) { fgPid = a.fgPid; created = a.created; kind = a.kind; statusMs = a.statusMs }
    }
    private enum Entry { case yes(Key, Int32, TimeInterval), no(Key, TimeInterval) }

    private let source: @Sendable () -> [SessionResolver.Proc]
    private let now: @Sendable () -> TimeInterval
    private let alive: @Sendable (Int32) -> Bool
    private let lock = NSLock()
    private var entries: [String: Entry] = [:]

    /// `source`: the process table (`ps`). `now`: a monotonic clock in seconds. `alive`: whether a pid still runs.
    public init(source: (@Sendable () -> [SessionResolver.Proc])? = nil,
                now: @escaping @Sendable () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
                alive: @escaping @Sendable (Int32) -> Bool = HCTreeCheck.pidAlive) {
        self.source = source ?? { SessionResolver.processes() }
        self.now = now; self.alive = alive
    }

    func lookup(_ ask: HCWrapperAsk, procs load: () -> [SessionResolver.Proc]) -> Int32? {
        let t = now()
        let key = Key(ask)
        let hit: Int32?? = lock.withLock {
            switch entries[ask.address] {
            case let .yes(k, pid, at) where k == key && t - at <= Self.yesTTL && alive(pid): return .some(pid)
            case let .no(k, at) where k == key && t - at <= Self.noTTL: return .some(nil)
            default: return nil
            }
        }
        if let hit { return hit }
        let procs = load()
        var found: Int32?
        if let agent = SessionResolver.agentInTree(root: ask.fgPid, kind: ask.kind, procs: procs, tieNewest: false) {
            let startMs = Int((agent.start.timeIntervalSince1970 * 1000).rounded(.down))
            // W6: the status changed at or after the agent started (`lstart` has 1 s steps).
            if ask.statusMs + 1000 >= startMs, agent.pid > 0 { found = agent.pid }
        }
        lock.withLock {
            if let found { entries[ask.address] = .yes(key, found, t) }
            else if !procs.isEmpty { entries[ask.address] = .no(key, t) }
            else { entries[ask.address] = nil }   // the read failed: no No entry either
        }
        return found
    }

    public func agentUnder(_ ask: HCWrapperAsk) -> Int32? { lookup(ask) { source() } }

    public func snapshot() -> HCAgentCheck? { Snapshot(tree: self) }

    public func prune(live: Set<String>) { lock.withLock { entries = entries.filter { live.contains($0.key) } } }

    /// One snapshot: the table is read at most once, on the first wrapper terminal that needs it.
    private final class Snapshot: HCAgentCheck, @unchecked Sendable {
        let tree: HCTreeCheck
        private let lock = NSLock()
        private var procs: [SessionResolver.Proc]?
        init(tree: HCTreeCheck) { self.tree = tree }
        func agentUnder(_ ask: HCWrapperAsk) -> Int32? {
            tree.lookup(ask) {
                lock.withLock {
                    if let procs { return procs }
                    let p = tree.source()
                    procs = p
                    return p
                }
            }
        }
    }

    /// `kill(pid, 0)`: the pid exists (EPERM: it exists, owned by another user).
    public static let pidAlive: @Sendable (Int32) -> Bool = { pid in
        guard pid > 0 else { return false }
        return kill(pid, 0) == 0 || errno == EPERM
    }
}
