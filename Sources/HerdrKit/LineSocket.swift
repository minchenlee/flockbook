import Foundation
import Darwin

public enum HerdrError: Error, Sendable, CustomStringConvertible {
    case socket(String)
    case server(code: String, message: String)
    case protocolError(String)
    case closed
    /// A read or write ran past the socket timeout. The request may still have reached the server.
    case timeout(String)

    public var description: String {
        switch self {
        case .socket(let m): "socket: \(m)"
        case .server(let c, let m): "server error \(c): \(m)"
        case .protocolError(let m): "protocol: \(m)"
        case .closed: "connection closed"
        case .timeout(let m): "timeout: \(m)"
        }
    }
}

/// Blocking NDJSON connection over a Unix socket. Call from a background task only.
final class LineSocket: @unchecked Sendable {
    /// Guarded by `fdLock`. A reader thread and writers use it while another thread may close the socket, so the
    /// descriptor is closed only when nobody is inside a syscall on it: `close()` shuts the socket down (waking a
    /// blocked read) and the last user closes it. Its number can never go to a new socket while a write is pending.
    private var _fd: Int32
    private let fdLock = NSLock()
    private var users = 0
    private var closing = false
    private var buffer = [UInt8]()
    var fileDescriptor: Int32 { fdLock.withLock { _fd } }

    /// The descriptor for one syscall (pair with `releaseFD`), or `.closed` once `close()` ran.
    func acquireFD() throws -> Int32 {
        try fdLock.withLock {
            guard !closing, _fd >= 0 else { throw HerdrError.closed }
            users += 1
            return _fd
        }
    }

    func releaseFD() {
        fdLock.withLock {
            users -= 1
            if closing, users == 0, _fd >= 0 { Darwin.close(_fd); _fd = -1 }
        }
    }

    /// One read under a use: bytes read (> 0), or throws `.closed` at EOF, `.timeout`, `.socket`.
    private func readSome(_ chunk: inout [UInt8]) throws -> Int {
        while true {
            let fd = try acquireFD()
            let n = Darwin.read(fd, &chunk, chunk.count)
            let e = errno
            releaseFD()
            if n < 0 { if e == EINTR { continue }; if e == EAGAIN { throw HerdrError.timeout("read") }; throw HerdrError.socket("read errno \(e)") }
            if n == 0 { throw HerdrError.closed }
            return n
        }
    }

    /// `timeout`: seconds for each read and write. nil blocks forever (event streams).
    init(path: String, timeout: TimeInterval? = nil) throws {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        _fd = fd
        guard fd >= 0 else { throw HerdrError.socket("socket() failed: \(errno)") }
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        let cap = MemoryLayout.size(ofValue: addr.sun_path)
        // A thrown init still runs deinit: drop `_fd` first so the number is closed once (it may be reused at once).
        guard bytes.count < cap else { _fd = -1; Darwin.close(fd); throw HerdrError.socket("path too long") }
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            for (i, b) in bytes.enumerated() { raw[i] = b }
            raw[bytes.count] = 0
        }
        let rc = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard rc == 0 else {
            let e = errno
            _fd = -1
            Darwin.close(fd)
            throw HerdrError.socket("connect \(path) failed: errno \(e)")
        }
        var one: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
        if let timeout {
            var tv = timeval(tv_sec: Int(timeout), tv_usec: Int32((timeout - timeout.rounded(.down)) * 1_000_000))
            setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
            setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        }
    }

    func writeLine(_ data: Data) throws {
        let fd = try acquireFD()
        defer { releaseFD() }
        var out = data
        out.append(0x0A)
        try out.withUnsafeBytes { (p: UnsafeRawBufferPointer) in
            var sent = 0
            while sent < p.count {
                let n = Darwin.write(fd, p.baseAddress! + sent, p.count - sent)
                if n < 0 { if errno == EINTR { continue }; throw HerdrError.socket("write errno \(errno)") }
                sent += n
            }
        }
    }

    /// Next full line, or throws `.closed` at EOF.
    func readLine() throws -> Data {
        while true {
            if let nl = buffer.firstIndex(of: 0x0A) {
                let line = Data(buffer[..<nl])
                buffer.removeSubrange(...nl)
                return line
            }
            var chunk = [UInt8](repeating: 0, count: 65536)
            let n = try readSome(&chunk)
            buffer.append(contentsOf: chunk[..<n])
        }
    }

    /// Writes raw bytes (framed mode). Same error rules as `writeLine`.
    func writeAll(_ data: Data) throws {
        let fd = try acquireFD()
        defer { releaseFD() }
        try data.withUnsafeBytes { (p: UnsafeRawBufferPointer) in
            var sent = 0
            while sent < p.count {
                let n = Darwin.write(fd, p.baseAddress! + sent, p.count - sent)
                if n < 0 { if errno == EINTR { continue }; throw HerdrError.socket("write errno \(errno)") }
                sent += n
            }
        }
    }

    /// Bytes already read past the last line (a protocol switch hands them to the next decoder), then clears them.
    func takeBuffered() -> [UInt8] { defer { buffer = [] }; return buffer }

    /// Next chunk of raw bytes, or throws `.closed` at EOF.
    func readChunk() throws -> ArraySlice<UInt8> {
        var chunk = [UInt8](repeating: 0, count: 65536)
        let n = try readSome(&chunk)
        return chunk[..<n]
    }

    /// Drops the read/write timeout (a connection that turns into a long-lived stream).
    func clearTimeout() {
        guard let fd = try? acquireFD() else { return }
        defer { releaseFD() }
        var tv = timeval(tv_sec: 0, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
    }

    func close() {
        fdLock.withLock {
            guard !closing, _fd >= 0 else { return }
            closing = true
            Darwin.shutdown(_fd, SHUT_RDWR)   // wakes a blocked read; the descriptor itself stays until the last user leaves
            if users == 0 { Darwin.close(_fd); _fd = -1 }
        }
    }

    deinit { if _fd >= 0 { Darwin.close(_fd) } }
}
