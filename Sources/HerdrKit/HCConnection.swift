import Foundation

/// JSON-RPC 2.0 helpers shared by the plain and framed hc connections.
/// A decoded JSON-RPC result object. JSONSerialization output is immutable once built, so it is safe to share.
public struct HCJSON: @unchecked Sendable {
    public let object: [String: Any]
    public init(_ object: [String: Any]) { self.object = object }
    public subscript(key: String) -> Any? { object[key] }
}

enum HCRPC {
    static func request(id: Int, method: String, params: [String: Any]) throws -> Data {
        try JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": id, "method": method, "params": params])
    }

    /// The message's integer id, or nil for a notification (no id) or a non-integer id.
    static func id(of o: [String: Any]) -> Int? { (o["id"] as? NSNumber)?.intValue }

    /// The `result` object (empty when it is null or not an object), or the error as `HerdrError.server(code: kind, …)`.
    /// hc errors carry a stable `data.kind` (`not_found`, `conflict`, `unavailable`, …); without it the numeric code is used.
    static func result(_ o: [String: Any]) throws -> HCJSON {
        if let err = o["error"] as? [String: Any] {
            let kind = (err["data"] as? [String: Any])?["kind"] as? String
            throw HerdrError.server(code: kind ?? "\((err["code"] as? NSNumber)?.intValue ?? 0)", message: err["message"] as? String ?? "")
        }
        return HCJSON(o["result"] as? [String: Any] ?? [:])
    }
}

/// A framed-mode hc connection: one socket, JSON-RPC on channel 0 (calls run side by side, matched by id), and
/// attach streams on their own channels. A reader thread demultiplexes. Use from any thread.
public final class HCFramedConnection: @unchecked Sendable {
    private let sock: LineSocket
    private let lock = NSLock()
    private let writeLock = NSLock()
    private var nextId = 1
    private var pending: [Int: CheckedContinuation<HCJSON, Error>] = [:]
    private var streams: [UInt32: HCAttachment] = [:]
    /// Frames that arrived on a channel between its attach reply and `attach()` registering the stream.
    /// Only channels in `awaiting` are parked; frames for any other unknown channel (late output, the core's empty
    /// CLOSE answer after our CLOSE) are dropped, so nothing piles up and a reused channel id starts clean.
    private var early: [UInt32: [HCFrame]] = [:]
    /// Channels named by an attach reply that `attach()` has not registered yet. Set on the reader thread, in socket
    /// order, before any frame of that channel can be dispatched.
    private var awaiting: Set<UInt32> = []
    /// Call ids of `terminal.attach` requests (their replies carry a channel).
    private var attachCalls: Set<Int> = []
    static let maxEarlyFrames = 4096
    private var closed = false

    private init(sock: LineSocket) { self.sock = sock }

    /// Connects, sends `hello {framed:true}` in plain mode and switches to framed mode after its reply.
    public static func open(socketPath: String) async throws -> HCFramedConnection {
        try await Task.detached(priority: .userInitiated) {
            let sock = try LineSocket(path: socketPath, timeout: 10)
            do {
                try sock.writeLine(try HCRPC.request(id: 0, method: "hello", params: ["framed": true, "generation": 1]))
                let line = try sock.readLine()
                guard let o = try JSONSerialization.jsonObject(with: line) as? [String: Any] else { throw HerdrError.protocolError("hello: not an object") }
                let r = try HCRPC.result(o)
                guard r["framed"] as? Bool == true else { throw HerdrError.protocolError("hello: core did not switch to framed mode") }
            } catch { sock.close(); throw error }
            sock.clearTimeout()
            let conn = HCFramedConnection(sock: sock)
            let leftover = sock.takeBuffered()
            Thread.detachNewThread { conn.readLoop(leftover) }
            return conn
        }.value
    }

    /// Ends the connection: pending calls fail, open streams finish.
    public func shutdown() { fail(HerdrError.closed) }

