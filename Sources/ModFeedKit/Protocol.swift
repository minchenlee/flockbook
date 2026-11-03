import Foundation

/// The sidecar's output: JSON Lines (one object per line). The first line is a `header` with the protocol version;
/// the last is an `end`. The contract is written in the mod repo's docs/protocol.md; a sidecar in another language
/// must print the same lines.
public enum FeedProtocol {
    /// Bumped on any change a mod must know about. The mod refuses a sidecar with another number.
    public static let version = 1
    public static let sidecarVersion = "0.1.0"
}

/// Collects output lines; `flush` prints them. Keys are sorted so the output is the same every time.
public final class JSONLines: @unchecked Sendable {
    public private(set) var lines: [String] = []
    private let encoder: JSONEncoder = { let e = JSONEncoder(); e.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]; return e }()
    public init() {}

    public func emit<T: Encodable>(_ value: T) {
        if let d = try? encoder.encode(value) { lines.append(String(decoding: d, as: UTF8.self)) }
    }

    public func emit(_ object: [String: Any?]) {
        let clean = object.compactMapValues { $0 }
        if let d = try? JSONSerialization.data(withJSONObject: clean, options: [.sortedKeys, .withoutEscapingSlashes]) {
            lines.append(String(decoding: d, as: UTF8.self))
        }
    }

    /// A line that is already JSON.
    public func raw(_ line: String) { lines.append(line) }

    public func text() -> String { lines.map { $0 + "\n" }.joined() }
}

/// A stable 64-bit FNV-1a hash (Swift's `Hasher` is seeded per process). Rows are compared across runs with it.
public func fnv1a(_ s: String) -> String {
    var h: UInt64 = 0xcbf29ce484222325
    for b in s.utf8 { h ^= UInt64(b); h = h &* 0x100000001b3 }
    return String(h, radix: 16)
}
