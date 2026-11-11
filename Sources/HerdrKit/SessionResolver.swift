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
/// hc Claude panes also watch the mtime of `<pid>.json`: Claude rewrites its `sessionId` there on /clear or /resume
/// without a new process. agy panes run `lsof` on every recheck: /new and /resume open another `presence/<id>.lock`
/// in the same process.
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
        /// hc panes: process ids to start from (foreground first). Empty for TUI panes (found by `HERDR_PANE_ID`).
        public var pids: [Int32]
        public init(paneId: String, kind: String, cwd: String, pids: [Int32] = []) {
            self.paneId = paneId; self.kind = kind; self.cwd = cwd; self.pids = pids
        }
    }

    /// `stamp`: mtime of the Claude pid file the entry came from (hc Claude panes only); `stampPid` names that file.
    /// It may be a child of the agent process, so it is kept apart from `pid`. `needPids`: the hc pane's pids then; a
    /// change (another program in the terminal) makes the entry due at once.
    private struct Entry { var pid: Int32?; var path: String?; var checked: Date; var stamp: Date? = nil; var stampPid: Int32? = nil
                           var needPids: [Int32] = [] }
    private var cache: [String: Entry] = [:]
    private let socketPath: String
    private let codexDir: String
    private let projectsDir: String
    /// The most bytes read of one log to judge it.
    private let verdictCap: Int
    /// How many times a log was read to judge it.
    public private(set) var verdictReads = 0
    private let claudeDir: String
    private let piAgentDir: String
    /// omp's sessions root, unless its process sets `PI_CODING_AGENT_DIR`.
    private let ompSessionsDir: String
    private let agyDir: String
    /// The home folder the open-file route matches logs under (`~/.codex/sessions/`, `~/.claude/projects/`, ...).
    private let home: String
    /// `lsof` of a process tree (injected by checks).
    private let lsof: @Sendable ([Int32]) -> [String]
    /// Unresolved panes are tried again after this; resolved ones are re-checked (pid still the same) after `recheck`.
    private let retry: TimeInterval
    private let recheck: TimeInterval
    /// The session runs on another machine: the resolver is off (see above).
    private let remote: Bool

    public init(socketPath: String, codexDir: String = SessionLog.defaultDir(agent: "codex"),
                projectsDir: String = SessionLog.defaultDir(agent: "claude"),
                claudeDir: String = NSHomeDirectory() + "/.claude/sessions",
                piAgentDir: String = NSHomeDirectory() + "/.pi/agent",
                ompSessionsDir: String = NSHomeDirectory() + "/.omp/agent/sessions",
                agyDir: String = NSHomeDirectory() + "/.gemini/antigravity-cli", home: String = NSHomeDirectory(),
                retry: TimeInterval = 5, recheck: TimeInterval = 30, verdictCap: Int = 8 << 20, remote: RemoteHost? = nil,
                openFiles: (@Sendable ([Int32]) -> [String])? = nil) {
        self.socketPath = socketPath; self.codexDir = codexDir; self.projectsDir = projectsDir; self.verdictCap = verdictCap
        self.claudeDir = claudeDir; self.piAgentDir = piAgentDir; self.ompSessionsDir = ompSessionsDir; self.agyDir = agyDir; self.home = home
        self.lsof = openFiles ?? { Self.openFiles(pids: $0) }
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
    /// `claimedCwd`: the cwd of the pane behind a claim, where known. herdr's TUI has reported another folder's Codex
    /// rollout for a pane; such a claim (pane cwd ≠ the rollout's cwd) does not hide a rollout from the Codex cwd + time match.
    /// `procs`: a process list to use instead of running `ps` (checks).
    public func resolve(_ needs: [Need], claimed: Set<String>, claimedCwd: [String: String] = [:],
                        procs given: [Proc]? = nil) -> [String: String] {
        if remote { return [:] }
        let ids = Set(needs.map(\.paneId))
        cache = cache.filter { ids.contains($0.key) }
        let now = Date()
        /// The pid file changed since the entry was made (a stat, no `ps`).
        func pidFileChanged(_ n: Need, _ e: Entry) -> Bool {
            guard e.stamp != nil, let pid = e.stampPid ?? e.pid else { return false }
            return (pidFileStamp(pid) ?? .distantPast) != e.stamp   // no file: distantPast both times, not a change
        }
        let due = needs.filter { n in
            guard let e = cache[n.paneId] else { return true }
            return now.timeIntervalSince(e.checked) >= (e.path == nil ? retry : recheck) || pidFileChanged(n, e) || e.needPids != n.pids
        }
        if !due.isEmpty {
            let procs = given ?? Self.processes()
            func agentOf(_ n: Need) -> Proc? {
                n.pids.isEmpty ? Self.agentProcess(paneId: n.paneId, socketPath: socketPath, procs: procs)
                               : Self.hcAgentProcess(pids: n.pids, kind: n.kind, procs: procs)
            }
            // Two pi (or omp) in one cwd with one session folder: each one's start splits its files between them. Both
            // count: two cwds can name one folder (/w/a-b, /w/a/b), one cwd can have two (omp's PI_CODING_AGENT_DIR).
            let piStarts: [(pane: String, dir: String, cwd: String, start: Date)] = needs.filter { Self.piLike($0.kind) }.compactMap { n in
                let cwd = (n.cwd as NSString).standardizingPath
                return agentOf(n).map { (n.paneId, piDir(kind: n.kind, cwd: cwd, line: $0.line), cwd, $0.start) }
            }
            // Earliest agent first: it takes its file before a later one looks.
            let ordered = due.map { ($0, agentOf($0)) }
                .sorted { ($0.1?.start ?? .distantFuture, $0.0.paneId) < ($1.1?.start ?? .distantFuture, $1.0.paneId) }
            for (n, agent) in ordered {
                // agy: /new or /resume swaps the presence lock in the same process, so every recheck reads it again.
                if let old = cache[n.paneId], let path = old.path, old.pid != nil, old.pid == agent?.pid, !pidFileChanged(n, old),
                   n.kind != "agy", !Self.piLike(n.kind) || FileManager.default.fileExists(atPath: path) {
                    cache[n.paneId]?.checked = now
                    cache[n.paneId]?.needPids = n.pids
                    continue
                }
                var taken = claimed
                for (p, e) in cache where p != n.paneId { if let path = e.path { taken.insert(path) } }
                let mine = piStarts.first { $0.pane == n.paneId }
                let rivals = piStarts.filter { $0.pane != n.paneId && $0.dir == mine?.dir && $0.cwd == mine?.cwd }.map(\.start)
                let found = agent.map { find(n, agent: $0, procs: procs, claimed: taken, claimedCwd: claimedCwd, rivals: rivals) }
                let claudeHC = !n.pids.isEmpty && n.kind == "claude"
                // Unresolved: watch the agent's own file, where a new session id would most likely appear.
                let stampPid = found?.source ?? agent?.pid
                cache[n.paneId] = Entry(pid: agent?.pid, path: found?.path, checked: now,
                                        stamp: claudeHC ? stampPid.flatMap { pidFileStamp($0) } ?? .distantPast : nil,
                                        stampPid: claudeHC ? stampPid : nil, needPids: n.pids)
            }
        }
        var out: [String: String] = [:]
        for n in needs { if let p = cache[n.paneId]?.path { out[n.paneId] = p } }
        return out
    }

    /// The session folder of a pi or omp agent (`line`: its process with the env) in `cwd`.
    private func piDir(kind: String, cwd: String, line: String) -> String {
        guard kind == "omp" else { return Self.piSessionDir(cwd: cwd, agentDir: piAgentDir) }
        // omp resolves both from its own cwd (path.resolve)
        func abs(_ p: String) -> String { p.hasPrefix("/") ? p : Self.lexical(cwd + "/" + p) }
        let sessions = Self.envValue(line, "PI_CODING_AGENT_DIR").map { Self.dropSlashes(abs($0)) + "/sessions" } ?? ompSessionsDir
        let tmp = Self.envValue(line, "TMPDIR").map(abs) ?? NSTemporaryDirectory()
        return Self.ompSessionDir(cwd: Self.realPath(cwd), sessions: sessions, home: Self.realPath(home), tmp: Self.realPath(tmp))
    }

    private func pidFileStamp(_ pid: Int32) -> Date? {
        (try? FileManager.default.attributesOfItem(atPath: "\(claudeDir)/\(pid).json"))?[.modificationDate] as? Date
    }

    /// The log path (or Claude session id) and, for hc Claude, the pid whose `<pid>.json` named it.
    private func find(_ n: Need, agent: Proc, procs: [Proc], claimed: Set<String>,
                      claimedCwd: [String: String], rivals: [Date]) -> (path: String?, source: Int32?) {
        if !n.pids.isEmpty, n.kind == "claude" {   // hc: Claude's own pid file names the session id
            guard let hit = Self.claudeSessionSource(pids: [agent.pid] + n.pids, procs: procs, dir: claudeDir) else { return (nil, nil) }
            return Self.isClaimed(hit.sid, claimed) ? (nil, hit.pid) : (hit.sid, hit.pid)
        }
        let tree = Self.subtree(agent.pid, procs: procs)
        var open = lsof(tree)
        if n.kind == "codex" {   // Codex also keeps its subagents' rollouts open: only the pane's own one counts
            open.removeAll { ($0 as NSString).lastPathComponent.hasPrefix("rollout-") && Self.codexSubagentRollout($0) }
        }
        if n.kind == "agy" {   // agy keeps no log open: its presence lock names the conversation
            guard let p = Self.agyLogPath(openFiles: open, dataDir: agyDir), !Self.isClaimed(p, claimed) else { return (nil, nil) }
            return (p, nil)
        }
        let grok = n.kind == "grok" ? Self.grokHome(agent.line) : nil
        if let p = Self.logPath(openFiles: open, kind: n.kind, grokHome: grok, home: home), !Self.isClaimed(p, claimed) { return (p, nil) }
        if let grok {   // Grok Build lists its running sessions with their pids
            let sessions = grok + "/sessions"
            guard let data = FileManager.default.contents(atPath: grok + "/active_sessions.json"),
                  let id = Self.grokSession(active: data, pids: tree),
                  let p = SessionLog.locateGrok(sessionId: id, sessionsDir: sessions)?.path, !Self.isClaimed(p, claimed) else { return (nil, nil) }
            return (p, nil)
        }
        if Self.piLike(n.kind) {   // pi and omp keep no file open: their cwd's session folder
            let dir = piDir(kind: n.kind, cwd: n.cwd, line: agent.line)
            let files: [(String, Date, Data)] = ((try? FileManager.default.contentsOfDirectory(atPath: dir)) ?? []).compactMap { f in
                guard f.hasSuffix(".jsonl") else { return nil }
                let path = dir + "/" + f
                guard let m = (try? FileManager.default.attributesOfItem(atPath: path))?[.modificationDate] as? Date,
                      let fh = FileHandle(forReadingAtPath: path) else { return nil }
                defer { try? fh.close() }
                return (path, m, (try? fh.read(upToCount: 1 << 16)) ?? Data())
            }
            return (Self.piPick(files, cwd: n.cwd, after: agent.start, rivals: rivals) { Self.isClaimed($0, claimed) }, nil)
        }
        guard n.kind == "codex" else { return (nil, nil) }
        // Every candidate's session_meta cwd is `n.cwd`: a claim by a pane elsewhere is a wrong report, not a rival.
        let c = Self.codexCandidates(dir: codexDir, cwd: n.cwd, after: agent.start)
            .filter { !Self.isClaimed($0, claimed, cwd: n.cwd, claimedCwd: claimedCwd) }
        return (c.count == 1 ? c[0] : nil, nil)
    }

    // MARK: - Pure parts (checked in HerdrKitChecks)

    /// `ps -E -ww -o pid=,ppid=,lstart=,command=` lines. `lstart` is five words in the C locale.
    /// `timeZone`: the one `ps` printed `lstart` in (another machine's run with `TZ=UTC0`).
    public static func parsePS(_ text: String, timeZone: TimeZone = .current) -> [Proc] {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = timeZone
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

    /// Executable names per agent kind, matched against the first two words of a command line (`node …/codex`).
    static let executables: [String: Set<String>] = ["claude": ["claude"], "codex": ["codex"], "cursor": ["cursor-agent", "cursor"], "pi": ["pi"]]

    static func runs(_ p: Proc, kind: String) -> Bool {
        let names = executables[kind] ?? [kind]
        let words = p.line.split(separator: " ", maxSplits: 2).prefix(2).map { ($0 as NSString).lastPathComponent }
        return words.contains { names.contains($0) }
    }

    /// hc: the agent process of a terminal from its pids (foreground first, then the terminal's own process).
    /// A pid that runs the agent itself wins; else the newest process in their trees that does.
    public static func hcAgentProcess(pids: [Int32], kind: String, procs: [Proc]) -> Proc? {
        let byPid = Dictionary(procs.map { ($0.pid, $0) }, uniquingKeysWith: { a, _ in a })
        for pid in pids { if let p = byPid[pid], runs(p, kind: kind) { return p } }
        let tree = Set(pids.flatMap { subtree($0, procs: procs) })
        return procs.filter { tree.contains($0.pid) && runs($0, kind: kind) }.max { $0.start < $1.start }
    }

    /// Claude Code writes `~/.claude/sessions/<pid>.json` with `pid`, `sessionId` and `procStart` (UTC, `ps lstart`
    /// format). Looks at the given pids and their process trees; a file counts only when its `procStart` matches the
    /// live process start within 2 s (pids get reused). Exactly one session id, or nil.
    public static func claudeSession(pids: [Int32], procs: [Proc], dir: String) -> String? {
        claudeSessionSource(pids: pids, procs: procs, dir: dir)?.sid
    }

    /// `claudeSession` plus the pid whose file named it (the first one in search order).
    public static func claudeSessionSource(pids: [Int32], procs: [Proc], dir: String) -> (sid: String, pid: Int32)? {
        claudeSessionSource(pids: pids, procs: procs) { FileManager.default.contents(atPath: "\(dir)/\($0).json") }
    }

    /// The pids whose `<pid>.json` may name the session: the given ones, then their process trees.
    public static func claudeSearchOrder(pids: [Int32], procs: [Proc]) -> [Int32] {
        var order: [Int32] = []
        for p in pids + pids.flatMap({ subtree($0, procs: procs) }) where !order.contains(p) { order.append(p) }
        return order
    }

    /// `read`: the contents of `<pid>.json` (here, or fetched from another machine).
    public static func claudeSessionSource(pids: [Int32], procs: [Proc], read: (Int32) -> Data?) -> (sid: String, pid: Int32)? {
        let byPid = Dictionary(procs.map { ($0.pid, $0) }, uniquingKeysWith: { a, _ in a })
        let order = claudeSearchOrder(pids: pids, procs: procs)
        let utc = DateFormatter()
        utc.locale = Locale(identifier: "en_US_POSIX"); utc.timeZone = TimeZone(identifier: "UTC"); utc.dateFormat = "EEE MMM d HH:mm:ss yyyy"
        var found = Set<String>()
        var source: Int32?
        for pid in order {
            guard let proc = byPid[pid],
                  let data = read(pid), data.count < 1 << 20,
                  let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  (o["pid"] as? NSNumber)?.int32Value == pid,
                  let sid = o["sessionId"] as? String, !sid.isEmpty, !sid.contains("/"),
                  let ps = (o["procStart"] as? String).flatMap({ utc.date(from: $0.split(separator: " ", omittingEmptySubsequences: true).joined(separator: " ")) }),
                  abs(ps.timeIntervalSince(proc.start)) <= 2 else { continue }
            found.insert(sid)
            if source == nil { source = pid }
        }
        guard found.count == 1, let sid = found.first, let source else { return nil }
        return (sid, source)
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
    /// `GROK_HOME` (default `~/.grok`). `home`: the home folder of the machine the files are on.
    public static func logPath(openFiles: [String], kind: String, grokHome: String? = nil, home: String = NSHomeDirectory()) -> String? {
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
            case "omp": return p.hasPrefix(home + "/.omp/")
            default: return p.hasPrefix(home + "/.claude/projects/")
            }
        })
        return hits.count == 1 ? hits.first : nil
    }

    /// agy holds `<dataDir>/presence/<conversation id>.lock` open while a conversation is active (none before the first
    /// prompt). Exactly one such lock, and its `brain/<id>/.system_generated/logs/transcript_full.jsonl` exists: that path.
    public static func agyLogPath(openFiles: [String],
                                  dataDir: String = NSHomeDirectory() + "/.gemini/antigravity-cli") -> String? {
        let dir = dataDir + "/presence/"
        let ids = Set(openFiles.compactMap { p -> String? in
            guard p.hasPrefix(dir), p.hasSuffix(".lock") else { return nil }
            let id = String(p.dropFirst(dir.count).dropLast(5))
            return UUID(uuidString: id) != nil ? id.lowercased() : nil
        })
        guard ids.count == 1, let id = ids.first else { return nil }
        let path = "\(dataDir)/brain/\(id)/.system_generated/logs/transcript_full.jsonl"
        return FileManager.default.fileExists(atPath: path) ? path : nil
    }

    /// A path counts as claimed when it, or its session id (the file name ends in `<id>.jsonl`), is in `claimed`.
    /// `isClaimed`, ignoring claims whose pane is known to sit in another cwd than `cwd` (the log's own cwd).
    static func isClaimed(_ path: String, _ claimed: Set<String>, cwd: String, claimedCwd: [String: String]) -> Bool {
        let want = (cwd as NSString).standardizingPath
        return claimed.contains { c in
            guard isClaimed(path, [c]) else { return false }
            guard let other = claimedCwd[c] else { return true }
            return (other as NSString).standardizingPath == want
        }
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
        guard let data = try? fh.read(upToCount: 1 << 20) else { return nil }
        return codexMeta(data: data)
    }

    /// The day folders (`<dir>/YYYY/MM/DD`) a rollout started at or after `after` can be in, up to `now` (at most 8).
    /// `slackDays`: days added before `after` and after `now` (another machine may keep another time zone).
    public static func codexDayFolders(dir: String, after: Date, now: Date = Date(), slackDays: Int = 0) -> [String] {
        let cal = Calendar.current
        var day = cal.date(byAdding: .day, value: -slackDays, to: cal.startOfDay(for: after.addingTimeInterval(-2))) ?? after
        var out: [String] = []
        // Slack after `now` too: a machine ahead in time zone writes tomorrow's folder.
        let last = cal.date(byAdding: .day, value: slackDays, to: now) ?? now
        while day <= last, out.count < 8 + 2 * slackDays {
            let c = cal.dateComponents([.year, .month, .day], from: day)
            out.append(String(format: "%@/%04d/%02d/%02d", dir, c.year ?? 0, c.month ?? 0, c.day ?? 0))
            day = cal.date(byAdding: .day, value: 1, to: day) ?? now.addingTimeInterval(1)
        }
        return out
    }

    /// The rollouts among `(path, first line)` whose `session_meta` cwd is `cwd` and that started at or after `after`
    /// (2 s slack), sorted.
    public static func codexMatches(_ firstLines: [(String, Data)], cwd: String, after: Date) -> [String] {
        let want = (cwd as NSString).standardizingPath, lower = after.addingTimeInterval(-2)
        return firstLines.compactMap { path, line in
            guard let meta = codexMeta(data: line), (meta.cwd as NSString).standardizingPath == want, meta.start >= lower else { return nil }
            return path
        }.sorted()
    }

    /// pi's session folder for a cwd (pi session-manager.js getDefaultSessionDirPath).
    public static func piSessionDir(cwd: String, agentDir: String) -> String {
        var c = Substring(cwd)
        if c.first == "/" || c.first == "\\" { c = c.dropFirst() }
        return agentDir + "/sessions/--" + String(c.map { "/\\:".contains($0) ? "-" : $0 }) + "--"
    }

    /// Agents that write pi's log: pi and its fork omp (oh-my-pi).
    static func piLike(_ kind: String) -> Bool { kind == "pi" || kind == "omp" }

    /// omp's session folder for a cwd (omp session-paths.ts getDefaultSessionDirName). All paths resolved already: the
    /// temp root first (`-tmp-<rel>`), then home (`-<rel>`), else `--<absolute>--`; `/`, `\` and `:` become `-`.
    public static func ompSessionDir(cwd: String, sessions: String, home: String, tmp: String) -> String {
        func enc(_ s: some StringProtocol) -> String { String(s.map { "/\\:".contains($0) ? "-" : $0 }) }
        func within(_ root: String) -> Substring? {
            let root = dropSlashes(root)
            if root.isEmpty { return nil }
            if cwd == root { return "" }
            guard cwd.hasPrefix(root), let c = cwd.dropFirst(root.count).first, c == "/" || c == "\\" else { return nil }
            return cwd.dropFirst(root.count + 1)
        }
        let name: String
        if let rel = within(tmp) {
            name = rel.isEmpty ? "-tmp" : "-tmp-" + enc(rel)
        } else if let rel = within(home) {
            name = "-" + enc(rel)
        } else {
            name = "--" + enc(cwd.first == "/" || cwd.first == "\\" ? cwd.dropFirst() : cwd[...]) + "--"
        }
        return sessions + "/" + name
    }

    /// The value of `key` in an agent process's env (`ps -E`), if set and not empty. `ps` joins the env with spaces, so
    /// a value runs to the next ` NAME=` (a path with spaces stays whole).
    static func envValue(_ procLine: String, _ key: String) -> String? {
        let s = " " + procLine + " "
        guard let at = s.range(of: " \(key)=") else { return nil }
        var rest = s[at.upperBound...]
        if let next = rest.firstMatch(of: /\ [A-Za-z_][A-Za-z0-9_]*=/) { rest = rest[..<next.range.lowerBound] }
        let value = String(rest).replacing(/\s+$/, with: "")
        return value.isEmpty ? nil : value
    }

    /// The path with its links resolved, as Node's realpath; as given when it cannot be resolved.
    static func realPath(_ p: String) -> String {
        guard let r = realpath(p, nil) else { return dropSlashes(p) }
        defer { free(r) }
        return String(cString: r)
    }

    /// The path without the slashes at its end.
    static func dropSlashes(_ p: String) -> String {
        var s = Substring(p)
        while s.last == "/" || s.last == "\\" { s = s.dropLast() }
        return String(s)
    }

    /// An absolute path without `.`, `..` and empty parts; links are not resolved.
    static func lexical(_ p: String) -> String {
        var parts: [Substring] = []
        for c in p.split(separator: "/") where c != "." { if c == ".." { _ = parts.popLast() } else { parts.append(c) } }
        return "/" + parts.joined(separator: "/")
    }

    /// pi's file among `(path, mtime, first line)`: a `session` header with this cwd, started or written at or after
    /// `after` (2 s slack; a resumed session is an older file pi writes to), not claimed; the newest by mtime (pi's
    /// `/new` starts a newer file in the same process). `rivals`: start times of the other pi in this cwd. pi stamps
    /// the header when it makes the session (at start) and writes the file later, so a header at or after a later
    /// rival's start is that rival's file, even when it is the newest.
    public static func piPick(_ files: [(String, Date, Data)], cwd: String, after: Date, rivals: [Date] = [], claimed: (String) -> Bool) -> String? {
        let later = rivals.filter { $0 > after }
        let want = (cwd as NSString).standardizingPath, lower = after.addingTimeInterval(-2)
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return files.filter { path, mtime, data in
            // omp writes a fixed-width `title` slot before the header
            var rest = data[...], header: [String: Any]?
            for _ in 0..<2 {
                let nl = rest.firstIndex(of: 0x0A) ?? rest.endIndex
                guard let o = try? JSONSerialization.jsonObject(with: rest[..<nl]) as? [String: Any] else { return false }
                if o["type"] as? String != "title" { header = o; break }
                rest = rest[(nl < rest.endIndex ? rest.index(after: nl) : nl)...]
            }
            guard let o = header, o["type"] as? String == "session",
                  let c = o["cwd"] as? String, (c as NSString).standardizingPath == want else { return false }
            let started = (o["timestamp"] as? String).flatMap { iso.date(from: $0) ?? ISO8601DateFormatter().date(from: $0) }
            if let t = started, later.contains(where: { t >= $0 }) { return false }
            return ((started ?? .distantPast) >= lower || mtime >= lower) && !claimed(path)
        }.max { ($0.1, $0.0) < ($1.1, $1.0) }?.0
    }

    /// cwd and start time from a rollout's first line.
    public static func codexMeta(data: Data) -> (cwd: String, start: Date)? {
        guard let nl = data.firstIndex(of: 0x0A) ?? (data.isEmpty ? nil : data.endIndex),
              let o = try? JSONSerialization.jsonObject(with: data[..<nl]) as? [String: Any],
              o["type"] as? String == "session_meta", let p = o["payload"] as? [String: Any], !isSubagentSource(p),
              let cwd = p["cwd"] as? String, let ts = p["timestamp"] as? String ?? o["timestamp"] as? String else { return nil }
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let start = iso.date(from: ts) ?? ISO8601DateFormatter().date(from: ts) else { return nil }
        return (cwd, start)
    }

    /// A subagent's rollout (the approvals reviewer "guardian", `spawn_agent` helpers): `source` is `{"subagent": ...}`.
    static func isSubagentSource(_ payload: [String: Any]) -> Bool {
        (payload["source"] as? [String: Any])?["subagent"] != nil
    }

    /// Whether the rollout at `path` belongs to a subagent of the Codex that keeps it open, not to the pane.
    static func codexSubagentRollout(_ path: String) -> Bool {
        guard let fh = FileHandle(forReadingAtPath: path) else { return false }
        defer { try? fh.close() }
        guard let data = try? fh.read(upToCount: 1 << 20), !data.isEmpty else { return false }
        let nl = data.firstIndex(of: 0x0A) ?? data.endIndex
        guard let o = try? JSONSerialization.jsonObject(with: data[..<nl]) as? [String: Any],
              let p = o["payload"] as? [String: Any] else { return false }
        return isSubagentSource(p)
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
