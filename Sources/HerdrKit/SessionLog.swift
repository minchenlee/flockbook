import Foundation

/// Result of reading an older log range. A failure preserves the current range and can be retried.
public enum EarlierLoadResult: Sendable, Equatable {
    case more
    case end
    case failed
}

/// Finds and tails an agent's JSONL session log (Claude Code, Codex, pi, …).
public final class SessionLog: Sendable {
    public let sessionId: String
    /// `Agent.kind`: "claude", "codex", "pi", "cursor", "grok", …
    public let agent: String
    public var isCodex: Bool { agent == "codex" }
    /// The host the agent runs on; nil: this Mac. A remote log is read through a local `RemoteLogMirror`.
    public let remote: RemoteHost?
    /// `sessionId` is herdr's `agent_session.value`: an id, or (kind "path", e.g. pi) the log file path itself.
    public init(sessionId: String, agent: String = "claude", remote: RemoteHost? = nil) {
        self.sessionId = sessionId; self.agent = agent; self.remote = remote
    }
    public convenience init(sessionId: String, codex: Bool) { self.init(sessionId: sessionId, agent: codex ? "codex" : "claude") }

    /// The log file the tailer reads; nil until `snapshots` found it. For a remote log: the local mirror file.
    public var url: URL? { state.lock.withLock { state.url } }

    /// Where an agent keeps its logs (the search root for an id).
    /// Grok Build: `$GROK_HOME/sessions`, else `~/.grok/sessions`.
    public static func defaultDir(agent: String, env: [String: String] = ProcessInfo.processInfo.environment) -> String {
        if agent == "grok" { return (env["GROK_HOME"].flatMap { $0.isEmpty ? nil : $0 } ?? NSHomeDirectory() + "/.grok") + "/sessions" }
        return NSHomeDirectory() + (agent == "codex" ? "/.codex/sessions" : agent == "cursor" ? "/.cursor/projects" : "/.claude/projects")
    }

    /// The log file for a session: an absolute `.jsonl` path is used as is (any agent); otherwise it is looked up by id.
    public static func locate(agent: String, sessionId: String, dir: String) -> URL? {
        if sessionId.hasPrefix("/") {
            return sessionId.hasSuffix(".jsonl") && !sessionId.contains("/../") && FileManager.default.fileExists(atPath: sessionId)
                ? URL(fileURLWithPath: sessionId) : nil
        }
        switch agent {
        case "codex": return locateCodex(sessionId: sessionId, sessionsDir: dir)
        case "cursor": return locateCursor(sessionId: sessionId, projectsDir: dir)
        case "grok": return locateGrok(sessionId: sessionId, sessionsDir: dir)
        default: return locate(sessionId: sessionId, projectsDir: dir)   // Claude Code (and, as before, any other id)
        }
    }

    /// Shared between the tailer task and `loadEarlier`. Holds only byte offsets, not the lines: a re-parse reads
    /// `[start, consumed)` from the file again, so memory does not grow with the raw log.
    private final class State: @unchecked Sendable {
        let lock = NSLock()
        var url: URL?
        var mirror: RemoteLogMirror?  // remote logs: `url` is its file
        var start: UInt64 = 0        // file offset of the first loaded line
        var consumed: UInt64 = 0     // file offset after the last whole line the tailer parsed
        var loaded = false
        var dirty = false            // the range changed; the tailer must re-parse it
        var wantFull = true          // backfill to the full tail after the short first view
        var wake: (@Sendable () -> Void)?
    }
    private let state = State()

