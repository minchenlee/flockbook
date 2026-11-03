import Foundation

/// One herdr-core framed-mode frame: `[channel u32 LE][kind u8][len u32 LE][payload]`, payload at most 16 MiB.
/// Channel 0 DATA carries JSON-RPC; other channels carry attach/event stream bytes.
public struct HCFrame: Sendable, Equatable {
    public enum Kind: Sendable, Equatable {
        case data, credit, close, resize, focus, facts
        case unknown(UInt8)
        init(_ b: UInt8) {
            switch b { case 0: self = .data; case 1: self = .credit; case 2: self = .close; case 3: self = .resize
            case 4: self = .focus; case 5: self = .facts; default: self = .unknown(b) }
        }
        var byte: UInt8 {
            switch self { case .data: 0; case .credit: 1; case .close: 2; case .resize: 3; case .focus: 4; case .facts: 5
            case .unknown(let b): b }
        }
    }
    public static let maxPayload = 16 * 1024 * 1024
    public var channel: UInt32
    public var kind: Kind
    public var payload: Data

    public init(channel: UInt32, kind: Kind, payload: Data) { self.channel = channel; self.kind = kind; self.payload = payload }

    public func encoded() -> Data {
        var d = Data(capacity: 9 + payload.count)
        withUnsafeBytes(of: channel.littleEndian) { d.append(contentsOf: $0) }
        d.append(kind.byte)
        withUnsafeBytes(of: UInt32(payload.count).littleEndian) { d.append(contentsOf: $0) }
        d.append(payload)
        return d
    }

    public static func credit(channel: UInt32, bytes: Int) -> HCFrame {
        var d = Data(); withUnsafeBytes(of: UInt32(clamping: bytes).littleEndian) { d.append(contentsOf: $0) }
        return HCFrame(channel: channel, kind: .credit, payload: d)
    }
    public static func resize(channel: UInt32, cols: Int, rows: Int) -> HCFrame {
        var d = Data()
        withUnsafeBytes(of: UInt16(clamping: cols).littleEndian) { d.append(contentsOf: $0) }
        withUnsafeBytes(of: UInt16(clamping: rows).littleEndian) { d.append(contentsOf: $0) }
        return HCFrame(channel: channel, kind: .resize, payload: d)
    }
    public static func focus(channel: UInt32, _ gained: Bool) -> HCFrame {
        HCFrame(channel: channel, kind: .focus, payload: Data([gained ? 1 : 0]))
    }
    public static func close(channel: UInt32) -> HCFrame { HCFrame(channel: channel, kind: .close, payload: Data()) }
}

/// Incremental decoder: feed bytes as they arrive, get whole frames back.
public struct HCFrameDecoder: Sendable {
    private var buf: [UInt8] = []
    public init() {}

    public mutating func feed(_ bytes: ArraySlice<UInt8>) throws -> [HCFrame] {
        buf.append(contentsOf: bytes)
        var out: [HCFrame] = []
        var at = 0
        while buf.count - at >= 9 {
            let ch = UInt32(buf[at]) | UInt32(buf[at + 1]) << 8 | UInt32(buf[at + 2]) << 16 | UInt32(buf[at + 3]) << 24
            let kind = buf[at + 4]
            let len = Int(UInt32(buf[at + 5]) | UInt32(buf[at + 6]) << 8 | UInt32(buf[at + 7]) << 16 | UInt32(buf[at + 8]) << 24)
            guard len <= HCFrame.maxPayload else { throw HerdrError.protocolError("hc frame payload \(len) bytes is over 16 MiB") }
            guard buf.count - at - 9 >= len else { break }
            out.append(HCFrame(channel: ch, kind: .init(kind), payload: Data(buf[(at + 9)..<(at + 9 + len)])))
            at += 9 + len
        }
        if at > 0 { buf.removeFirst(at) }
        return out
    }
}
