import CryptoKit
import Foundation

/// Conversations of agents on another machine (`box/t7`, parity item 3): the log is found there with `machine.exec`
/// (ps, Claude's `<pid>.json`, lsof, Codex rollouts' first lines; the same rules as `SessionResolver`) and mirrored
/// into a local file by polling `file.read {on, offset}`, so `SessionLog` tails it as any local log. Read-only on the
/// other machine: nothing is sent to its terminals.
extension HCClient {
    /// Tests: where mirrors go and how often they poll.
    public func setMirror(root: String, poll: TimeInterval, recheck: TimeInterval = 30) {
        mirrorRoot = root; mirrorPoll = poll; remoteRecheck = recheck; mirrorRootReady = false
    }

    /// Local mirror paths (paneId → absolute `.jsonl`) for the remote agent panes among `needs`. `claimed`: session
    /// ids or paths other panes use. Mirrors of logs no longer found stop.
    public func resolveRemote(_ needs: [SessionResolver.Need], claimed: Set<String>) async -> [String: String] {
        let remote = needs.compactMap { n -> (SessionResolver.Need, String, String)? in
            guard let a = HCMapper.address(fromPaneId: n.paneId), case let m = HCMapper.machinePrefix(a), !m.isEmpty else { return nil }
            return (n, String(m.dropLast()), a)
        }
        let now = Date()
        remoteLogs = remoteLogs.filter { k, _ in remote.contains { $0.0.paneId == k } }
        let due = remote.filter { n, _, _ in
            guard let e = remoteLogs[n.paneId] else { return true }
            return now.timeIntervalSince(e.checked) >= (e.path == nil ? min(5, remoteRecheck) : remoteRecheck)
        }
        if !due.isEmpty, let terms = try? await liveTerminals() {
            let byAddr = Dictionary(terms.compactMap { t in (t["address"] as? String).map { ($0, t) } }, uniquingKeysWith: { a, _ in a })
            var taken = claimed
            for (p, e) in remoteLogs where !due.contains(where: { $0.0.paneId == p }) { if let path = e.path { taken.insert(path) } }
            for machine in Set(due.map(\.1)).sorted() {
                guard let (home, procs) = try? await remoteProcesses(on: machine) else { continue }
                func agentOf(_ a: String, _ kind: String) -> (SessionResolver.Proc, [Int32])? {
                    let rec = byAddr[a] ?? [:]
                    let fg = (rec["foreground"] as? [String: Any])?["pid"] as? Int
                    let pids = [fg, rec["pid"] as? Int].compactMap { $0 }.filter { $0 > 0 }.map(Int32.init)
                    return SessionResolver.hcAgentProcess(pids: pids, kind: kind, procs: procs).map { ($0, pids) }
                }
                let here = remote.filter { HCMapper.machinePrefix($0.2) == machine + "/" }
                // Two pi in one cwd: each one's start splits the folder's files between them.
                let piStarts = here.filter { $0.0.kind == "pi" }.compactMap { n, _, a in
                    agentOf(a, "pi").map { (pane: n.paneId, cwd: (n.cwd as NSString).standardizingPath, start: $0.0.start) }
                }
                // Earliest agent first: it takes its file before a later one looks.
                let ordered = due.filter { HCMapper.machinePrefix($0.2) == machine + "/" }.map { ($0.0, agentOf($0.2, $0.0.kind)) }
                    .sorted { ($0.1?.0.start ?? .distantFuture, $0.0.paneId) < ($1.1?.0.start ?? .distantFuture, $1.0.paneId) }
                for (n, found) in ordered {
                    let old = remoteLogs[n.paneId]
                    guard let (agent, pids) = found else {
                        // Not in ps this time: a missed lookup, not another agent. Keep what was found.
                        if let old, old.path != nil, old.machine == machine { remoteLogs[n.paneId]?.checked = now }
                        else { remoteLogs[n.paneId] = RemoteLog(path: nil, checked: now) }
                        continue
                    }
                    let cwd = (n.cwd as NSString).standardizingPath
                    let rivals = piStarts.filter { $0.pane != n.paneId && $0.cwd == cwd }.map(\.start)
                    let previous = old?.pid == agent.pid && old?.machine == machine ? old?.path : nil
                    var path = await remoteLog(n, agent: agent, pids: pids, procs: procs, home: home, on: machine, claimed: taken,
                                               rivals: rivals, previous: previous)
                    // The same agent and nothing found: a failed exec or listing. Keep the last path until the pane goes
                    // or another agent runs in it; dropping it would delete the mirror and copy it again.
                    if path == nil, let previous, !taken.contains(previous) { path = previous }
                    if let path { taken.insert(path) }
                    remoteLogs[n.paneId] = RemoteLog(machine: machine, path: path, checked: now, pid: agent.pid)
                }
            }
        }
        var out: [String: String] = [:]
        var keep = Set<String>()
        for (p, e) in remoteLogs {
            guard let path = e.path, let m = e.machine else { continue }
            out[p] = startMirror(m, path)
            keep.insert(m + "\u{0}" + path)
        }
        for k in mirrors.keys where !keep.contains(k) { endMirror(k) }
        return out
    }