    /// Loads up to `bytes` more from before the current start, aligned to whole lines, and puts the parsed items
    /// in front of the stream (the next `items`/`snapshots` emission, within one poll). Tool results that sit in a
    /// later segment still match their calls, because the whole loaded range is parsed again together.
    /// Distinguishes more content, the beginning of the log, and a failed read (including a disconnected host).
    @discardableResult
    public func loadEarlier(bytes: Int = 2_000_000) async -> EarlierLoadResult {
        let (url, start, mirror): (URL?, UInt64, RemoteLogMirror?) = state.lock.withLock { (state.loaded ? state.url : nil, state.start, state.mirror) }
        guard let url, start > 0 else { return .end }
        guard let fh = try? FileHandle(forReadingFrom: url) else { return .failed }
        defer { try? fh.close() }
        var span = UInt64(max(bytes, 1))
        while true {
            let from = start > span ? start - span : 0
            // Fetch before reading the sparse mirror: a failed host read cannot advance the loaded range.
            if let mirror, !(await mirror.ensure(from: from > 0 ? from - 1 : 0, to: start)) { return .failed }
            // one byte before `from` tells whether `from` is already a line start
            do { try fh.seek(toOffset: from > 0 ? from - 1 : 0) } catch { return .failed }
            let count = Int(start - from) + (from > 0 ? 1 : 0)
            guard var data = try? fh.read(upToCount: count), data.count == count else { return .failed }
            var newStart = from
            if from > 0 {
                let prev = data.removeFirst()
                if prev != 0x0A {
                    guard let nl = data.firstIndex(of: 0x0A) else {          // one line longer than the span: widen
                        if from == 0 { return .failed }
                        span *= 2; continue
                    }
                    newStart = from + UInt64(data.distance(from: data.startIndex, to: nl)) + 1
                    data = Data(data[(nl + 1)...])
                }
            }
            if newStart == start && from > 0 { span *= 2; continue }          // nothing whole in this span: widen
            let more: Bool = state.lock.withLock {
                guard state.start == start else { return state.start > 0 }    // another load or a reset won
                state.start = newStart
                state.dirty = true
                return newStart > 0
            }
            state.lock.withLock { state.wake }?()
            return more ? .more : .end
        }
    }

    /// Full mode keeps up to 2 MB of tail loaded; short mode keeps about 256 KB. Switching up loads earlier content in the
    /// background (items grow at the front); switching down drops the front items. Call before `snapshots` to choose the
    /// mode of the first load. New logs start in full mode.
    public func setFull(_ full: Bool) {
        let (url, start, consumed, loaded) = state.lock.withLock { () -> (URL?, UInt64, UInt64, Bool) in
            state.wantFull = full
            return (state.url, state.start, state.consumed, state.loaded)
        }
        guard loaded, let url else { return }
        let short = Self.shortTail, full2 = Self.fullTail
        if full {
            let have = consumed - start
            if start > 0, have < full2 { Task.detached { [self] in _ = await loadEarlier(bytes: Int(full2 - have)) } }
        } else if consumed - start > short + short / 2, let fh = try? FileHandle(forReadingFrom: url) {
            defer { try? fh.close() }
            var cur = consumed - short - 1                   // offset of the byte read next
            try? fh.seek(toOffset: cur)
            var prev: UInt8? = nil
            var found: UInt64?
            outer: while cur < consumed {
                guard let chunk = try? fh.read(upToCount: 65536), !chunk.isEmpty else { return }
                for b in chunk {
                    if prev == 0x0A { found = cur; break outer }
                    prev = b; cur += 1
                }
            }
            guard let pos = found, pos > start, pos < consumed else { return }
            let newStart = pos
            state.lock.withLock {
                guard state.start == start else { return }
                state.start = newStart; state.dirty = true
            }
            state.lock.withLock { state.wake }?()
        }
    }
    static let shortTail: UInt64 = 256 * 1024
    static let fullTail: UInt64 = 2 * 1024 * 1024

    /// Pure parse for tests.
    public static func parse(lines: [String]) -> [ConversationItem] {
        var p = ConversationParser()
        for l in lines { p.feed(line: l) }
        return p.items()
    }

