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
        return names.map { n in
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

    /// Made before the key. False: this prompt (its id, or its tool and command) is claimed already.
    public func claim(terminal: String, id: String, tool: String, command: String, blocked: Bool, now: Date = Date()) -> Bool {
        guard let folder = folder(terminal), let url = file(terminal, id) else { return true }
        if isClaimed(terminal: terminal, id: id, tool: tool, command: command, blocked: blocked, now: now) { return false }
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try? FileManager.default.removeItem(at: url)   // not live (isClaimed above): its time is over
        let fd = open(url.path, O_WRONLY | O_CREAT | O_EXCL, 0o600)
        guard fd >= 0 else { return errno != EEXIST }  // EEXIST: another run claimed it just now; else as without a cache
        let data = (try? JSONEncoder().encode(Mark(id: id, tool: tool, command: Self.flat(command), held: false))) ?? Data()
        _ = data.withUnsafeBytes { write(fd, $0.baseAddress, $0.count) }
        close(fd)
        return true
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
        for (url, m) in marks(terminal) {
            if let box, let m, m.tool == box.tool, m.command == Self.flat(box.command) { continue }
            try? FileManager.default.removeItem(at: url)
        }
    }
}
