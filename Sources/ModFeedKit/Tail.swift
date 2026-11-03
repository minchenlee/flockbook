import Foundation
import HerdrKit

/// Reads the end of a session log and parses it.
public enum FeedTail {
    /// The app shows the last 2 MB first (`SessionLog.fullTail`); the mod does the same.
    public static let bytes = 2 * 1024 * 1024

    public struct Range: Sendable, Equatable {
        public var lines: [String]
        /// File offset of the first whole line read.
        public var start: UInt64
        /// File size when read. Bytes after the last newline (a line still being written) are left for the next read.
        public var size: UInt64
    }

    /// The whole lines from `from` (a line start), or else from the last `bytes` of the file, where a cut first line
    /// is dropped. A last line with no newline yet is left out.
    public static func read(_ url: URL, from: UInt64? = nil, bytes: Int = bytes) throws -> Range {
        let fh = try FileHandle(forReadingFrom: url)
        defer { try? fh.close() }
        let size = try fh.seekToEnd()
        var start = from.map { min($0, size) } ?? (size > UInt64(bytes) ? size - UInt64(bytes) : 0)
        try fh.seek(toOffset: start)
        let data = try fh.read(upToCount: Int(size - start)) ?? Data()
        var body = data[...]
        if from == nil, start > 0 {
            guard let nl = body.firstIndex(of: UInt8(ascii: "\n")) else { return Range(lines: [], start: size, size: size) }
            start += UInt64(nl - body.startIndex + 1)
            body = body[(nl + 1)...]
        }
        var parts = body.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: false)
        if !parts.isEmpty { parts.removeLast() }   // after the last "\n": empty, or a line still being written
        return Range(lines: parts.compactMap { $0.isEmpty ? nil : String(decoding: $0, as: UTF8.self) }, start: start, size: size)
    }

    /// The whole lines that end before `before` (a line start): up to `bytes` back, a cut first line dropped.
    /// `start` is where they begin; 0 means the file's start was reached.
    public static func read(_ url: URL, before: UInt64, bytes: Int = bytes) throws -> Range {
        let fh = try FileHandle(forReadingFrom: url)
        defer { try? fh.close() }
        let size = try fh.seekToEnd()
        let end = min(before, size)
        var start = end > UInt64(bytes) ? end - UInt64(bytes) : 0
        try fh.seek(toOffset: start)
        let data = try fh.read(upToCount: Int(end - start)) ?? Data()
        var body = data[...]
        if start > 0 {
            guard let nl = body.firstIndex(of: UInt8(ascii: "\n")) else { return Range(lines: [], start: end, size: size) }
            start += UInt64(nl - body.startIndex + 1)
            body = body[(nl + 1)...]
        }
        let parts = body.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: true)
        return Range(lines: parts.map { String(decoding: $0, as: UTF8.self) }, start: start, size: size)
    }

    public static func items(_ lines: [String]) -> [ConversationItem] {
        var p = ConversationParser()
        for l in lines { p.feed(line: l) }
        return p.items()
    }

    /// The model named by the newest assistant line ("claude-opus-5-5", "gpt-5.5-codex"), for logs with no status line.
    public static func model(_ lines: [String]) -> String? {
        for l in lines.reversed() where l.contains("\"model\"") {
            guard let o = (try? JSONSerialization.jsonObject(with: Data(l.utf8))) as? [String: Any] else { continue }
            if let m = (o["message"] as? [String: Any])?["model"] as? String, !m.isEmpty, !m.hasPrefix("<") { return m }
            if let m = (o["payload"] as? [String: Any])?["model"] as? String, !m.isEmpty { return m }
        }
        return nil
    }
}
