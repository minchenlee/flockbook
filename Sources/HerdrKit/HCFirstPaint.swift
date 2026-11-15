import Foundation

/// A terminal's first paint, read through a watch-only attach (`terminal.attach` with `observe: true`) on a framed
/// connection of its own. Only the paint has the screen's attributes (dim) and the cursor; `terminal.read` has text
/// only. Twin of herdr-core `hc::observe`.
///
/// Read-only: the connection sends the framed `hello`, one attach request and a CLOSE for its channel, nothing else
/// (no keys, no resize, no takeover; the core also holds an observer to that). It ends at `limit` at the latest.
enum HCFirstPaint {
    /// The paint is done once nothing more came for this long.
    static let quiet: TimeInterval = 0.06
    private static func now() -> TimeInterval { ProcessInfo.processInfo.systemUptime }

    /// What the look got so far. Once `over`, a connection or attachment that comes late is closed at once.
    private final class Look: @unchecked Sendable {
        private let lock = NSLock()
        private var data = Data(), last: TimeInterval = 0, ended = false, error: Error?, over = false
        private var conn: HCFramedConnection?, att: HCAttachment?

        func keep(_ c: HCFramedConnection) -> Bool { lock.withLock { if over { return false }; conn = c; return true } }
        func keep(_ a: HCAttachment) -> Bool { lock.withLock { if over { return false }; att = a; return true } }
        func append(_ d: Data, at t: TimeInterval) { lock.withLock { data.append(d); last = t } }
        func end(_ e: Error?) { lock.withLock { ended = true; error = e } }
        /// Detaches and closes, whatever happened.
        func finish() {
            let (c, a) = lock.withLock { () -> (HCFramedConnection?, HCAttachment?) in over = true; return (conn, att) }
            a?.close()
            c?.close()
        }
        /// The paint, whether the stream ended, its error, and whether everything handed out is read with no frame
        /// part still coming.
        func state() -> (data: Data, last: TimeInterval, ended: Bool, error: Error?, settled: Bool) {
            lock.withLock {
                let settled = att.map { $0.deliveredBytes == data.count } == true && conn?.hasPartialFrame == false
                return (data, last, ended, error, settled)
            }
        }
    }

    /// The first paint of `terminal`, or an error: the connection failed, the core refused, or no paint came in `limit`.
    static func read(socketPath: String, terminal: String, limit: TimeInterval) async throws -> Data {
        let end = now() + limit
        let look = Look()
        let worker = Task.detached {
            do {
                let conn = try await HCFramedConnection.open(socketPath: socketPath)
                guard look.keep(conn) else { conn.close(); return }
                let att = try await conn.attach(terminal: terminal, cols: 80, rows: 24, history: 0, observe: true)
                guard look.keep(att) else { att.close(); conn.close(); return }
                for try await d in att.bytes { look.append(d, at: now()) }
                look.end(nil)
            } catch { look.end(error) }
        }
        defer { worker.cancel(); look.finish() }
        while true {
            try await Task.sleep(for: .milliseconds(10))
            let s = look.state(), t = now()
            if let e = s.error { throw e }
            // The core closed the channel: the paint so far, or nothing.
            if s.ended { if s.data.isEmpty { throw HerdrError.closed }; return s.data }
            // A frame part still coming (a redraw, maybe of typed text) is not quiet, and not done.
            if !s.data.isEmpty, s.settled, t - s.last >= quiet { return s.data }
            if t >= end {
                if s.data.isEmpty || !s.settled { throw HerdrError.timeout("first paint") }
                return s.data
            }
        }
    }
}