    /// Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`. Accepts a session id or the file path itself.
    public static func locateCodex(sessionId: String, sessionsDir: String = NSHomeDirectory() + "/.codex/sessions") -> URL? {
        let fm = FileManager.default
        if sessionId.hasPrefix("/") { return fm.fileExists(atPath: sessionId) && sessionId.hasSuffix(".jsonl") ? URL(fileURLWithPath: sessionId) : nil }
        guard !sessionId.isEmpty, !sessionId.contains("/"), !sessionId.contains("..") else { return nil }
        // Newest day first: a live session is almost always in the last few day folders, so this stops early.
        func sub(_ p: String) -> [String] { ((try? fm.contentsOfDirectory(atPath: p)) ?? []).sorted(by: >) }
        let suffix = "-\(sessionId).jsonl"
        for y in sub(sessionsDir) { for m in sub("\(sessionsDir)/\(y)") { for d in sub("\(sessionsDir)/\(y)/\(m)") {
            let dir = "\(sessionsDir)/\(y)/\(m)/\(d)"
            if let f = sub(dir).first(where: { $0.hasPrefix("rollout-") && $0.hasSuffix(suffix) }) { return URL(fileURLWithPath: "\(dir)/\(f)") }
        } } }
        return nil
    }

    /// Cursor: `~/.cursor/projects/*/agent-transcripts/<id>/<id>.jsonl`.
    public static func locateCursor(sessionId: String, projectsDir: String = NSHomeDirectory() + "/.cursor/projects") -> URL? {
        guard !sessionId.isEmpty, !sessionId.contains("/"), !sessionId.contains("..") else { return nil }
        let fm = FileManager.default
        for d in (try? fm.contentsOfDirectory(atPath: projectsDir)) ?? [] {
            let p = "\(projectsDir)/\(d)/agent-transcripts/\(sessionId)/\(sessionId).jsonl"
            if fm.fileExists(atPath: p) { return URL(fileURLWithPath: p) }
        }
        return nil
    }

    /// Grok Build: `<sessions>/<url-encoded cwd, or a slug for a long one>/<id>/updates.jsonl`. Not `chat_history.jsonl`,
    /// which compaction rewrites.
    public static func locateGrok(sessionId: String, sessionsDir: String) -> URL? {
        guard !sessionId.isEmpty, !sessionId.contains("/"), !sessionId.contains("..") else { return nil }
        let fm = FileManager.default
        for d in (try? fm.contentsOfDirectory(atPath: sessionsDir)) ?? [] {
            let p = "\(sessionsDir)/\(d)/\(sessionId)/updates.jsonl"
            if fm.fileExists(atPath: p) { return URL(fileURLWithPath: p) }
        }
        return nil
    }

    /// `~/.claude/projects/*/<id>.jsonl`
    /// A Claude Code log, followed to the newest log of its conversation (see `continued(_:projectsDir:)`).
    public static func locate(sessionId: String, projectsDir: String = NSHomeDirectory() + "/.claude/projects") -> URL? {
        locateOne(sessionId: sessionId, projectsDir: projectsDir).map { continued($0, projectsDir: projectsDir) }
    }

    private static func locateOne(sessionId: String, projectsDir: String, near: URL? = nil) -> URL? {
        // reject path tricks in the id
        guard !sessionId.isEmpty, !sessionId.contains("/"), !sessionId.contains("..") else { return nil }
        let fm = FileManager.default
        if let near {
            let p = near.deletingLastPathComponent().appendingPathComponent(sessionId + ".jsonl")
            if fm.fileExists(atPath: p.path) { return p }
        }
        guard let dirs = try? fm.contentsOfDirectory(atPath: projectsDir) else { return nil }
        for d in dirs {
            let p = "\(projectsDir)/\(d)/\(sessionId).jsonl"
            if fm.fileExists(atPath: p) { return URL(fileURLWithPath: p) }
        }
        return nil
    }

    /// Claude Code ends a log with a `continued-in` line when the conversation goes on in another session's log, while
    /// herdr keeps the old id. The newest log of the chain; `log` itself when its last line is not one, or the next log
    /// is not there.
    public static func continued(_ log: URL, projectsDir: String = NSHomeDirectory() + "/.claude/projects") -> URL {
        var at = log, seen: Set<String> = [log.path]
        while seen.count <= 16, let id = continuedIn(at), let next = locateOne(sessionId: id, projectsDir: projectsDir, near: at),
              seen.insert(next.path).inserted {
            at = next
        }
        return at
    }

