import Foundation
import Darwin
@testable import HerdrKit

/// A tiny herdr stand-in on a Unix socket, for timing the store without touching a real session.
/// Answers `session.snapshot` with one workspace whose label is `label`, keeps `events.subscribe` connections open,
/// and can push an event, drop every connection, or stop listening (socket loss).
final class FakeHerdr: @unchecked Sendable {
    let path: String
    private let lock = NSLock()
    private var listenFD: Int32 = -1
    private var subscribers: [Int32] = []
    private var open: [Int32] = []
    private var _label = "v0"
    private var _snapshots = 0
    private var _accepts = 0
    private var _screen = ""
    private var _agents = "[]"
    private var _requests: [(method: String, params: [String: Any])] = []
    private var _onInput: ((String, [String: Any]) -> Void)?
    /// Runs after each input request (send_text, send_keys, send_input), before its reply: a test changes the screen.
    var onInput: ((String, [String: Any]) -> Void)? { get { lock.withLock { _onInput } } set { lock.withLock { _onInput = newValue } } }

    init(path: String) { self.path = path }

    var label: String { get { lock.withLock { _label } } set { lock.withLock { _label = newValue } } }
    var snapshotCount: Int { lock.withLock { _snapshots } }
    /// Connections accepted so far (any request, events too).
    var acceptCount: Int { lock.withLock { _accepts } }
    /// What `pane.read` returns as the visible screen.
    var screen: String { get { lock.withLock { _screen } } set { lock.withLock { _screen = newValue } } }
    /// The snapshot's `agents` array, as JSON.
    var agents: String { get { lock.withLock { _agents } } set { lock.withLock { _agents = newValue } } }
    /// Every request received, in order.
    var requests: [(method: String, params: [String: Any])] { lock.withLock { _requests } }

    func start() throws {
        unlink(path)
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        var addr = sockaddr_un(); addr.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            for (i, b) in path.utf8.enumerated() { raw[i] = b }; raw[path.utf8.count] = 0
        }
        let rc = withUnsafePointer(to: &addr) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) } }
        guard rc == 0, listen(fd, 16) == 0 else { close(fd); throw HerdrError.socket("fake bind \(errno)") }
        lock.withLock { listenFD = fd }
        Thread.detachNewThread { [self] in
            while true {
                let c = accept(fd, nil, nil)
                if c < 0 { return }
                var one: Int32 = 1
                setsockopt(c, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
                lock.withLock { open.append(c); _accepts += 1 }
                Thread.detachNewThread { self.serve(c) }
            }
        }
    }

    /// Socket loss: stop listening, close every connection, remove the socket file.
    func kill() {
        lock.withLock {
            if listenFD >= 0 { shutdown(listenFD, SHUT_RDWR); close(listenFD); listenFD = -1 }
            for c in open { shutdown(c, SHUT_RDWR); close(c) }
            open = []; subscribers = []
        }
        unlink(path)
    }

    /// Sends one event line to every subscriber.
    func pushEvent() {
        let line = Array(#"{"event":"workspace.renamed","data":{"workspace_id":"w1"}}"#.utf8) + [0x0A]
        let subs = lock.withLock { subscribers }
        for s in subs { _ = line.withUnsafeBytes { write(s, $0.baseAddress, $0.count) } }
    }

    private func serve(_ c: Int32) {
        var buf = [UInt8]()
        var chunk = [UInt8](repeating: 0, count: 4096)
        while true {
            let n = read(c, &chunk, chunk.count)
            if n <= 0 { return }
            buf += chunk[..<n]
            while let nl = buf.firstIndex(of: 0x0A) {
                let line = Data(buf[..<nl]); buf.removeSubrange(...nl)
                guard let o = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { continue }
                let id = o["id"] as? String ?? ""
                lock.withLock { _requests.append((o["method"] as? String ?? "", o["params"] as? [String: Any] ?? [:])) }
                let reply: String
                switch o["method"] as? String {
                case "session.snapshot":
                    let (l, agents) = lock.withLock { _snapshots += 1; return (_label, _agents) }
                    reply = #"{"id":"\#(id)","result":{"type":"snapshot","snapshot":{"workspaces":[{"workspace_id":"w1","label":"\#(l)","number":1}],"tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1}],"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","terminal_id":"term_1","cwd":"/tmp"}],"agents":\#(agents)}}}"#
                case "events.subscribe":
                    lock.withLock { subscribers.append(c) }
                    reply = #"{"id":"\#(id)","result":{"type":"subscription_started"}}"#
                case "pane.read":
                    let text = String(decoding: try! JSONSerialization.data(withJSONObject: [screen]), as: UTF8.self).dropFirst().dropLast()
                    reply = #"{"id":"\#(id)","result":{"type":"pane_read","read":{"text":\#(text)}}}"#
                case "pane.send_text", "pane.send_keys", "pane.send_input":
                    onInput?(o["method"] as? String ?? "", o["params"] as? [String: Any] ?? [:])
                    reply = #"{"id":"\#(id)","result":{"type":"ok"}}"#
                default:
                    reply = #"{"id":"\#(id)","error":{"code":"unknown","message":"fake"}}"#
                }
                let out = Array(reply.utf8) + [0x0A]
                _ = out.withUnsafeBytes { write(c, $0.baseAddress, $0.count) }
            }
        }
    }
}
