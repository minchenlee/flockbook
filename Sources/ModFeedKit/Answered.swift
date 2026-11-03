import Foundation

/// The approvals this sidecar answered, kept next to the cache file (`answered/`), as the app's `ApprovalSender` keeps
/// them in memory: one answer per prompt. Each sidecar run starts from nothing, and two views (or two windows) run
/// their own sidecars, so the claim is a file made with an exclusive create before the key goes out.
///
/// A claim holds while its prompt stays on screen, up to `heldWait` (about hc's send timeout: a key hc held for its
/// typing-quiet rule may still land). A `card` read that shows another prompt (or none) for that terminal drops it.
public struct FeedAnswered: Sendable {
    public static let heldWait: TimeInterval = 60
    let dir: URL?

    /// No cache file: nothing is kept (each press stands alone).
    public init(cache: URL?) { dir = cache?.deletingLastPathComponent().appendingPathComponent("answered", isDirectory: true) }

    private func folder(_ terminal: String) -> URL? { dir?.appendingPathComponent(fnv1a(terminal), isDirectory: true) }
    private func file(_ terminal: String, _ id: String) -> URL? { folder(terminal)?.appendingPathComponent(fnv1a(id)) }

    /// Made before the key. False: another run answered this prompt less than `heldWait` ago.
    public func claim(terminal: String, id: String, now: Date = Date()) -> Bool {
        guard let folder = folder(terminal), let url = file(terminal, id) else { return true }
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        for _ in 0..<2 {
            let fd = open(url.path, O_WRONLY | O_CREAT | O_EXCL, 0o600)
            if fd >= 0 {
                _ = id.utf8CString.withUnsafeBufferPointer { write(fd, $0.baseAddress, $0.count - 1) }
                close(fd)
                return true
            }
            guard errno == EEXIST else { return true }   // cannot keep a claim: as without a cache
            let made = (try? FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate] as? Date) ?? now
            if now.timeIntervalSince(made) < Self.heldWait { return false }
            try? FileManager.default.removeItem(at: url)  // an old claim: its key had its time
        }
        return false
    }

    /// The key did not go out (another prompt, or an error before it): the prompt can be answered again.
    public func release(terminal: String, id: String) {
        if let url = file(terminal, id) { try? FileManager.default.removeItem(at: url) }
    }

    /// Whether `id` is claimed (within `heldWait`).
    public func isClaimed(terminal: String, id: String, now: Date = Date()) -> Bool {
        guard let url = file(terminal, id),
              let made = try? FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate] as? Date else { return false }
        return now.timeIntervalSince(made) < Self.heldWait
    }

    /// A read of the terminal shows `current` (nil: no approval): every other claim of that terminal goes, as its
    /// prompt left the screen.
    public func seen(terminal: String, current: String?) {
        guard let folder = folder(terminal),
              let names = try? FileManager.default.contentsOfDirectory(atPath: folder.path) else { return }
        let keep = current.map(fnv1a)
        for n in names where n != keep { try? FileManager.default.removeItem(at: folder.appendingPathComponent(n)) }
    }
}
