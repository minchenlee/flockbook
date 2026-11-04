import Foundation

/// The approvals this sidecar answered, kept next to the cache file (`answered/`), as the app's `ApprovalSender` keeps
/// them in memory: one answer per prompt. Each sidecar run starts from nothing, and two views (or two windows) run
/// their own sidecars, so the claim is a file made with an exclusive create before the key goes out.
///
/// A claim names its prompt by the screen id and by the tool and command: a resized pane re-wraps the box and gives
/// it another screen id, but it is the same prompt. A claim holds up to `heldWait` (about hc's send timeout); a held
/// key (`hold`, hc's typing-quiet rule) holds it as long as the agent stays blocked. It goes only on positive
/// evidence that its prompt left: a screen read that shows no approval box, or a box with another command (`seen`).
public struct FeedAnswered: Sendable {
    public static let heldWait: TimeInterval = 60
    let dir: URL?

    struct Mark: Codable { var id: String; var tool: String; var command: String; var held: Bool }

    /// No cache file: nothing is kept (each press stands alone).
    public init(cache: URL?) { dir = cache?.deletingLastPathComponent().appendingPathComponent("answered", isDirectory: true) }

    private func folder(_ terminal: String) -> URL? { dir?.appendingPathComponent(fnv1a(terminal), isDirectory: true) }
    private func file(_ terminal: String, _ id: String) -> URL? { folder(terminal)?.appendingPathComponent(fnv1a(id)) }

    /// The command without its spaces and line breaks: the same whatever the box's width.
    static func flat(_ command: String) -> String { command.filter { !$0.isWhitespace } }

    private func marks(_ terminal: String) -> [(url: URL, mark: Mark?)] {
        guard let folder = folder(terminal),
              let names = try? FileManager.default.contentsOfDirectory(atPath: folder.path) else { return [] }
        return names.filter { !$0.hasPrefix(".") }.map { n in   // "." names: a claim being made, the lock
            let u = folder.appendingPathComponent(n)
            return (u, (try? Data(contentsOf: u)).flatMap { try? JSONDecoder().decode(Mark.self, from: $0) })
        }
    }