    /// Stops every mirror and deletes its copy.
    public func stopMirrors() {
        for k in Array(mirrors.keys) { endMirror(k) }
        rmdir(mirrorDir)   // only when empty
    }

    /// Stops one mirror (key: machine NUL remote path) and deletes its local copy: it is a private conversation.
    private func endMirror(_ key: String) {
        mirrors[key]?.cancel()
        mirrors[key] = nil
        let parts = key.split(separator: "\u{0}", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return }
        let local = mirrorPath(parts[0], parts[1])
        try? FileManager.default.removeItem(atPath: local)
        mirrorIssues[local] = nil
    }

    // MARK: - Finding the log there

    /// A shell line on `machine`, its stdout. C locale and UTC (`ps lstart`), at most 20 s.
    func exec(on machine: String, _ line: String) async throws -> String {
        let r = try await call("machine.exec", json: HCJSON(["on": machine, "line": line, "env": ["LC_ALL": "C", "TZ": "UTC0"], "timeout": 20]))
        return String(decoding: Data(base64Encoded: r["stdout"] as? String ?? "") ?? Data(), as: UTF8.self)
    }

    /// The machine's home and processes (one exec).
    func remoteProcesses(on machine: String) async throws -> (home: String, procs: [SessionResolver.Proc]) {
        let out = try await exec(on: machine, #"printf '%s\n' "$HOME"; ps -A -ww -o pid=,ppid=,lstart=,args="#)
        let lines = out.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: false)
        guard let home = lines.first.map(String.init), home.hasPrefix("/") else { throw HerdrError.protocolError("no home on \(machine)") }
        return (home, SessionResolver.parsePS(lines.count > 1 ? String(lines[1]) : "", timeZone: TimeZone(identifier: "UTC")!))
    }