    public var isClosed: Bool { lock.withLock { closed } }
    /// Frames parked for channels whose attach reply is being handled (checks only).
    var earlyCount: Int { lock.withLock { early.values.reduce(0) { $0 + $1.count } } }

    func write(_ f: HCFrame) {
        let d = f.encoded()
        writeLock.withLock { _ = try? sock.writeAll(d) }
    }

    /// One JSON-RPC call on channel 0.
    public func call(_ method: String, _ params: [String: any Sendable] = [:]) async throws -> HCJSON {
        try await call(method, params, attach: false)
    }

    private func call(_ method: String, _ params: [String: any Sendable], attach: Bool) async throws -> HCJSON {
        let (id, isClosed) = lock.withLock { () -> (Int, Bool) in
            nextId += 1
            if attach, !closed { attachCalls.insert(nextId) }
            return (nextId, closed)
        }
        if isClosed { throw HerdrError.closed }
        let body = try HCRPC.request(id: id, method: method, params: params as [String: Any])
        return try await withCheckedThrowingContinuation { cont in
            let ok = lock.withLock { () -> Bool in
                if closed { return false }
                pending[id] = cont; return true
            }
            guard ok else { cont.resume(throwing: HerdrError.closed); return }
            write(HCFrame(channel: 0, kind: .data, payload: body))
        }
    }

    /// `terminal.attach` on this connection. The stream starts with the first paint, then the program's raw output.
    /// `observe`: watch only, never type or resize (the core enforces it).
    /// `resumed`: back after a dropped line; the core sets the size only once this viewer is active again.
    public func attach(terminal: String, cols: Int, rows: Int, history: Int = 0, observe: Bool = false,
                       resumed: Bool = false) async throws -> HCAttachment {
        let r = try await call("terminal.attach", ["terminal": terminal, "cols": cols, "rows": rows, "history": history,
                                                   "observe": observe, "resumed": resumed], attach: true)
        guard let ch = (r["channel"] as? NSNumber)?.uint32Value else { throw HerdrError.protocolError("terminal.attach: no channel") }
        let term = r["terminal"] as? [String: Any]
        let att = HCAttachment(channel: ch, connection: self,
                               cols: (term?["cols"] as? NSNumber)?.intValue ?? cols, rows: (term?["rows"] as? NSNumber)?.intValue ?? rows)
        // Hold the attachment's delivery lock while it is registered and its backlog goes out: a live frame the reader
        // finds under `streams[ch]` waits for that lock, so it always lands after the first paint and the parked frames.
        if !register(att) { att.finish(error: HerdrError.closed) }
        return att
    }

    private func register(_ att: HCAttachment) -> Bool {
        att.deliveryLock.withLock {
            let ch = att.channel
            let backlog = lock.withLock { () -> [HCFrame]? in
                if closed { return nil }
                streams[ch] = att
                awaiting.remove(ch)
                return early.removeValue(forKey: ch) ?? []
            }
            for f in backlog ?? [] { att.deliverLocked(f) }
            return backlog != nil
        }
    }

    func detach(_ ch: UInt32) {
        lock.withLock { streams[ch] = nil; early[ch] = nil; awaiting.remove(ch) }
    }

    private func readLoop(_ leftover: [UInt8]) {
        var dec = HCFrameDecoder()
        do {
            var frames = try dec.feed(leftover[...])
            while true {
                for f in frames { dispatch(f) }
                frames = try dec.feed(try sock.readChunk())
            }
        } catch {
            fail(error)
        }
    }

    private func dispatch(_ f: HCFrame) {
        if f.channel == 0 {
            guard f.kind == .data, let o = try? JSONSerialization.jsonObject(with: f.payload) as? [String: Any],
                  let id = HCRPC.id(of: o) else { return }   // notifications on channel 0 are not used
            // An attach reply opens its channel for parking right here, before the reader can see the channel's frames.
            let attachCh = (o["result"] as? [String: Any])?["channel"] as? NSNumber
            guard let cont = lock.withLock({ () -> CheckedContinuation<HCJSON, Error>? in
                if attachCalls.remove(id) != nil, let attachCh { awaiting.insert(attachCh.uint32Value) }
                return pending.removeValue(forKey: id)
            }) else { return }
            do { cont.resume(returning: try HCRPC.result(o)) } catch { cont.resume(throwing: error) }
            return
        }
        let att = lock.withLock { () -> HCAttachment? in
            if let s = streams[f.channel] { return s }
            if awaiting.contains(f.channel), early[f.channel, default: []].count < Self.maxEarlyFrames { early[f.channel, default: []].append(f) }
            return nil
        }
        att?.deliver(f)
    }