    /// The `continuedInSessionId` of a log whose last line is a `continued-in` line, else nil.
    public static func continuedIn(_ log: URL) -> String? {
        guard let fh = try? FileHandle(forReadingFrom: log) else { return nil }
        defer { try? fh.close() }
        let size = (try? fh.seekToEnd()) ?? 0
        guard size > 0 else { return nil }
        try? fh.seek(toOffset: size > 4096 ? size - 4096 : 0)
        let tail = (try? fh.readToEnd()) ?? Data()
        guard let last = tail.split(separator: 0x0A).last,
              let o = try? JSONSerialization.jsonObject(with: Data(last)) as? [String: Any],
              o["type"] as? String == "continued-in" else { return nil }
        return o["continuedInSessionId"] as? String
    }

    public struct Snapshot: Sendable {
        /// Conversation as logged. May end with a `pending` permissionRequest.
        public var items: [ConversationItem]
        /// A tool call has no result yet (running or waiting for approval).
        public var hasOpenToolUse: Bool = false
        /// Where the first read stands. `.ready` with no items means the log exists and has nothing to show yet.
        public var load: ConversationLoadState = .ready
        /// Claude Code's queue as the log has it (`ClaudeQueueLog`), oldest first.
        public var queued = ClaudeQueueLog()
    }

    /// Full item list, re-yielded whenever the file grows. Yields `[]` first.
    public func items(projectsDir: String? = nil,
                      pollInterval: Duration = .seconds(10)) -> AsyncStream<[ConversationItem]> {
        let inner = snapshots(projectsDir: projectsDir, pollInterval: pollInterval)
        return AsyncStream { cont in
            let t = Task { for await s in inner { cont.yield(s.items) }; cont.finish() }
            cont.onTermination = { _ in t.cancel() }
        }
    }

    /// Wakes the tailer. A `DispatchSource` on the file (write, extend, rename, delete) yields at once; a timer is the
    /// safety net (1 s while the file is not located, `heartbeat` after).
    private final class Watch: @unchecked Sendable {
        private let lock = NSLock()
        private var source: DispatchSourceFileSystemObject?
        private let timer = DispatchSource.makeTimerSource(queue: .global(qos: .utility))
        private var lost = false
        private let heartbeat: TimeInterval
        let stream: AsyncStream<Void>
        private let cont: AsyncStream<Void>.Continuation

        init(heartbeat: Duration) {
            let c = heartbeat.components
            self.heartbeat = max(0.01, Double(c.seconds) + Double(c.attoseconds) / 1e18)
            (stream, cont) = AsyncStream.makeStream(of: Void.self, bufferingPolicy: .bufferingNewest(1))
            timer.setEventHandler { [cont] in cont.yield() }
            schedule(min(1, self.heartbeat))
            timer.resume()
        }
        private func schedule(_ s: TimeInterval) { timer.schedule(deadline: .now() + s, repeating: s) }
        var armed: Bool { lock.withLock { source != nil } }