    /// `SessionResolver.find` on another machine: Claude's pid file, else the one log its tree holds open, else (Codex)
    /// the one unclaimed rollout of its cwd that started after it, (pi) the newest file of its cwd's session folder.
    private func remoteLog(_ n: SessionResolver.Need, agent: SessionResolver.Proc, pids: [Int32], procs: [SessionResolver.Proc],
                           home: String, on machine: String, claimed: Set<String>,
                           rivals: [Date] = [], previous: String? = nil) async -> String? {
        // Claims are session ids, remote paths, or (another remote pane's) mirror paths.
        func taken(_ p: String) -> Bool { SessionResolver.isClaimed(p, claimed) || claimed.contains(mirrorPath(machine, p)) }
        if n.kind == "claude" {
            let order = SessionResolver.claudeSearchOrder(pids: [agent.pid] + pids, procs: procs).prefix(64)
            let line = #"cd "$HOME/.claude/sessions" 2>/dev/null || exit 0; for p in "#
                + order.map(String.init).joined(separator: " ")
                + #"; do [ -f "$p.json" ] && printf '%s\t' "$p" && tr '\n' ' ' < "$p.json" && echo; done"#
            guard let out = try? await exec(on: machine, line) else { return nil }
            var files: [Int32: Data] = [:]
            for l in out.split(separator: "\n") {
                let parts = l.split(separator: "\t", maxSplits: 1)
                if parts.count == 2, let pid = Int32(parts[0]) { files[pid] = Data(parts[1].utf8) }
            }
            guard let hit = SessionResolver.claudeSessionSource(pids: [agent.pid] + pids, procs: procs, read: { files[$0] }),
                  hit.sid.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" || $0 == "." }), !hit.sid.hasPrefix("."),
                  let ls = try? await exec(on: machine, #"ls -1 "$HOME"/.claude/projects/*/"# + "'\(hit.sid).jsonl' 2>/dev/null") else { return nil }
            let paths = ls.split(separator: "\n").map(String.init).filter { $0.hasPrefix(home + "/.claude/projects/") }
            guard paths.count == 1, !taken(paths[0]) else { return nil }
            return paths[0]
        }
        let tree = SessionResolver.subtree(agent.pid, procs: procs)
        if let out = try? await exec(on: machine, "lsof -nP -Fn -p " + tree.map(String.init).joined(separator: ",") + " 2>/dev/null") {
            let open = out.split(separator: "\n").compactMap { $0.hasPrefix("n/") ? String($0.dropFirst()) : nil }
            if let p = SessionResolver.logPath(openFiles: open, kind: n.kind, home: home), !taken(p) { return p }
        }
        if n.kind == "pi" {
            // Its cwd's session folder there: path, mtime (GNU stat, else BSD), first line.
            let dir = SessionResolver.piSessionDir(cwd: n.cwd, agentDir: home + "/.pi/agent")
            let line = "for f in " + Self.shellQuote(dir) + "/*.jsonl"
                + #"; do [ -f "$f" ] || continue; printf '%s\t%s\t' "$f" "$(stat -c %Y "$f" 2>/dev/null || stat -f %m "$f")"; head -n 1 "$f" | head -c 65536; echo; done"#
            guard let out = try? await exec(on: machine, line) else { return nil }
            let files: [(String, Date, Data)] = out.split(separator: "\n").compactMap { l in
                let p = l.split(separator: "\t", maxSplits: 2)
                guard p.count == 3, let t = TimeInterval(p[1].trimmingCharacters(in: .whitespaces)) else { return nil }
                return (String(p[0]), Date(timeIntervalSince1970: t), Data(p[2].utf8))
            }
            // Same agent, its file still there: keep it, so a recheck never flips or re-copies.
            if let previous, files.contains(where: { $0.0 == previous }), !taken(previous) { return previous }
            return SessionResolver.piPick(files, cwd: n.cwd, after: agent.start, rivals: rivals, claimed: taken)
        }
        guard n.kind == "codex" else { return nil }
        // Day folders a day early too: that machine may keep another time zone.
        let folders = SessionResolver.codexDayFolders(dir: home + "/.codex/sessions", after: agent.start, slackDays: 1)
        let line = "for f in " + folders.map { Self.shellQuote($0) + "/rollout-*.jsonl" }.joined(separator: " ")
            + #"; do [ -f "$f" ] || continue; printf '%s\t' "$f"; head -n 1 "$f" | head -c 1048576; echo; done"#
        guard let out = try? await exec(on: machine, line) else { return nil }
        let firsts: [(String, Data)] = out.split(separator: "\n").compactMap { l in
            let parts = l.split(separator: "\t", maxSplits: 1)
            return parts.count == 2 ? (String(parts[0]), Data(parts[1].utf8)) : nil
        }
        let c = SessionResolver.codexMatches(firsts, cwd: n.cwd, after: agent.start).filter { !taken($0) }
        return c.count == 1 ? c[0] : nil
    }

    static func shellQuote(_ s: String) -> String { "'" + s.replacingOccurrences(of: "'", with: #"'\''"#) + "'" }

    // MARK: - Mirroring

    /// This process's mirror folder: `<root>/<pid>`, so another Flockbook process (the app, a check, a headless mode)
    /// never touches these copies.
    var mirrorDir: String { mirrorRoot + "/" + String(getpid()) }

    /// A mirror copy's name: 16 lowercase hex, `-`, the log's name.
    static func isMirrorName(_ name: String) -> Bool {
        let u = Array(name.utf8)
        return u.count > 17 && u[16] == UInt8(ascii: "-") && u[..<16].allSatisfy { (0x30...0x39).contains($0) || (0x61...0x66).contains($0) }
    }

    /// Copies left by earlier runs: in `root`, only folders named by a pid whose process is gone (or this process's own,
    /// left by an earlier process with the same pid), only regular files with a mirror name, then the folder if empty.
    /// Nothing else is removed, and nothing recursively: the root may be any folder a caller gives.
    static func sweepMirrors(root: String) {
        let fm = FileManager.default
        func type(_ p: String) -> FileAttributeType? { (try? fm.attributesOfItem(atPath: p))?[.type] as? FileAttributeType }
        for d in (try? fm.contentsOfDirectory(atPath: root)) ?? [] {
            guard let pid = Int32(d), pid > 0, String(pid) == d else { continue }
            guard pid == getpid() || (kill(pid, 0) != 0 && errno == ESRCH) else { continue }
            let dir = root + "/" + d
            guard type(dir) == .typeDirectory else { continue }   // not a link to elsewhere
            for f in (try? fm.contentsOfDirectory(atPath: dir)) ?? [] where isMirrorName(f) && type(dir + "/" + f) == .typeRegular {
                unlink(dir + "/" + f)
            }
            if pid != getpid() { rmdir(dir) }   // only when empty
        }
    }

    /// The local file for a remote log: `<root>/<pid>/<sha256(machine NUL path), 16 hex>-<log name>`, one flat folder,
    /// so no machine or path can collide with another or point outside it. The name is cut to 120 bytes.
    func mirrorPath(_ machine: String, _ remote: String) -> String {
        let hash = SHA256.hash(data: Data((machine + "\u{0}" + remote).utf8)).prefix(8).map { String(format: "%02x", $0) }.joined()
        var name = (remote as NSString).lastPathComponent.replacingOccurrences(of: "/", with: "_")
        while name.utf8.count > 120 { name.removeFirst() }
        if name.isEmpty || name == "." || name == ".." { name = "log" }
        return mirrorDir + "/" + hash + "-" + name
    }

    /// Starts (once) the poller that copies `remote` on `machine` into its mirror; returns the mirror's path. The file
    /// exists (maybe empty) when this returns.
    private func startMirror(_ machine: String, _ remote: String) -> String {
        let local = mirrorPath(machine, remote)
        let key = machine + "\u{0}" + remote
        guard mirrors[key] == nil else { return local }
        let fm = FileManager.default
        let owner: [FileAttributeKey: Any] = [.posixPermissions: 0o700]
        if !mirrorRootReady {
            // Copies left by gone processes (crash, quit) are not read again; this process starts with an empty,
            // owner-only folder of its own.
            try? fm.createDirectory(atPath: mirrorRoot, withIntermediateDirectories: true, attributes: owner)
            Self.sweepMirrors(root: mirrorRoot)
            try? fm.createDirectory(atPath: mirrorDir, withIntermediateDirectories: true, attributes: owner)
            try? fm.setAttributes(owner, ofItemAtPath: mirrorDir)
            mirrorRootReady = true
        }
        try? fm.createDirectory(atPath: (local as NSString).deletingLastPathComponent, withIntermediateDirectories: true, attributes: owner)
        fm.createFile(atPath: local, contents: Data(), attributes: [.posixPermissions: 0o600])
        let poll = mirrorPoll
        mirrors[key] = Task { [weak self] in
            var st = MirrorState()
            while !Task.isCancelled, let self {
                let step = await self.mirrorOnce(machine, remote, local, &st)
                // Failing reads back off (up to 30 s): a gone machine or hc is not asked every poll.
                let wait = switch step {
                case .grew: poll
                case .idle: poll * 2
                case .retry: 0.0
                case .failed: min(30, poll * 2 * pow(2, Double(min(st.failures, 6))))
                }
                if wait > 0 { try? await Task.sleep(nanoseconds: UInt64(wait * 1e9)) }
            }
        }
        return local
    }

    /// What went wrong copying each mirror (path → message): lines skipped, reads failing.
    public func mirrorProblems() -> [String: String] { mirrorIssues }

    private func note(_ local: String, _ msg: String) {
        var all = mirrorIssues[local].map { $0 + "; " } ?? ""
        if all.count > 2000 { all = "" }
        mirrorIssues[local] = all + msg
        NSLog("Flockbook remote log %@: %@", (local as NSString).lastPathComponent, msg)
    }

    enum MirrorStep { case grew, idle, retry, failed }

    /// One `file.read` from line `next`: whole lines are appended (a last line without its end waits); a remote file
    /// with fewer lines than copied was rewritten, so the copy starts over. hc gives at most 8 MB: a single longer line
    /// is skipped and noted. (hc reads by line number only, no byte offset: it counts the file's lines on every read.)
    private func mirrorOnce(_ machine: String, _ remote: String, _ local: String, _ st: inout MirrorState) async -> MirrorStep {
        let r: HCJSON
        do {
            r = try await call("file.read", json: HCJSON(["on": machine, "path": remote, "offset": st.next, "limit": MirrorState.chunk]))
        } catch HerdrError.server(code: "invalid_params", let msg) where msg.contains("over 8 MB") {
            note(local, "line \(st.next) is over 8 MB: skipped")
            st.next += 1
            return .retry
        } catch {
            // `not_text` (the file is not UTF-8: hc refuses it from any line), a gone machine, hc down: back off.
            st.failures += 1
            if st.failures == 1, case HerdrError.server(code: "not_text", _) = error { note(local, "the file is not UTF-8 text: copy stopped") }
            else if st.failures == 3 { note(local, "reads failing: \(error)") }
            return .failed
        }
        st.failures = 0
        // Ended while the read was under way: the file may already be the next mirror of this log. No
        // suspension point lies between this check and the writes below.
        guard !Task.isCancelled else { return .idle }
        let lines = r["lines"] as? Int ?? 0
        if lines < st.next - 1 {
            FileManager.default.createFile(atPath: local, contents: Data(), attributes: [.posixPermissions: 0o600])
            st.next = 1
            return .grew
        }
        let text = r["text"] as? String ?? ""
        guard let cut = text.lastIndex(of: "\n") else { return .idle }
        let whole = text[...cut]
        guard let fh = FileHandle(forWritingAtPath: local) else { return .idle }
        defer { try? fh.close() }
        do {
            try fh.seekToEnd()
            try fh.write(contentsOf: Data(whole.utf8))
        } catch { return .idle }
        st.next += whole.reduce(0) { $1 == "\n" ? $0 + 1 : $0 }
        return .grew
    }
}

/// Where one mirror stands: the next remote line and failed reads in a row.
struct MirrorState {
    /// Most lines per read.
    static let chunk = 20000
    var next = 1
    var failures = 0
}

/// One remote pane's log: the machine and the path there (nil: not found yet), and when it was looked for.
struct RemoteLog: Sendable {
    var machine: String?
    var path: String?
    var checked: Date
    /// The agent's pid there when the path was found.
    var pid: Int32?
    init(machine: String? = nil, path: String?, checked: Date, pid: Int32? = nil) {
        self.machine = machine; self.path = path; self.checked = checked; self.pid = pid
    }
}