    private func fail(_ error: Error) {
        let (conts, atts) = lock.withLock { () -> ([CheckedContinuation<HCJSON, Error>], [HCAttachment]) in
            let c = Array(pending.values), a = Array(streams.values)
            pending = [:]; streams = [:]; early = [:]; awaiting = []; attachCalls = []; closed = true
            return (c, a)
        }
        sock.close()
        for c in conts { c.resume(throwing: error) }
        for a in atts { a.finish(error: error) }
    }
}

/// One attach stream. Read `bytes`; call `consumed(_:)` after the viewer has written them out (CREDIT, so the core
/// keeps sending: it starts with 256 KiB and repaints a viewer that falls 4 MiB behind).
public final class HCAttachment: @unchecked Sendable {
    public let channel: UInt32
    /// The pty size when attached (the terminal record's cols/rows).
    public let cols: Int
    public let rows: Int
    public let bytes: AsyncThrowingStream<Data, Error>
    private let cont: AsyncThrowingStream<Data, Error>.Continuation
    private weak var connection: HCFramedConnection?
    private let lock = NSLock()
    private var _closeReason: [String: Any]?
    private var done = false
    /// Serializes delivery: the connection holds it while it hands over the backlog (see `attach`).
    let deliveryLock = NSLock()

    init(channel: UInt32, connection: HCFramedConnection, cols: Int = 0, rows: Int = 0) {
        self.channel = channel
        self.cols = cols; self.rows = rows
        self.connection = connection
        (bytes, cont) = AsyncThrowingStream.makeStream(of: Data.self)
    }

    /// The core's CLOSE reason (`{"reason":"exited","exit_code":0,"signal":null}`), once the stream ended that way.
    public var closeReason: [String: Any]? { lock.withLock { _closeReason } }

    /// No CREDIT once the stream ended: the core dropped the channel, and its id may already belong to a new attach.
    public func consumed(_ n: Int) {
        guard n > 0, !lock.withLock({ done }) else { return }
        connection?.write(.credit(channel: channel, bytes: n))
    }
    /// Keys/text from the viewer, already encoded (the core never encodes keys on this path).
    public func send(_ data: Data) { if !data.isEmpty { connection?.write(HCFrame(channel: channel, kind: .data, payload: data)) } }
    public func resize(cols: Int, rows: Int) { connection?.write(.resize(channel: channel, cols: cols, rows: rows)) }
    public func focus(_ gained: Bool) { connection?.write(.focus(channel: channel, gained)) }

    /// Detaches: sends CLOSE and ends the stream locally.
    public func close() {
        guard lock.withLock({ () -> Bool in if done { return false }; done = true; return true }) else { return }
        connection?.write(.close(channel: channel))
        connection?.detach(channel)
        cont.finish()
    }

    func deliver(_ f: HCFrame) { deliveryLock.withLock { deliverLocked(f) } }

    /// Caller holds `deliveryLock`.
    func deliverLocked(_ f: HCFrame) {
        switch f.kind {
        case .data: cont.yield(f.payload)
        case .close:
            let reason = (try? JSONSerialization.jsonObject(with: f.payload)) as? [String: Any]
            lock.withLock { _closeReason = reason; done = true }
            connection?.detach(channel)
            cont.finish()
        default: break   // CREDIT for our own keys: they are tiny, nothing to pace
        }
    }

    func finish(error: Error) {
        lock.withLock { done = true }
        cont.finish(throwing: error)
    }
}
