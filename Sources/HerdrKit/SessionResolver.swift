import Foundation

/// A Claude pane whose herdr session is not shown: `teammate` is herdr's session, `lead` the log shown instead.
public struct LeadSwap: Sendable, Hashable {
    public var teammate: String
    public var lead: String?
    public init(teammate: String, lead: String?) { self.teammate = teammate; self.lead = lead }
}

/// Finds the log of an agent pane that herdr reports without `agent_session` (seen with Codex on herdr 0.9.1).
/// Read-only: one `ps`, one `lsof`, and the first line of candidate logs. It never sends anything to a pane.
///
/// 1. The pane's process: the env holds `HERDR_PANE_ID` and `HERDR_SOCKET_PATH` (the session), so panes of
///    other herdr sessions never match. The agent is the newest child of the pane's shell.
/// 2. A log file that the agent's process tree holds open (`lsof`). Exactly one, or none.
/// 3. Grok Build only: the session `active_sessions.json` lists for the agent's pid.
/// 4. Codex only: the rollout whose `session_meta` cwd is the pane's cwd and that started at or after the agent.
///    Exactly one unclaimed match, or none. It never guesses between two.
///
/// The result is cached per pane and kept while the agent pid stays the same.
///
/// For a session on a `RemoteHost` it is off: `ps` and `lsof` here cannot see the host's processes, and the relay
/// socket path matches no pane. Panes show herdr's `agent_session` as it is: `resolve` finds nothing, and `leads`
/// judges every named Claude session a lead's log (teammate logs are not held back remotely).
public actor SessionResolver {
    public struct Proc: Sendable, Hashable {
        public var pid: Int32
        public var ppid: Int32
        public var start: Date
        /// Command line followed by the environment, as `ps -E` prints it.
        public var line: String
    }

    public struct Need: Sendable, Hashable {
        public var paneId: String
        public var kind: String
        public var cwd: String
        public init(paneId: String, kind: String, cwd: String) { self.paneId = paneId; self.kind = kind; self.cwd = cwd }
    }

    private struct Entry { var pid: Int32?; var path: String?; var checked: Date }
    private var cache: [String: Entry] = [:]
    private let socketPath: String
    private let codexDir: String
    private let projectsDir: String
    /// The most bytes read of one log to judge it.
    private let verdictCap: Int
    /// How many times a log was read to judge it.
    public private(set) var verdictReads = 0
    /// Unresolved panes are tried again after this; resolved ones are re-checked (pid still the same) after `recheck`.
    private let retry: TimeInterval
    private let recheck: TimeInterval
    /// The session runs on another machine: the resolver is off (see above).
    private let remote: Bool

    public init(socketPath: String, codexDir: String = SessionLog.defaultDir(agent: "codex"),
                projectsDir: String = SessionLog.defaultDir(agent: "claude"),
                retry: TimeInterval = 5, recheck: TimeInterval = 30, verdictCap: Int = 8 << 20, remote: RemoteHost? = nil) {
        self.socketPath = socketPath; self.codexDir = codexDir; self.projectsDir = projectsDir; self.verdictCap = verdictCap
        self.retry = retry; self.recheck = recheck; self.remote = remote != nil
    }

    // MARK: - Teammates

    /// A Claude pane and the session herdr names for it.
    public struct LeadAsk: Sendable, Hashable {
        public var paneId: String
        public var sessionId: String
        public init(paneId: String, sessionId: String) { self.paneId = paneId; self.sessionId = sessionId }
    }

    /// One pass over the Claude panes: the panes whose log is not shown (paneId → swap), and the sessions judged to
    /// be a lead's log, which the panes show as herdr names them.
    public struct LeadPass: Sendable, Hashable {
        public var swaps: [String: LeadSwap] = [:]
        public var leads: Set<String> = []
        public init(swaps: [String: LeadSwap] = [:], leads: Set<String> = []) { self.swaps = swaps; self.leads = leads }
    }

    /// `mate` nil: undecided. `final`: decided, or undecided after `verdictCap` bytes; read again only if it shrinks.
    private struct Verdict { var file: Int; var size: UInt64; var mate: Bool?; var final: Bool }
    /// Standardized path → whether the log is a teammate's, while it is the same file and has not shrunk since last
    /// seen. An undecided log that is not final is read again once its size changes.
    private var verdicts: [String: Verdict] = [:]
    /// paneId → the standardized path of the last lead log herdr gave for the pane.
    private var lastLead: [String: String] = [:]

    /// Judges the log herdr names for each Claude pane. A lead's log is shown as it is and becomes the pane's last
    /// lead. A teammate's log, an undecided one or one not found is never shown: the pane gets its last lead (judged
    /// again, and unless another pane shows that log), else no log.
    public func leads(_ asks: [LeadAsk], claimed: Set<String>) -> LeadPass {
        if remote { return LeadPass(leads: Set(asks.map(\.sessionId))) }
        let ids = Set(asks.map(\.paneId))
        lastLead = lastLead.filter { ids.contains($0.key) }
        var pass = LeadPass()
        var asked: Set<String> = []
        for a in asks {
            let p = path(a.sessionId)
            if let p { asked.insert(p) }
            if let p, judge(p) == false {
                lastLead[a.paneId] = p
                pass.leads.insert(a.sessionId)
            }
        }
        asked.formUnion(lastLead.values)
        verdicts = verdicts.filter { asked.contains($0.key) }
        located = located.filter { id in asks.contains { $0.sessionId == id.key } }
        let shown = Set(claimed.map { $0.hasPrefix("/") ? ($0 as NSString).standardizingPath : $0 })
        for a in asks where !pass.leads.contains(a.sessionId) {
            let lead = lastLead[a.paneId].flatMap { l in
                judge(l) != false || Self.isClaimed(l, shown) || lastLead.contains { $0.key != a.paneId && $0.value == l } ? nil : l
            }
            pass.swaps[a.paneId] = LeadSwap(teammate: a.sessionId, lead: lead)
        }
        return pass
    }

    /// Whether the log at `path` is a teammate's, nil while undecided. A decided verdict is kept for the same file
    /// (file number) while it does not shrink.
    private func judge(_ path: String) -> Bool? {
        let a = try? FileManager.default.attributesOfItem(atPath: path)
        guard let file = a?[.systemFileNumber] as? Int, let size = a?[.size] as? UInt64 else { return nil }
        if var v = verdicts[path], v.file == file, size >= v.size, v.final || size == v.size {
            v.size = size
            verdicts[path] = v
            return v.mate
        }
        verdictReads += 1
        let r = Self.teammateVerdict(path: path, cap: verdictCap)
        verdicts[path] = Verdict(file: file, size: size, mate: r.mate, final: r.mate != nil || r.capped)
        return r.mate
    }

    /// Session id → its log's standardized path, kept while the file is there.
    private var located: [String: String] = [:]

    private func path(_ idOrPath: String) -> String? {
        if idOrPath.hasPrefix("/") { return (idOrPath as NSString).standardizingPath }
        if let p = located[idOrPath], FileManager.default.fileExists(atPath: p) {
            let next = SessionLog.continued(URL(fileURLWithPath: p), projectsDir: projectsDir).path   // went on since
            located[idOrPath] = next
            return next
        }
        let p = SessionLog.locate(agent: "claude", sessionId: idOrPath, dir: projectsDir).map { ($0.path as NSString).standardizingPath }
        located[idOrPath] = p
        return p
    }

    /// Whether a Claude log is a teammate's: its first message line (`user` or `assistant`) carries `teamName` and
    /// `agentName`. Nil while no message line decides, such as a first line not yet whole.
    public static func teammateVerdict(_ data: Data) -> Bool? {
        for line in data.split(separator: 0x0A) {
            switch lineVerdict(line) {
            case .skip: continue
            case .decided(let mate): return mate
            case .undecided: return nil
            }
        }
        return nil
    }

    /// `teammateVerdict` of the log at `path`, read line by line: at most 64 KB of each line and `cap` bytes in all.
    /// `capped`: the read stopped at `cap`.
    static func teammateVerdict(path: String, cap: Int) -> (mate: Bool?, capped: Bool) {
        guard let fh = FileHandle(forReadingAtPath: path) else { return (nil, false) }
        defer { try? fh.close() }
        let keep = 64 << 10
        var line = Data(), cut = false, read = 0
        while read < cap, let d = try? fh.read(upToCount: min(64 << 10, cap - read)), !d.isEmpty {
            read += d.count
            var rest = d[...]
            while let nl = rest.firstIndex(of: 0x0A) {
                if !cut { line.append(rest[..<nl].prefix(keep - line.count)) }
                switch lineVerdict(line) {
                case .skip: break
                case .decided(let mate): return (mate, false)
                case .undecided: return (nil, false)
                }
                line = Data(); cut = false
                rest = rest[rest.index(after: nl)...]
            }
            if !cut { line.append(rest.prefix(keep - line.count)); cut = line.count >= keep }
        }
        return (line.isEmpty ? nil : teammateVerdict(line), read >= cap)
    }

    private enum LineVerdict { case skip, decided(Bool), undecided }

    /// One log line. A line cut off (by the 64 KB limit, or not yet whole) is judged by its keys: one that begins
    /// with a `type` other than `user` or `assistant` is skipped; else the keys before `"message":`, where Claude Code
    /// writes `teamName`, `agentName` and then `type`, decide: `type` without `teamName` is not a teammate's.
    private static func lineVerdict(_ line: Data) -> LineVerdict {
        if let o = try? JSONSerialization.jsonObject(with: line) as? [String: Any] {
            guard let type = o["type"] as? String, type == "user" || type == "assistant" else { return .skip }
            func named(_ k: String) -> Bool { !((o[k] as? String) ?? "").isEmpty }
            return .decided(named("teamName") && named("agentName"))
        }
        let s = String(decoding: line, as: UTF8.self)
        if s.hasPrefix(#"{"type":""#), !s.hasPrefix(#"{"type":"user""#), !s.hasPrefix(#"{"type":"assistant""#) { return .skip }
        let keys = s.range(of: #""message":"#).map { s[..<$0.lowerBound] } ?? s[...]
        guard keys.contains(#""type":"user""#) || keys.contains(#""type":"assistant""#) else { return .undecided }
        func named(_ k: String) -> Bool { keys.range(of: "\"\(k)\":\"[^\"]", options: .regularExpression) != nil }
        if !keys.contains(#""teamName":"#) { return .decided(false) }
        return named("teamName") && named("agentName") ? .decided(true) : .undecided
    }


    /// Agent kinds whose session id herdr reports through its own integration (`herdr integration install <kind>`).
    public static let herdrIntegrations: Set<String> = ["claude", "codex", "grok"]

    /// On a host nothing guesses a session from its files (`resolve` returns none), so an agent herdr reports with no
    /// session id has no conversation until herdr's integration is installed there. The command to run on the host, or nil.
    public static func hostSetup(kind: String, sessionId: String?, remote: RemoteHost?) -> String? {
        guard remote != nil, sessionId == nil, herdrIntegrations.contains(kind) else { return nil }
        return "herdr integration install \(kind)"
    }

    /// Log paths for the given panes (paneId → absolute `.jsonl` path). `claimed`: session ids or paths that other
    /// panes already use; they are never handed out again.
    public func resolve(_ needs: [Need], claimed: Set<String>) -> [String: String] {
        if remote { return [:] }
        let ids = Set(needs.map(\.paneId))
        cache = cache.filter { ids.contains($0.key) }
        let now = Date()
        let due = needs.filter { n in
            guard let e = cache[n.paneId] else { return true }
            return now.timeIntervalSince(e.checked) >= (e.path == nil ? retry : recheck)
        }
        if !due.isEmpty {
            let procs = Self.processes()
            for n in due {
                let agent = Self.agentProcess(paneId: n.paneId, socketPath: socketPath, procs: procs)
                if let old = cache[n.paneId], old.path != nil, old.pid != nil, old.pid == agent?.pid {
                    cache[n.paneId]?.checked = now
                    continue
                }
                var taken = claimed
                for (p, e) in cache where p != n.paneId { if let path = e.path { taken.insert(path) } }
                let path = agent.flatMap { find(n, agent: $0, procs: procs, claimed: taken) }
                cache[n.paneId] = Entry(pid: agent?.pid, path: path, checked: now)
            }
        }
        var out: [String: String] = [:]
        for n in needs { if let p = cache[n.paneId]?.path { out[n.paneId] = p } }
        return out
    }

    private func find(_ n: Need, agent: Proc, procs: [Proc], claimed: Set<String>) -> String? {
        let tree = Self.subtree(agent.pid, procs: procs)
        let open = Self.openFiles(pids: tree)
        let grok = n.kind == "grok" ? Self.grokHome(agent.line) : nil
        if let p = Self.logPath(openFiles: open, kind: n.kind, grokHome: grok), !Self.isClaimed(p, claimed) { return p }
        if let grok {   // Grok Build lists its running sessions with their pids
            let sessions = grok + "/sessions"
            guard let data = FileManager.default.contents(atPath: grok + "/active_sessions.json"),
                  let id = Self.grokSession(active: data, pids: tree),
                  let p = SessionLog.locateGrok(sessionId: id, sessionsDir: sessions)?.path, !Self.isClaimed(p, claimed) else { return nil }
            return p
        }
        guard n.kind == "codex" else { return nil }
        let c = Self.codexCandidates(dir: codexDir, cwd: n.cwd, after: agent.start).filter { !Self.isClaimed($0, claimed) }
        return c.count == 1 ? c[0] : nil
    }

    // MARK: - Pure parts (checked in HerdrKitChecks)

    /// `ps -E -ww -o pid=,ppid=,lstart=,command=` lines. `lstart` is five words in the C locale.
    public static func parsePS(_ text: String) -> [Proc] {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "EEE MMM d HH:mm:ss yyyy"
        var out: [Proc] = []
        for raw in text.split(separator: "\n") {
            let w = raw.split(separator: " ", maxSplits: 7, omittingEmptySubsequences: true)
            guard w.count == 8, let pid = Int32(w[0]), let ppid = Int32(w[1]),
                  let start = f.date(from: w[2...6].joined(separator: " ")) else { continue }
            out.append(Proc(pid: pid, ppid: ppid, start: start, line: String(w[7])))
        }
        return out
    }

    /// The agent process of a pane: the newest direct child of the pane's shell (the topmost process that carries
    /// the pane's env). Without a child, the topmost process itself (a pane started with the agent as its command).
    public static func agentProcess(paneId: String, socketPath: String, procs: [Proc]) -> Proc? {
        let pane = " HERDR_PANE_ID=\(paneId) ", sock = " HERDR_SOCKET_PATH=\(socketPath) "
        let mine = procs.filter { ($0.line + " ").contains(pane) && ($0.line + " ").contains(sock) }
        let pids = Set(mine.map(\.pid))
        let roots = mine.filter { !pids.contains($0.ppid) }
        let rootPids = Set(roots.map(\.pid))
        let kids = mine.filter { rootPids.contains($0.ppid) }
        return (kids.isEmpty ? roots : kids).max { $0.start < $1.start }
    }

    public static func subtree(_ pid: Int32, procs: [Proc]) -> [Int32] {
        var out = [pid], i = 0
        while i < out.count {
            let p = out[i]
            out += procs.filter { $0.ppid == p && !out.contains($0.pid) }.map(\.pid)
            i += 1
        }
        return out
    }

    /// The one agent log among open files, or nil when there is none or more than one. Grok Build keeps its session's
    /// `events.jsonl` open, not the conversation: the log is `updates.jsonl` in the same folder. `grokHome`: the agent's
    /// `GROK_HOME` (default `~/.grok`).
    public static func logPath(openFiles: [String], kind: String, grokHome: String? = nil) -> String? {
        let home = NSHomeDirectory()
        if kind == "grok" {
            let sessions = (grokHome ?? home + "/.grok") + "/sessions/"
            let dirs = Set(openFiles.compactMap { p -> String? in
                guard p.hasPrefix(sessions), !p.contains("/../") else { return nil }
                let rest = p.dropFirst(sessions.count).split(separator: "/")
                guard rest.count == 3, rest[2] == "events.jsonl" || rest[2] == "updates.jsonl" else { return nil }
                return sessions + rest[0] + "/" + rest[1]
            })
            return dirs.count == 1 ? dirs.first! + "/updates.jsonl" : nil
        }
        let hits = Set(openFiles.filter { p in
            guard p.hasSuffix(".jsonl"), !p.contains("/../") else { return false }
            switch kind {
            case "codex": return p.hasPrefix(home + "/.codex/sessions/") && (p as NSString).lastPathComponent.hasPrefix("rollout-")
            case "cursor": return p.hasPrefix(home + "/.cursor/projects/") && p.contains("/agent-transcripts/")
            case "pi": return p.hasPrefix(home + "/.pi/")
            default: return p.hasPrefix(home + "/.claude/projects/")
            }
        })
        return hits.count == 1 ? hits.first : nil
    }

    /// A path counts as claimed when it, or its session id (the file name ends in `<id>.jsonl`; Grok Build's
    /// `<id>/updates.jsonl`), is in `claimed`.
    public static func isClaimed(_ path: String, _ claimed: Set<String>) -> Bool {
        if claimed.contains(path) { return true }
        let name = (path as NSString).lastPathComponent
        let folder = name == "updates.jsonl" ? ((path as NSString).deletingLastPathComponent as NSString).lastPathComponent : nil
        return claimed.contains { !$0.isEmpty && !$0.contains("/") && (name == "\($0).jsonl" || name.hasSuffix("-\($0).jsonl") || folder == $0) }
    }

    /// Grok Build's home for an agent process: `GROK_HOME` from its env (`ps -E`), else `~/.grok`.
    public static func grokHome(_ procLine: String) -> String {
        if let m = (" " + procLine + " ").firstMatch(of: /\sGROK_HOME=(\S+)\s/) { return String(m.1) }
        return NSHomeDirectory() + "/.grok"
    }

    /// The session of the one `active_sessions.json` entry whose pid is one of `pids`; nil for none or several.
    public static func grokSession(active: Data, pids: [Int32]) -> String? {
        guard let list = try? JSONSerialization.jsonObject(with: active) as? [[String: Any]] else { return nil }
        let want = Set(pids.map(Int.init))
        let ids = Set(list.compactMap { e -> String? in
            guard let pid = e["pid"] as? Int, want.contains(pid), let id = e["session_id"] as? String, !id.isEmpty else { return nil }
            return id
        })
        return ids.count == 1 ? ids.first : nil
    }

    /// Codex rollouts (`YYYY/MM/DD/rollout-<local time>-<id>.jsonl`) that started at or after `after` (2 s slack:
    /// `lstart` has whole seconds) and whose `session_meta` cwd is `cwd`. Looks at the day folders from `after` on.
    public static func codexCandidates(dir: String, cwd: String, after: Date, now: Date = Date()) -> [String] {
        let fm = FileManager.default
        let cal = Calendar.current
        let names = DateFormatter()
        names.locale = Locale(identifier: "en_US_POSIX")
        names.dateFormat = "yyyy-MM-dd'T'HH-mm-ss"
        let lower = after.addingTimeInterval(-2)
        let want = (cwd as NSString).standardizingPath
        var out: [String] = []
        var day = cal.startOfDay(for: lower)
        var guardDays = 0
        while day <= now, guardDays < 8 {
            let c = cal.dateComponents([.year, .month, .day], from: day)
            let folder = String(format: "%@/%04d/%02d/%02d", dir, c.year ?? 0, c.month ?? 0, c.day ?? 0)
            for f in (try? fm.contentsOfDirectory(atPath: folder)) ?? [] where f.hasPrefix("rollout-") && f.hasSuffix(".jsonl") {
                guard let t = names.date(from: String(f.dropFirst(8).prefix(19))), t >= lower else { continue }
                let path = folder + "/" + f
                guard let meta = codexMeta(path), (meta.cwd as NSString).standardizingPath == want, meta.start >= lower else { continue }
                out.append(path)
            }
            day = cal.date(byAdding: .day, value: 1, to: day) ?? now.addingTimeInterval(1)
            guardDays += 1
        }
        return out.sorted()
    }

    /// cwd and start time from a rollout's first line (`session_meta`). Reads at most 1 MB.
    public static func codexMeta(_ path: String) -> (cwd: String, start: Date)? {
        guard let fh = FileHandle(forReadingAtPath: path) else { return nil }
        defer { try? fh.close() }
        guard let data = try? fh.read(upToCount: 1 << 20), let nl = data.firstIndex(of: 0x0A) ?? (data.isEmpty ? nil : data.endIndex),
              let o = try? JSONSerialization.jsonObject(with: data[..<nl]) as? [String: Any],
              o["type"] as? String == "session_meta", let p = o["payload"] as? [String: Any],
              let cwd = p["cwd"] as? String, let ts = p["timestamp"] as? String ?? o["timestamp"] as? String else { return nil }
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let start = iso.date(from: ts) ?? ISO8601DateFormatter().date(from: ts) else { return nil }
        return (cwd, start)
    }

    // MARK: - Subprocesses (read-only)

    static func processes() -> [Proc] {
        parsePS(run("/bin/ps", ["-xE", "-ww", "-o", "pid=,ppid=,lstart=,command="]))
    }

    static func openFiles(pids: [Int32]) -> [String] {
        guard !pids.isEmpty else { return [] }
        let out = run("/usr/sbin/lsof", ["-nP", "-Fn", "-p", pids.map(String.init).joined(separator: ",")])
        return out.split(separator: "\n").compactMap { $0.hasPrefix("n/") ? String($0.dropFirst()) : nil }
    }

    private static func run(_ tool: String, _ args: [String]) -> String {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: tool)
        p.arguments = args
        p.environment = ["LC_ALL": "C", "PATH": "/usr/bin:/bin:/usr/sbin:/sbin"]
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = FileHandle.nullDevice
        do { try p.run() } catch { return "" }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        return String(decoding: data, as: UTF8.self)
    }
}