    /// Whether the claim at `url` still holds. A held key's claim is renewed while the agent is blocked.
    private func live(_ url: URL, _ mark: Mark?, blocked: Bool, now: Date) -> Bool {
        if mark?.held == true, blocked {
            try? FileManager.default.setAttributes([.modificationDate: now], ofItemAtPath: url.path)
            return true
        }
        let made = (try? FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate] as? Date) ?? now
        return now.timeIntervalSince(made) < Self.heldWait
    }

    private func same(_ m: Mark?, id: String, tool: String, command: String) -> Bool {
        guard let m else { return false }
        return m.id == id || (m.tool == tool && m.command == Self.flat(command))
    }

    /// A file that does not parse and is younger than this may be another run's claim mid-write (an older sidecar
    /// wrote it in two steps): it is not removed.
    static let unreadGrace: TimeInterval = 1
    /// A lock older than this is from a run that died mid-claim.
    static let staleLock: TimeInterval = 5

    /// An unreadable file younger than `unreadGrace`.
    private func freshUnread(_ url: URL, _ mark: Mark?, now: Date) -> Bool {
        guard mark == nil,
              let made = try? FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate] as? Date else { return false }
        return now.timeIntervalSince(made) < Self.unreadGrace
    }

    private enum Took { case held(URL), busy, unavailable }

    /// A lock file (`.lock`, exclusive create) held while one run claims. A stale one is removed and taken once more.
    private func takeLock(_ folder: URL) -> Took {
        let lock = folder.appendingPathComponent(".lock")
        for _ in 0..<2 {
            let fd = open(lock.path, O_WRONLY | O_CREAT | O_EXCL, 0o600)
            if fd >= 0 { close(fd); return .held(lock) }
            guard errno == EEXIST else { return .unavailable }
            let made = (try? FileManager.default.attributesOfItem(atPath: lock.path)[.modificationDate] as? Date) ?? Date()
            if Date().timeIntervalSince(made) < Self.staleLock { return .busy }
            try? FileManager.default.removeItem(at: lock)
        }
        return .busy
    }

    /// Made before the key. False: this prompt (its id, or its tool and command) is claimed already.
    ///
    /// Runs under a lock file in the folder: another run that presses at the same moment gets false. The mark is
    /// written to a temp file and then hard-linked to the claim name, so the claim never exists without its content.
    /// A file there that did not parse is removed only when older than `unreadGrace`.
    public func claim(terminal: String, id: String, tool: String, command: String, blocked: Bool, now: Date = Date()) -> Bool {
        guard let folder = folder(terminal), let url = file(terminal, id) else { return true }
        if isClaimed(terminal: terminal, id: id, tool: tool, command: command, blocked: blocked, now: now) { return false }
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        // A folder that takes no lock file at all (not writable): as without a cache, the same as before.
        var lock: URL?
        switch takeLock(folder) {
        case .held(let l): lock = l
        case .busy: return false
        case .unavailable: lock = nil
        }
        defer { if let lock { try? FileManager.default.removeItem(at: lock) } }
        if isClaimed(terminal: terminal, id: id, tool: tool, command: command, blocked: blocked, now: now) { return false }
        if FileManager.default.fileExists(atPath: url.path) {
            let mark = (try? Data(contentsOf: url)).flatMap { try? JSONDecoder().decode(Mark.self, from: $0) }
            if freshUnread(url, mark, now: now) { return false }
            try? FileManager.default.removeItem(at: url)   // not live (isClaimed above): its time is over
        }
        let tmp = folder.appendingPathComponent(".claim-\(getpid())-\(UInt64(Date().timeIntervalSince1970 * 1e9))")
        let data = (try? JSONEncoder().encode(Mark(id: id, tool: tool, command: Self.flat(command), held: false))) ?? Data()
        let fd = open(tmp.path, O_WRONLY | O_CREAT | O_EXCL, 0o600)
        guard fd >= 0 else { return true }   // as without a cache
        let n = data.withUnsafeBytes { write(fd, $0.baseAddress, $0.count) }
        close(fd)
        guard n == data.count else { unlink(tmp.path); return true }
        let linked = link(tmp.path, url.path)
        let err = errno
        unlink(tmp.path)
        return linked == 0 || err != EEXIST   // EEXIST: another run claimed it just now; else as without a cache
    }

    /// The key may still land (hc held it): the claim lasts while the agent stays blocked.
    public func hold(terminal: String, id: String) {
        guard let url = file(terminal, id), let data = try? Data(contentsOf: url),
              var m = try? JSONDecoder().decode(Mark.self, from: data) else { return }
        m.held = true
        try? JSONEncoder().encode(m).write(to: url, options: .atomic)
    }

    /// The key did not go out (another prompt, or an error before it): the prompt can be answered again.
    public func release(terminal: String, id: String) {
        if let url = file(terminal, id) { try? FileManager.default.removeItem(at: url) }
    }

    /// Whether this prompt (its id, or its tool and command) is claimed.
    public func isClaimed(terminal: String, id: String, tool: String, command: String, blocked: Bool, now: Date = Date()) -> Bool {
        marks(terminal).contains { same($0.mark, id: id, tool: tool, command: command) && live($0.url, $0.mark, blocked: blocked, now: now) }
    }

    /// A screen read that worked shows `box` (nil: no approval box): the claims of other prompts go, as they left the
    /// screen. Never called when the read failed: that is no evidence.
    public func seen(terminal: String, box: (tool: String, command: String)?) {
        let now = Date()
        for (url, m) in marks(terminal) {
            if freshUnread(url, m, now: now) { continue }
            if let box, let m, m.tool == box.tool, m.command == Self.flat(box.command) { continue }
            try? FileManager.default.removeItem(at: url)
        }
    }
}