        func arm(path: String) {
            disarm()
            let fd = open(path, O_EVTONLY)
            guard fd >= 0 else { return }
            let src = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: [.write, .extend, .rename, .delete],
                                                                queue: .global(qos: .utility))
            src.setEventHandler { [weak self, weak src] in
                guard let self, let src else { return }
                if !src.data.isDisjoint(with: [.rename, .delete]) { self.lock.withLock { self.lost = true } }
                self.cont.yield()
            }
            src.setCancelHandler { close(fd) }
            lock.withLock { source = src; lost = false }
            src.resume()
            schedule(heartbeat)
        }
        func disarm() {
            let old = lock.withLock { () -> DispatchSourceFileSystemObject? in defer { source = nil }; return source }
            old?.cancel()
            schedule(min(1, heartbeat))
        }
        /// The file was renamed or deleted since the last call.
        /// While the file is not located: 1 s at first, 5 s after `slow()` (a missing log is looked up rarely).
        func slow() { if !armed { schedule(max(5, heartbeat)) } }
        /// Back to the 1 s timer after `slow()`, for a file that is found but never armed (a remote log's mirror).
        func fast() { if !armed { schedule(min(1, heartbeat)) } }
        func takeLost() -> Bool { lock.withLock { defer { lost = false }; return lost } }
        func poke() { cont.yield() }
        func stop() { disarm(); timer.cancel(); cont.finish() }
    }

    /// The first load reads only the last `initialTailBytes` (default 256 KB, aligned to a line) and yields it at once;
    /// then it backfills up to 2 MB in the background (items grow at the front, ids stay the same). Passing
    /// `initialTailBytes` explicitly turns the backfill off. Later growth is read when the file watcher fires.
    /// `pollInterval` is only the safety-net timer.
    public func snapshots(projectsDir: String? = nil,
                          pollInterval: Duration = .seconds(10),
                          initialTailBytes: UInt64? = nil) -> AsyncStream<Snapshot> {
        let id = sessionId, agent = self.agent, state = self.state, remote = self.remote
        let first = initialTailBytes ?? Self.shortTail
        let total = initialTailBytes ?? Self.fullTail
        let dir = projectsDir ?? Self.defaultDir(agent: agent)
        let mirrorTail = max(first, total) + RemoteLogMirror.slack
        return AsyncStream { cont in
            let watch = Watch(heartbeat: pollInterval)
            state.lock.withLock { state.wake = { [weak watch] in watch?.poke() } }
            let task = Task.detached { [self] in
                var parser = ConversationParser()
                var offset: UInt64 = 0
                var leftover = Data()
                var leftoverPos: UInt64 = 0     // file offset of leftover's first byte
                var started = false
                var skipPartial = false
                var backfill = false
                var url: URL?
                var mirror: RemoteLogMirror?
                var gone = 0   // syncs in a row where the host said the file is not there
                var mirrorFilled = false   // the mirror holds the tail: until then its file is not read
                func dropMirror() {
                    mirror?.remove(); mirror = nil; url = nil; mirrorFilled = false
                    state.lock.withLock { state.mirror = nil; state.url = nil }
                }
                func reset() {
                    offset = 0; leftover = Data(); leftoverPos = 0; parser = ConversationParser(); started = false
                    skipPartial = false; backfill = false
                    state.lock.withLock { state.start = 0; state.consumed = 0; state.loaded = false; state.dirty = false }
                }
                reset()
                cont.yield(Snapshot(items: [], load: .loading))
                var reported: ConversationLoadState = .loading
                var misses = 0
                func report(_ l: ConversationLoadState) {   // state-only change (no new items)
                    guard l != reported else { return }
                    reported = l; cont.yield(Snapshot(items: parser.items(), hasOpenToolUse: parser.hasOpenToolUse, load: l, queued: parser.queue))
                }
                var wakes = watch.stream.makeAsyncIterator()
                while !Task.isCancelled {
                    if watch.takeLost() { watch.disarm(); url = nil; reset() }
                    if let host = remote {
                        // No file events over ssh: the timer wakes this loop (1 s, or `pollInterval` if shorter) and the
                        // mirror fetches what the host file grew by. The watch is never armed on the mirror.
                        if mirror == nil, let p = await RemoteLogMirror.locate(agent: agent, sessionId: id, host: host, dir: projectsDir),
                           let m = try? RemoteLogMirror(host: host, path: p) {
                            mirror = m; url = m.url; gone = 0
                            state.lock.withLock { state.mirror = m }
                        }
                        if let m = mirror {
                            switch await m.sync(tail: mirrorTail) {
                            case .reset: reset(); gone = 0
                            case .gone:   // deleted or moved on the host: look it up again
                                gone += 1
                                if gone >= 5 { dropMirror(); reset() }
                            case .unreachable: break   // keep what the mirror has (ssh down, sleep): the view stays
                            default: gone = 0
                            }
                            mirrorFilled = await m.isFilled
                        }
                    } else if url == nil { url = SessionLog.locate(agent: agent, sessionId: id, dir: dir) }
                    if let url { state.lock.withLock { state.url = url } }
                    if let url, remote == nil, !watch.armed { watch.arm(path: url.path) }
                    if url == nil {
                        report(.missing)
                        misses += 1
                        if misses == 10 { watch.slow() }
                    } else {
                        if misses >= 10, remote != nil { watch.fast() }
                        misses = 0
                    }
                    if let url, (try? FileHandle(forReadingFrom: url)) == nil { report(.failed("cannot read \(url.lastPathComponent)")) }
                    var moveTo: URL?
                    if let url, remote == nil || mirrorFilled, let fh = try? FileHandle(forReadingFrom: url) {
                        defer { try? fh.close() }
                        let size = (try? fh.seekToEnd()) ?? 0
                        if size < offset { reset() }   // truncated
                        if !started {
                            started = true
                            if size > first { offset = size - first; skipPartial = true }
                            leftoverPos = offset
                            if !skipPartial { state.lock.withLock { state.start = 0; state.loaded = true } }
                        }
                        if size > offset {
                            try? fh.seek(toOffset: offset)
                            let data = (try? fh.readToEnd()) ?? Data()
                            offset += UInt64(data.count)
                            leftover.append(data)
                            if skipPartial {   // we landed mid-line: drop up to the first newline
                                if let nl = leftover.firstIndex(of: 0x0A) {
                                    let n = leftover.distance(from: leftover.startIndex, to: nl) + 1
                                    leftover.removeSubrange(leftover.startIndex...nl); leftoverPos += UInt64(n); skipPartial = false
                                    let p = leftoverPos
                                    state.lock.withLock { state.start = p; state.consumed = p; state.loaded = true }
                                    backfill = total > first && p > 0 && state.lock.withLock { state.wantFull }
                                } else { leftover = Data(); leftoverPos = offset }
                            }
                            while let nl = leftover.firstIndex(of: 0x0A) {
                                let n = leftover.distance(from: leftover.startIndex, to: nl) + 1
                                if let line = String(data: leftover[leftover.startIndex..<nl], encoding: .utf8) {
                                    parser.feed(line: line, offset: leftoverPos)
                                }
                                leftover.removeSubrange(leftover.startIndex...nl)
                                leftoverPos += UInt64(n)
                            }
                            let consumed = leftoverPos
                            state.lock.withLock { state.consumed = consumed }
                            reported = .ready
                            cont.yield(Snapshot(items: parser.items(), hasOpenToolUse: parser.hasOpenToolUse, queued: parser.queue))
                            // The conversation went on in a new log (`continued-in`): tail that one from the start.
                            if remote == nil, parser.continuedIn != nil {
                                let next = SessionLog.continued(url, projectsDir: dir)
                                if next != url { moveTo = next }
                            }
                        }
                        report(.ready)   // located and read, even when the file is still empty
                        if backfill {   // the short first view is out; fill in the rest of the tail now
                            backfill = false
                            _ = await self.loadEarlier(bytes: Int(total - first))
                        }
                        if let (from, to) = state.lock.withLock({ () -> (UInt64, UInt64)? in
                            guard state.dirty else { return nil }
                            state.dirty = false
                            return (state.start, state.consumed)
                        }) {
                            parser = ConversationParser()
                            try? fh.seek(toOffset: from)
                            let data = (try? fh.read(upToCount: Int(to - from))) ?? Data()
                            var pos = from
                            for raw in data.split(separator: 0x0A, omittingEmptySubsequences: false).dropLast() {
                                if let t = String(data: raw, encoding: .utf8) { parser.feed(line: t, offset: pos) }
                                pos += UInt64(raw.count) + 1
                            }
                            cont.yield(Snapshot(items: parser.items(), hasOpenToolUse: parser.hasOpenToolUse, queued: parser.queue))
                        }
                    }
                    if let moveTo { watch.disarm(); url = moveTo; reset(); continue }
                    if await wakes.next() == nil { break }
                }
                if mirror != nil { dropMirror() }
                cont.finish()
            }
            cont.onTermination = { _ in task.cancel(); watch.stop() }
        }
    }
}

/// Load state of one conversation log, for the UI's empty and loading states.
public enum ConversationLoadState: Sendable, Equatable {
    /// The first read has not finished (or tailing has not started).
    case loading
    /// The log was read. Items may still be empty (a new session that has not written anything).
    case ready
    /// No log file for this session id (yet). The tailer keeps looking once a second and turns `.ready` when it appears.
    case missing
    /// The file exists but cannot be opened.
    case failed(String)
}
