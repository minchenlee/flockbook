import Foundation
import HerdrKit

/// Finds the session log behind one herdr-core terminal, with a small file cache so most calls skip `ps` and `lsof`.
public enum FeedLocate {
    public struct Entry: Codable, Sendable, Equatable {
        public var kind: String
        public var cwd: String
        public var pids: [Int32]
        public var path: String?
        /// Seconds since 1970.
        public var checked: Double
        public init(kind: String, cwd: String, pids: [Int32], path: String?, checked: Double) {
            self.kind = kind; self.cwd = cwd; self.pids = pids; self.path = path; self.checked = checked
        }
    }
    public struct Cache: Codable, Sendable, Equatable {
        public var entries: [String: Entry] = [:]
        /// terminal → the rows the last `feed` printed (see `Feed`).
        public var feeds: [String: Feed.State] = [:]
        public init() {}
    }

    public struct Found: Sendable {
        public var pane: Pane
        public var path: String?
        /// The cache was used (no process scan).
        public var cached: Bool
    }

    /// A cached path is checked again after this long, as the app's resolver re-checks resolved panes.
    public static let recheck: TimeInterval = 30

    public enum Failure: Error, CustomStringConvertible {
        case noTerminal(String)
        case notAgent(String)
        public var description: String {
            switch self {
            case .noTerminal(let t): "no terminal \(t) in herdr-core"
            case .notAgent(let t): "terminal \(t) runs no agent"
            }
        }
    }

    /// The pane for a terminal address ("t7"), its pane id ("hc:t7") or its herdr-app address.
    public static func pane(_ ws: [Workspace], terminal: String) -> Pane? {
        let panes = ws.flatMap(\.tabs).flatMap(\.panes)
        return panes.first { $0.terminalId == terminal || $0.id == terminal || $0.id == "hc:\(terminal)" }
    }

    /// The cache entry still holds: same agent kind and processes, checked lately, and its log is still there.
    public static func fresh(_ e: Entry?, for pane: Pane, kind: String, now: Date) -> Bool {
        guard let e, e.kind == kind, e.pids == pane.processIds, now.timeIntervalSince1970 - e.checked < recheck else { return false }
        if let p = e.path { return FileManager.default.fileExists(atPath: p) }
        return false
    }

    public static func readCache(_ url: URL?) -> Cache {
        guard let url, let d = try? Data(contentsOf: url), let c = try? JSONDecoder().decode(Cache.self, from: d) else { return Cache() }
        return c
    }

    /// Written whole through a temp file, so a reader never sees half a file.
    public static func writeCache(_ c: Cache, to url: URL?) {
        guard let url, let d = try? JSONEncoder().encode(c) else { return }
        let dir = url.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let tmp = dir.appendingPathComponent(".\(url.lastPathComponent).\(getpid())")
        guard FileManager.default.createFile(atPath: tmp.path, contents: d, attributes: [.posixPermissions: 0o600]) else { return }
        if rename(tmp.path, url.path) != 0 { try? FileManager.default.removeItem(at: tmp) }
    }

    /// Finds the log for `terminal`. On a cache miss it resolves every local agent pane in one pass (one `ps`), so two
    /// panes never get the same log, and caches them all.
    public static func find(terminal: String, workspaces ws: [Workspace], resolver: SessionResolver, cacheURL: URL?,
                            now: Date = Date()) async throws -> Found {
        guard let pane = pane(ws, terminal: terminal) else { throw Failure.noTerminal(terminal) }
        guard let kind = pane.agent?.kind else { throw Failure.notAgent(terminal) }
        var cache = readCache(cacheURL)
        // Claude goes on in a new log and herdr keeps the old id: both paths follow `continued-in` (a cached path too,
        // or the chat flips between the two logs until the next recheck).
        func latest(_ p: String?, kind: String) -> String? {
            guard let p, kind == "claude" else { return p }
            return SessionLog.continued(URL(fileURLWithPath: p)).path
        }
        if let e = cache.entries[pane.terminalId], fresh(e, for: pane, kind: kind, now: now) {
            return Found(pane: pane, path: latest(e.path, kind: kind), cached: true)
        }
        let agents = ws.flatMap(\.tabs).flatMap(\.panes).filter { $0.agent != nil && !$0.terminalId.contains("/") }
        let needs = agents.map { SessionResolver.Need(paneId: $0.id, kind: $0.agent!.kind, cwd: $0.cwd, pids: $0.processIds) }
        let found = await resolver.resolve(needs, claimed: [])
        var next = Cache()
        next.feeds = cache.feeds
        // The resolver gives a full path, or for some agents only a session id: look that up in the agent's folder.
        func absolute(_ v: String?, kind: String) -> String? {
            guard let v, !v.isEmpty else { return nil }
            if v.hasPrefix("/") { return v }
            return SessionLog.locate(agent: kind, sessionId: v, dir: SessionLog.defaultDir(agent: kind))?.path
        }
        for p in agents {
            let k = p.agent!.kind
            next.entries[p.terminalId] = Entry(kind: k, cwd: p.cwd, pids: p.processIds, path: latest(absolute(found[p.id], kind: k), kind: k),
                                               checked: now.timeIntervalSince1970)
        }
        cache = next
        writeCache(cache, to: cacheURL)
        return Found(pane: pane, path: latest(absolute(found[pane.id], kind: kind), kind: kind), cached: false)
    }
}
