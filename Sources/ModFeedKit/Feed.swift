import Foundation
import HerdrKit

/// `feed`: the rows of one agent's conversation, whole the first time and as a splice after that.
///
/// The sidecar keeps no process between calls. The mod passes back the `cursor` of the last answer; the sidecar
/// keeps the rows it printed for that cursor (their hashes) in the cache file. When the log grew, it parses the same
/// range again (so a tool result still finds its call, and the turn footer moves), compares, and prints only the rows
/// from the first one that changed: `splice {at}` and the rows after it. The mod drops its rows from `at` on and
/// appends the new ones.
public enum Feed {
    /// The range parsed again on each growth stops growing here; then the sidecar starts a new 2 MB tail (splice 0).
    public static let maxSpan: UInt64 = 8 * 1024 * 1024

    public struct State: Codable, Sendable, Equatable {
        public var path: String
        public var start: UInt64
        public var size: UInt64
        public var hashes: [String]
        public var cursor: String { "1.\(fnv1a(path)).\(start).\(size).\(fnv1a(hashes.joined(separator: ",")))" }
    }

    /// What the cursor names: the log (by hash), the range start, the size then, the rows printed.
    struct Cursor: Equatable { var pathHash: String; var start: UInt64; var size: UInt64; var rowsHash: String }
    static func parse(_ c: String?) -> Cursor? {
        guard let p = c?.split(separator: ".").map(String.init), p.count == 5, p[0] == "1",
              let s = UInt64(p[2]), let z = UInt64(p[3]) else { return nil }
        return Cursor(pathHash: p[1], start: s, size: z, rowsHash: p[4])
    }

    /// Prints the `log` line and the rows (or `unchanged`) for `url`, and returns the state to keep.
    /// `previous`: the state kept for this terminal. It counts only when `cursor` names it.
    @discardableResult
    public static func run(url: URL?, agent: String, cursor: String?, previous: State?, out: JSONLines,
                           now: Date = Date(), calendar: Calendar = .current) throws -> State? {
        guard let url else {
            out.emit(["type": "log", "agent": agent, "path": NSNull()])
            out.emit(["type": "splice", "at": 0, "count": 0])
            return nil
        }
        let path = url.path
        let size = (try? FileManager.default.attributesOfItem(atPath: path)[.size] as? NSNumber)?.uint64Value ?? 0
        let c = parse(cursor)
        let prev = previous.flatMap { p in c.map { $0.pathHash == fnv1a(p.path) && $0.start == p.start && $0.size == p.size
            && $0.rowsHash == fnv1a(p.hashes.joined(separator: ",")) } == true ? p : nil }
        if let prev, prev.path == path, prev.size == size {
            out.emit(["type": "log", "agent": agent, "path": path, "start": prev.start, "size": size, "cursor": prev.cursor])
            out.emit(["type": "unchanged"])
            return prev
        }
        // Keep the old range start while the log only grew and the range is not too big.
        let keep = prev.flatMap { p in p.path == path && size >= p.size && size - p.start <= maxSpan ? p.start : nil }
        let range = try FeedTail.read(url, from: keep)
        let rows = FeedRows.rows(FeedTail.items(range.lines), now: now, calendar: calendar)
        let enc = JSONLines()
        for r in rows { enc.emit(r) }
        let hashes = enc.lines.map(fnv1a)
        var at = 0
        if let prev, keep != nil {
            while at < min(prev.hashes.count, hashes.count), prev.hashes[at] == hashes[at] { at += 1 }
        }
        let state = State(path: path, start: range.start, size: range.size, hashes: hashes)
        out.emit(["type": "log", "agent": agent, "path": path, "start": range.start, "size": range.size,
                  "cursor": state.cursor, "model": FeedTail.model(range.lines)])
        out.emit(["type": "splice", "at": at, "count": rows.count])
        for l in enc.lines[at...] { out.raw(l) }
        return state
    }

    /// `feed --before <start>`: the rows of the 2 MB of `url` before `start`, for scrolling up past the tail.
    /// Prints `older {start, count}` and the rows. It keeps no state: the tail's cursor is not touched.
    /// `start` 0 in the answer: the log's first row is in it.
    public static func older(url: URL, before: UInt64, out: JSONLines, now: Date = Date(), calendar: Calendar = .current) throws {
        let range = try FeedTail.read(url, before: before)
        let rows = FeedRows.rows(FeedTail.items(range.lines), now: now, calendar: calendar)
        out.emit(["type": "older", "path": url.path, "start": range.start, "count": rows.count])
        for r in rows { out.emit(r) }
    }
}
