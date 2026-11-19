import CryptoKit
import Foundation

/// A local copy of an agent log that lives on a `RemoteHost`. Every byte sits at the same offset as on the host: the file
/// is sparse, and ranges not fetched yet are holes. So the byte offsets in items and `LogImage.Ref`s mean the same in
/// both files, and `SessionLog`'s parser, `LogImage.read`, LogPictures and the phone read the mirror like a local log.
///
/// The fetched bytes are one range `[low, end)`. `sync` fetches the tail first, then what the host file grew by (there
/// are no file events over ssh, so `SessionLog` calls it about once a second while the log is watched). `ensure` fetches
/// an earlier range before `loadEarlier` reads it. A host file that shrinks (truncated or replaced) resets the mirror.
///
/// Mirrors live in `directory` (mode 700), one file per watched log (mode 600). A mirror is deleted when its log is no
/// longer watched (`remove`, or when it is freed); files older than a day, and the oldest ones over `maxTotal`, are
/// deleted when the next mirror is made, but not the ones another app run has open (each live mirror holds a shared
/// `flock`).
public actor RemoteLogMirror {
    /// The local mirror file.
    public nonisolated let url: URL
    /// The log's path on the host.
    public nonisolated let path: String
    public nonisolated let host: RemoteHost
    private let files: RemoteFiles
    private let fd: Int32
    private var low: UInt64 = 0
    private var end: UInt64 = 0
    private var filled = false
    private var queue: Task<Void, Never>?

    /// The folder for mirror files. Checks point it at a temp folder.
    nonisolated(unsafe) public static var directory: URL = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("HerdrDesk/remote-logs", isDirectory: true)
    /// Bytes before the mirrored end compared with the host's on each sync (a replaced file resets the mirror).
    static let seam: UInt64 = 64
    /// Fetched before the first view's tail, so `loadEarlier` can align it to a line start without a round trip.
    static let slack: UInt64 = 64 * 1024
    /// Largest read in one host command.
    static let chunk = 8 << 20
    /// Mirror files of other runs older than this are deleted.
    static let maxAge: TimeInterval = 86_400
    /// The cache folder's cap. Live mirrors count but are never deleted for it.
    static let maxTotal: UInt64 = 512 << 20
    /// Names of the mirror files this process has open.
    private static let live = Locked<Set<String>>([])

    public init(host: RemoteHost, path: String, dir: URL = RemoteLogMirror.directory) throws {
        try Self.prepare(dir)
        Self.sweep(dir: dir)
        var key = host.label + "\u{0}" + path
        let digest = key.withUTF8 { SHA256.hash(data: UnsafeRawBufferPointer($0)) }
        let name = digest.prefix(8).map { String(format: "%02x", $0) }.joined() + "-"
            + UUID().uuidString.prefix(8).lowercased() + ".jsonl"
        let url = dir.appendingPathComponent(name)
        let fd = open(url.path, O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard fd >= 0 else { throw HerdrError.socket("cannot make \(url.path): \(String(cString: strerror(errno)))") }
        fchmod(fd, 0o600)
        flock(fd, LOCK_SH)   // another app run's sweep leaves it alone
        self.url = url; self.path = path; self.host = host; self.fd = fd
        self.files = RemoteFiles(host: host)
        Self.live.update { _ = $0.insert(name) }
        MirrorSyncs.register(url.path)
    }

    deinit {
        close(fd)
        unlink(url.path)
        MirrorSyncs.drop(url.path)
        let name = url.lastPathComponent
        Self.live.update { _ = $0.remove(name) }
    }

    /// Deletes the mirror file. Reads of `url` fail afterwards; the mirror itself must not be used again.
    public nonisolated func remove() {
        unlink(url.path)
        let name = url.lastPathComponent
        Self.live.update { _ = $0.remove(name) }
    }

    /// The tail is in: the mirror file can be read. False before the first fetch and after a reset whose fetch failed.
    public var isFilled: Bool { filled }

    public enum Change: Sendable, Equatable {
        /// The host did not answer (ssh down, timeout). The mirror is as it was.
        case unreachable
        /// The host answered: no such file (deleted or moved away). The mirror is as it was.
        case gone
        case unchanged
        /// New bytes at the end (or the first tail).
        case grew
        /// The host file shrank: the mirror started over with its tail. Offsets read before mean nothing now.
        case reset
    }

    /// Brings the mirror up to the host file's size. The first call fetches the last `tail` bytes (holes before them).
    public func sync(tail: UInt64) async -> Change {
        let c = await serial { await self.doSync(tail: tail) }
        if c == .grew || c == .unchanged || c == .reset { MirrorSyncs.bump(url.path) }   // the host answered: caught up
        return c
    }

    /// Makes sure `[from, to)` is fetched. False when the host did not give it all, or `to` is past the mirrored end.
    public func ensure(from: UInt64, to: UInt64) async -> Bool {
        await serial { await self.doEnsure(from: from, to: to) }
    }

    /// Runs host fetches one at a time (an actor lets another call in at each `await`).
    private func serial<T: Sendable>(_ op: @escaping @Sendable () async -> T) async -> T {
        let prev = queue
        let t = Task { await prev?.value; return await op() }
        queue = Task { _ = await t.value }
        return await t.value
    }

    private func doSync(tail: UInt64) async -> Change {
        let size: UInt64
        // the last bytes already mirrored, read again on the host: a file replaced by one that is not smaller differs
        let seam: Range<UInt64>? = filled && end > 0 ? (end - min(Self.seam, end))..<end : nil
        var hostSeam: Data?
        switch await hostSize(seam: seam) {
        case .some(.some(let s)): size = s.size; hostSeam = s.seam
        case .some(.none): return .gone
        case .none: return .unreachable
        }
        var reset = false
        if filled && (size < end || (seam != nil && hostSeam != mirrored(seam!))) {
            ftruncate(fd, 0); low = 0; end = 0; filled = false; reset = true
        }
        if !filled {
            let from = size > tail ? size - tail : 0
            let got = await fetch(from, size)
            if got == from && size > from { return reset ? .reset : .unreachable }
            low = from; end = got; filled = true
            return reset ? .reset : .grew
        }
        if size == end { return .unchanged }
        let got = await fetch(end, size)
        if got == end { return .unreachable }
        end = got
        return .grew
    }

    private func doEnsure(from: UInt64, to: UInt64) async -> Bool {
        guard filled, to <= end else { return false }
        if from >= low { return true }
        let target = low
        guard await fetch(from, target) == target else { return false }
        low = from
        return true
    }

    /// The host file's size, and its bytes in `seam` (hex from `od`, so one round trip): nil when the host did not
    /// answer, `.some(nil)` when it answered that the file is not there.
    private func hostSize(seam: Range<UInt64>?) async -> (size: UInt64, seam: Data?)?? {
        let q = RemoteHost.quote(path)
        // exit 3: no such file (ssh's own failures are 255, a timeout -1)
        var cmd = "[ -f \(q) ] || exit 3; wc -c < \(q)"
        if let seam { cmd += " && tail -c +\(seam.lowerBound + 1) \(q) | head -c \(seam.count) | od -An -v -tx1" }
        guard let r = try? await host.run(cmd, timeout: files.timeout) else { return nil }
        if r.status == 3 { return .some(nil) }
        let lines = String(decoding: r.stdout, as: UTF8.self).split(separator: "\n", maxSplits: 1)
        guard r.ok, let first = lines.first, let n = UInt64(first.trimmingCharacters(in: .whitespaces)) else { return nil }
        guard seam != nil else { return .some((n, nil)) }
        let hex = lines.count > 1 ? lines[1].split(whereSeparator: { $0 == " " || $0 == "\n" }) : []
        return .some((n, Data(hex.compactMap { UInt8($0, radix: 16) })))
    }

    /// The mirror's own bytes in `r` (already fetched).
    private func mirrored(_ r: Range<UInt64>) -> Data {
        var d = Data(count: r.count)
        let n = d.withUnsafeMutableBytes { pread(fd, $0.baseAddress, r.count, off_t(r.lowerBound)) }
        return n == r.count ? d : Data()
    }

    /// Fetches `[from, to)` into the mirror at the same offsets; returns where the contiguous fetched bytes end.
    private func fetch(_ from: UInt64, _ to: UInt64) async -> UInt64 {
        var at = from
        while at < to {
            let n = Int(min(UInt64(Self.chunk), to - at))
            guard let d = await files.read(path, offset: at, count: n), !d.isEmpty, write(d, at: at) else { break }
            at += UInt64(d.count)
            if d.count < n { break }
        }
        return at
    }

    private func write(_ d: Data, at offset: UInt64) -> Bool {
        d.withUnsafeBytes { buf in
            guard let base = buf.baseAddress else { return true }
            var done = 0
            while done < buf.count {
                let n = pwrite(fd, base + done, buf.count - done, off_t(offset) + off_t(done))
                if n < 0 && errno == EINTR { continue }
                if n <= 0 { return false }
                done += n
            }
            return true
        }
    }

    // MARK: - Cache folder

    /// Makes `dir` (mode 700) and refuses one that is not a folder of this user.
    static func prepare(_ dir: URL) throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        var st = stat()
        guard lstat(dir.path, &st) == 0, (st.st_mode & S_IFMT) == S_IFDIR, st.st_uid == getuid() else {
            throw HerdrError.socket("\(dir.path) is not a folder of this user")
        }
        if st.st_mode & 0o777 != 0o700 { chmod(dir.path, 0o700) }
    }

    /// Deletes mirror files of other runs older than `maxAge`, then the oldest ones while the folder is over `maxTotal`.
    /// The mirrors this process has open stay.
    static func sweep(dir: URL, maxAge: TimeInterval = RemoteLogMirror.maxAge, maxTotal: UInt64 = RemoteLogMirror.maxTotal, now: Date = Date()) {
        let fm = FileManager.default
        let keys: [URLResourceKey] = [.contentModificationDateKey, .totalFileAllocatedSizeKey, .fileSizeKey, .isRegularFileKey]
        guard let urls = try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: keys) else { return }
        let live = Self.live.get()
        var total: UInt64 = 0
        var old: [(url: URL, date: Date, size: UInt64)] = []
        for u in urls where u.pathExtension == "jsonl" {
            guard let v = try? u.resourceValues(forKeys: Set(keys)), v.isRegularFile == true else { continue }
            let size = UInt64(max(0, v.totalFileAllocatedSize ?? v.fileSize ?? 0))
            let date = v.contentModificationDate ?? .distantPast
            if live.contains(u.lastPathComponent) { total += size; continue }
            if now.timeIntervalSince(date) > maxAge, removeUnlocked(u) { continue }
            total += size
            old.append((u, date, size))
        }
        for f in old.sorted(by: { $0.date < $1.date }) where total > maxTotal {
            if removeUnlocked(f.url) { total -= min(total, f.size) }
        }
    }

    /// Deletes a mirror file unless another app run has it open (it holds a shared lock). True: deleted.
    static func removeUnlocked(_ u: URL) -> Bool {
        let f = open(u.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK)
        guard f >= 0 else { return false }
        defer { close(f) }
        guard flock(f, LOCK_EX | LOCK_NB) == 0 else { return false }
        return unlink(u.path) == 0
    }

    // MARK: - Finding the log on the host

    /// The log's path on the host, by the same rules as `SessionLog.locate`: an absolute `.jsonl` path without "/../"
    /// is used as is; otherwise the id is looked up under `dir` (nil: the agent's folder in the host user's home).
    public static func locate(agent: String, sessionId: String, host: RemoteHost, dir: String?) async -> String? {
        guard let cmd = locateCommand(agent: agent, sessionId: sessionId, dir: dir),
              let r = try? await host.run(cmd, timeout: .seconds(30)), r.ok else { return nil }
        let p = String(decoding: r.stdout, as: UTF8.self)
        return p.hasPrefix("/") && p.hasSuffix(".jsonl") && !p.contains("\n") && !p.contains("/../") ? p : nil
    }

    /// The `sh` command line that prints the log's host path (exit 1: none). Nil for an id `SessionLog.locate` refuses.
    /// Ids are quoted, so a glob character in one matches only itself.
    static func locateCommand(agent: String, sessionId id: String, dir: String?) -> String? {
        let q = RemoteHost.quote
        if id.hasPrefix("/") {
            guard id.hasSuffix(".jsonl"), !id.contains("/../") else { return nil }
            return "[ -f \(q(id)) ] && printf '%s' \(q(id))"
        }
        guard !id.isEmpty, !id.contains("/"), !id.contains("..") else { return nil }
        let rel = agent == "codex" ? ".codex/sessions" : agent == "cursor" ? ".cursor/projects" : ".claude/projects"
        let d = dir.map(q) ?? (agent == "grok" ? "\"${GROK_HOME:-$HOME/.grok}\"/sessions" : "\"$HOME\"/\(rel)")
        let glob: String
        var newest = false
        switch agent {
        case "codex":   // sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl; the last match in glob order is the newest day
            glob = "\(d)/*/*/*/rollout-*\(q("-\(id).jsonl"))"; newest = true
        case "cursor":  // projects/*/agent-transcripts/<id>/<id>.jsonl
            glob = "\(d)/*/agent-transcripts/\(q(id))/\(q(id + ".jsonl"))"
        case "grok":    // sessions/<cwd folder>/<id>/updates.jsonl (not chat_history.jsonl, which compaction rewrites)
            glob = "\(d)/*/\(q(id))/updates.jsonl"
        default:        // Claude Code: projects/*/<id>.jsonl
            glob = "\(d)/*/\(q(id + ".jsonl"))"
        }
        let take = newest ? "r=$f" : "{ r=$f; break; }"
        return "r=; for f in \(glob); do [ -f \"$f\" ] && \(take); done; [ -n \"$r\" ] && printf '%s' \"$r\""
    }
}
