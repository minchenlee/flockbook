import Foundation
import Darwin
@testable import HerdrKit

/// A tiny herdr-core (`hc`) stand-in on a Unix socket. Speaks JSON-RPC 2.0 lines (plain mode) and, after
/// `hello {framed:true}`, the framed codec. Serves `terminal.list`, `terminal.get`, `doc.get`, `events.subscribe`
/// (plain mode), `terminal.send`, `terminal.keys`, `terminal.attach` (framed). Records every call and every byte a viewer
/// sends, so checks can assert on them. Never touches a real hc.
final class FakeHC: @unchecked Sendable {
    let path: String
    private let lock = NSLock()
    private var listenFD: Int32 = -1
    /// Each `start` is a generation; `kill` ends it. An accept thread serves only while its generation is current.
    private var generation = 0
    private var activeGeneration = -1
    private var open: [Int32] = []
    private var subscribers: [Int32] = []
    private var _terminals: [[String: Any]] = []
    private var _docs: [String: Any] = [:]
    private var _docVersions: [String: Int] = [:]
    private var nextTerminal = 100
    /// Layout writes: `doc.set` stores with a version (`conflict` on a stale one), `terminal.run/kill/label` change the
    /// list. Off: they are refused, as before.
    var writable = false
    /// The next `doc.set` calls fail with `conflict` after this edit ran as "another writer" (version bumped).
    var conflicts: [(Any?) -> Any] = []
    /// `terminal.kill` on these addresses fails (and the terminal stays).
    var killFails: Set<String> = []
    private var _calls: [(method: String, params: [String: Any])] = []
    private var _credits: [UInt32: Int] = [:]
    private var _viewerBytes: [UInt32: Data] = [:]
    private var _resizes: [UInt32: [(Int, Int)]] = [:]
    private var _closedByViewer: Set<UInt32> = []
    private var attachWriters: [UInt32: (Int32, NSLock)] = [:]
    private var nextChannel: UInt32 = 1
    /// Connection of every channel ever opened (kept after CLOSE, so checks can send late frames).
    private var connOf: [UInt32: (Int32, NSLock)] = [:]
    /// Reuse the lowest free channel id after a CLOSE (the core may).
    var reuseChannels = false
    /// Numbered DATA frames ("#1;", "#2;", …) sent right after the first paint of each attach.
    var paintBurst = 0
    /// Seconds `terminal.send` takes before it answers (the core waits for typing quiet, then for `working`).
    var sendDelay: Double = 0
    /// Seconds `terminal.keys` takes before it answers (hc's typing-quiet rule can hold keys this long).
    var keysDelay: Double = 0
    /// When set, `terminal.attach` fails with this error kind.
    var attachError: String?
    /// Seconds before `terminal.attach` answers.
    var attachDelay: Double = 0
    /// Seconds `doc.get` takes per document key (`on/name`): a slow or hung machine.
    var docDelays: [String: Double] = [:]
    /// `doc.get` of these documents (`docKey`) fails; `machine.list` fails while `machineListFails`.
    var docFails: Set<String> = []
    var machineListFails = false
    /// Where `terminal.put` says the file landed.
    var putDir = "/home/b/.hc/put"
    /// Seconds a `file.read` of `<on>:<path>` waits before it answers.
    var readDelays: [String: Double] = [:]
    /// Most bytes one `file.read` gives (hc: 8 MB).
    var fileByteCap = 8 << 20
    /// The newest attach channel (0: none yet).
    var lastChannel: UInt32 { lock.withLock { nextChannel - 1 } }
    private var _screens: [String: String] = [:]
    /// What `terminal.read` returns per address (the visible screen as text).
    func setScreen(_ address: String, _ text: String) { lock.withLock { _screens[address] = text } }
    /// Called after each `terminal.keys` (address, keys), e.g. to move a fake picker on.
    var onKeys: (@Sendable (String, [String]) -> Void)?
    private var seq = 0
    /// First paint sent on each new attach channel.
    var firstPaint = Data("\u{1b}[2J\u{1b}[Hhello from t1\r\n".utf8)

    init(path: String) { self.path = path }

    var terminals: [[String: Any]] { get { lock.withLock { _terminals } } set { lock.withLock { _terminals = newValue } } }
    func setDoc(_ name: String, _ value: Any) { lock.withLock { _docs[name] = value; _docVersions[name, default: 0] += 1 } }
    func doc(_ name: String) -> Any? { lock.withLock { _docs[name] } }
    func docVersion(_ name: String) -> Int { lock.withLock { _docVersions[name] ?? 0 } }
    var calls: [(method: String, params: [String: Any])] { lock.withLock { _calls } }
    private var _histories: [String: String] = [:]
    /// History above the screen per terminal (`terminal.read {lines}` gives its last `lines` lines first).
    var histories: [String: String] { get { lock.withLock { _histories } } set { lock.withLock { _histories = newValue } } }
    private var _execReplies: [(String, String)] = []
    /// `machine.exec`: the stdout of the first reply whose key occurs in the line (or argv joined); else empty, code 1.
    var execReplies: [(String, String)] { get { lock.withLock { _execReplies } } set { lock.withLock { _execReplies = newValue } } }
    private var _files: [String: String] = [:]
    /// `file.read`: contents by `<on>:<path>` (`:<path>` here).
    var files: [String: String] { get { lock.withLock { _files } } set { lock.withLock { _files = newValue } } }
    private var _machines: [[String: Any]] = []
    /// What `machine.list` returns. A doc on a machine is stored under `<machine>/<name>` (`doc.get/set {on}`).
    var machines: [[String: Any]] { get { lock.withLock { _machines } } set { lock.withLock { _machines = newValue } } }
    static func machine(_ name: String, state: String = "connected", temporary: Bool = false) -> [String: Any] {
        ["name": name, "id": "core-\(name)", "host": name, "target": "sh -c", "state": state, "temporary": temporary,
         "session": NSNull(), "since_ms": NSNull()]
    }

    /// `herdr-app` here, `box/herdr-app` for `{on: "box"}`.
    static func docKey(_ params: [String: Any]) -> String {
        let n = params["name"] as? String ?? ""
        guard let on = params["on"] as? String, !on.isEmpty else { return n }
        return "\(on)/\(n)"
    }
    func credits(_ ch: UInt32) -> Int { lock.withLock { _credits[ch] ?? 0 } }
    func viewerBytes(_ ch: UInt32) -> Data { lock.withLock { _viewerBytes[ch] ?? Data() } }
    func resizes(_ ch: UInt32) -> [(Int, Int)] { lock.withLock { _resizes[ch] ?? [] } }
    func closedByViewer(_ ch: UInt32) -> Bool { lock.withLock { _closedByViewer.contains(ch) } }

    static func terminal(_ addr: String, labels: [String: String] = [:], argv: [String] = ["claude"], pid: Int? = 4242,
                         status: String? = nil, kind: String? = nil, msg: String? = nil, turn: Int = 0, title: String = "",
                         cwd: String = "/tmp/proj", parent: String? = nil, fg: [String: Any]? = nil, state: String = "live") -> [String: Any] {
        var t: [String: Any] = ["address": addr, "labels": labels, "argv": argv, "cwd": cwd, "state": state,
                                "created_ms": 1, "screen": "exact", "cols": 80, "rows": 24, "turn": turn, "title": title,
                                "reported": status != nil, "records": [String: Any]()]
        t["pid"] = pid ?? NSNull()
        t["parent"] = parent ?? NSNull()
        if let status { var s: [String: Any] = ["state": status]; if let kind { s["kind"] = kind }; if let msg { s["msg"] = msg }; t["status"] = s } else { t["status"] = NSNull() }
        t["foreground"] = fg ?? NSNull()
        return t
    }

    func start() throws {
        unlink(path)
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        var addr = sockaddr_un(); addr.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            for (i, b) in path.utf8.enumerated() { raw[i] = b }; raw[path.utf8.count] = 0
        }
        let rc = withUnsafePointer(to: &addr) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) } }
        guard rc == 0, listen(fd, 16) == 0 else { close(fd); throw HerdrError.socket("fakehc bind \(errno)") }
        let gen = lock.withLock { () -> Int in listenFD = fd; generation += 1; activeGeneration = generation; return generation }
        // Only this thread closes `fd` (and each serve thread its own connection): a descriptor closed under a thread
        // still blocked on it could be reused by the next fake, and the old thread would take that fake's connections.
        Thread.detachNewThread { [self] in
            while true {
                let c = accept(fd, nil, nil)
                if lock.withLock({ activeGeneration != gen }) { if c >= 0 { close(c) }; close(fd); return }
                if c < 0 { if errno == EINTR || errno == ECONNABORTED { continue }; close(fd); return }
                var one: Int32 = 1
                setsockopt(c, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
                lock.withLock { open.append(c) }
                Thread.detachNewThread { self.serve(c) }
            }
        }
    }

    func kill() {
        let (wasListening, conns) = lock.withLock { () -> (Bool, [Int32]) in
            defer { activeGeneration = -1; listenFD = -1; subscribers = []; attachWriters = [:]; connOf = [:] }
            return (activeGeneration >= 0, open)
        }
        // Wake the accept thread (its generation ended: it closes the listening socket), then end every connection; the
        // serve threads close their descriptors themselves.
        if wasListening, let wake = try? LineSocket(path: path, timeout: 1) { wake.close() }
        for c in conns { shutdown(c, SHUT_RDWR) }
        unlink(path)
    }

    /// Pushes one event notification to every plain-mode subscriber.
    func pushEvent(type: String, terminal: String?, data: Any) {
        let s = lock.withLock { seq += 1; return seq }
        var p: [String: Any] = ["seq": s, "type": type, "data": data]
        if let terminal { p["terminal"] = terminal }
        let line = (try! JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "method": "event", "params": p])) + Data([0x0A])
        let subs = lock.withLock { subscribers }
        for c in subs { Self.writeAll(c, line) }
    }

    /// Sends program output on an attach channel (as the core does after the first paint).
    func output(_ ch: UInt32, _ bytes: Data) {
        guard let (c, wl) = lock.withLock({ attachWriters[ch] }) else { return }
        wl.withLock { Self.writeAll(c, HCFrame(channel: ch, kind: .data, payload: bytes).encoded()) }
    }

    /// Ends an attach channel the way the core does when the program exits.
    func exitProgram(_ ch: UInt32) {
        guard let (c, wl) = lock.withLock({ attachWriters[ch] }) else { return }
        let reason = Data(#"{"reason":"exited","exit_code":0,"signal":null}"#.utf8)
        wl.withLock { Self.writeAll(c, HCFrame(channel: ch, kind: .close, payload: reason).encoded()) }
    }

    /// Any frame on the connection that once carried `ch`, even after it closed (late frames, stale CLOSE).
    func rawFrame(_ ch: UInt32, _ kind: HCFrame.Kind, _ payload: Data = Data()) {
        guard let (c, wl) = lock.withLock({ connOf[ch] }) else { return }
        wl.withLock { Self.writeAll(c, HCFrame(channel: ch, kind: kind, payload: payload).encoded()) }
    }

    private static func writeAll(_ c: Int32, _ d: Data) {
        d.withUnsafeBytes { p in
            var sent = 0
            while sent < p.count { let n = write(c, p.baseAddress! + sent, p.count - sent); if n <= 0 { return }; sent += n }
        }
    }

    private func reply(id: Any, result: Any?, error: (Int, String, String)? = nil) -> Data {
        var o: [String: Any] = ["jsonrpc": "2.0", "id": id]
        if let error { o["error"] = ["code": error.0, "message": error.1, "data": ["kind": error.2, "hint": ""]] } else { o["result"] = result ?? NSNull() }
        return try! JSONSerialization.data(withJSONObject: o)
    }

    /// Answers one request. `framed` tells attach whether it may open a channel.
    private func handle(_ o: [String: Any], conn c: Int32, framed: Bool, writeLock: NSLock) -> (Data, after: (() -> Void)?) {
        let id = o["id"] ?? NSNull()
        let method = o["method"] as? String ?? ""
        let params = o["params"] as? [String: Any] ?? [:]
        lock.withLock { _calls.append((method, params)) }
        switch method {
        case "hello":
            return (reply(id: id, result: ["generation": 1, "version": "0.0-fake", "core": ["id": "fake", "name": "fakehost"],
                                           "framed": params["framed"] as? Bool ?? false, "methods": [String](), "features": [String]()]), nil)
        case "terminal.list":
            return (reply(id: id, result: ["terminals": terminals, "hidden_finished": 0, "machines": [Any]()]), nil)
        case "terminal.get":
            let a = params["terminal"] as? String ?? ""
            if let t = terminals.first(where: { $0["address"] as? String == a }) { return (reply(id: id, result: t), nil) }
            return (reply(id: id, result: nil, error: (-32000, "no terminal \(a)", "not_found")), nil)
        case "machine.exec":
            let cmd = (params["line"] as? String) ?? ((params["argv"] as? [String]) ?? []).joined(separator: " ")
            let hit = lock.withLock { _execReplies.first { cmd.contains($0.0) }?.1 }
            return (reply(id: id, result: ["stdout": Data((hit ?? "").utf8).base64EncodedString(), "stderr": "", "code": hit == nil ? 1 : 0,
                                           "truncated": false, "timed_out": false, "ms": 1]), nil)
        case "file.read":
            let key = "\(params["on"] as? String ?? ""):\(params["path"] as? String ?? "")"
            if let d = readDelays[key], d > 0 { Thread.sleep(forTimeInterval: d) }
            guard let text = lock.withLock({ _files[key] }) else {
                return (reply(id: id, result: nil, error: (-32000, "no such file", "not_found")), nil)
            }
            // Lines with their ends kept; a last line without one counts too.
            var lines: [Substring] = []
            var start = text.startIndex
            while start < text.endIndex {
                let end = text[start...].firstIndex(of: "\n").map { text.index(after: $0) } ?? text.endIndex
                lines.append(text[start..<end]); start = end
            }
            let first = max((params["offset"] as? Int) ?? 1, 1)
            let limit = (params["limit"] as? Int) ?? Int.max
            // As hc (live, 2026-11-02): a file that is not UTF-8 (here: holds a NUL) is `not_text` from any offset;
            // whole lines up to `fileByteCap` bytes; a first line over it is `invalid_params` "line N is over 8 MB".
            if text.contains("\u{0}") { return (reply(id: id, result: nil, error: (-32000, "x is not UTF-8 text", "not_text")), nil) }
            var last = first - 1, bytes = 0
            while last < lines.count, last - first + 1 < limit {
                bytes += lines[last].utf8.count
                if bytes > fileByteCap {
                    if last == first - 1 { return (reply(id: id, result: nil, error: (-32602, "line \(first) is over 8 MB", "invalid_params")), nil) }
                    break
                }
                last += 1
            }
            let got = first <= last ? lines[(first - 1)..<last].joined() : ""
            return (reply(id: id, result: ["text": got, "first": first, "last": max(last, first - 1), "lines": lines.count]), nil)
        case "machine.list":
            if machineListFails { return (reply(id: id, result: nil, error: (-32000, "core busy", "internal")), nil) }
            return (reply(id: id, result: ["machines": machines]), nil)
        case "doc.get":
            let n = Self.docKey(params)
            if let d = docDelays[n], d > 0 { Thread.sleep(forTimeInterval: d) }
            if docFails.contains(n) { return (reply(id: id, result: nil, error: (-32000, "machine timed out", "timeout")), nil) }
            let (v, ver) = lock.withLock { (_docs[n], _docVersions[n] ?? 0) }
            return (reply(id: id, result: ["value": v ?? NSNull(), "version": ver]), nil)
        case "doc.set" where writable:
            let n = Self.docKey(params)
            let err: Bool = lock.withLock {
                if !conflicts.isEmpty { let f = conflicts.removeFirst(); _docs[n] = f(_docs[n]); _docVersions[n, default: 0] += 1 }
                guard (params["version"] as? NSNumber)?.intValue == (_docVersions[n] ?? 0) else { return true }
                _docs[n] = params["value"]; _docVersions[n, default: 0] += 1
                return false
            }
            if err { return (reply(id: id, result: nil, error: (-32000, "document changed", "conflict")), nil) }
            return (reply(id: id, result: ["version": docVersion(n)]), nil)
        case "terminal.run" where writable:
            let on = (params["on"] as? String).map { $0 + "/" } ?? ""
            let addr: String = lock.withLock { nextTerminal += 1; return "\(on)t\(nextTerminal)" }
            var t = FakeHC.terminal(addr, labels: params["labels"] as? [String: String] ?? [:], argv: params["argv"] as? [String] ?? [],
                                    cwd: params["cwd"] as? String ?? "/home/fake")
            t["keep"] = params["keep"] as? Bool ?? false
            lock.withLock { _terminals.append(t) }
            return (reply(id: id, result: ["address": addr]), nil)
        case "terminal.kill" where writable && killFails.contains(params["terminal"] as? String ?? ""):
            return (reply(id: id, result: nil, error: (-32000, "fake cannot kill", "busy")), nil)
        case "terminal.kill" where writable:
            let a = params["terminal"] as? String ?? ""
            lock.withLock { _terminals.removeAll { $0["address"] as? String == a } }
            return (reply(id: id, result: [String: Any]()), nil)
        case "terminal.label" where writable:
            let a = params["terminal"] as? String ?? ""
            let l = params["labels"] as? [String: String] ?? [:]
            lock.withLock {
                if let i = _terminals.firstIndex(where: { $0["address"] as? String == a }) {
                    var cur = _terminals[i]["labels"] as? [String: String] ?? [:]
                    for (k, v) in l { cur[k] = v }
                    _terminals[i]["labels"] = cur
                }
            }
            return (reply(id: id, result: [String: Any]()), nil)
        case "doc.set", "terminal.kill", "terminal.rm", "terminal.label", "terminal.run", "server.stop":
            return (reply(id: id, result: nil, error: (-32000, "fake refuses \(method)", "refused")), nil)
        case "events.subscribe":
            let s = lock.withLock { subscribers.append(c); return seq }
            return (reply(id: id, result: ["seq": s, "terminals": terminals]), nil)
        case "terminal.read":
            let a = params["terminal"] as? String ?? ""
            guard let text = lock.withLock({ _screens[a] }) else { return (reply(id: id, result: nil, error: (-32000, "no terminal \(a)", "not_found")), nil) }
            let want = params["lines"] as? Int ?? 0
            let hist = lock.withLock { _histories[a] ?? "" }.split(separator: "\n", omittingEmptySubsequences: false).dropLast()
            let above = want > 0 ? hist.suffix(want).map { $0 + "\n" }.joined() : ""
            return (reply(id: id, result: ["text": above + text, "complete": true, "how": above.isEmpty ? "screen" : "history", "why": NSNull()]), nil)
        case "terminal.put":
            // As hc: the file lands in a folder of its own on the terminal's machine; the path as that machine sees it.
            let a = params["terminal"] as? String ?? ""
            guard terminals.contains(where: { $0["address"] as? String == a }) else { return (reply(id: id, result: nil, error: (-32000, "no terminal", "not_found")), nil) }
            let name = params["name"] as? String ?? "file"
            let data = Data(base64Encoded: params["data"] as? String ?? "") ?? Data()
            return (reply(id: id, result: ["path": "\(putDir)/\(a.replacingOccurrences(of: "/", with: "-"))/\(name)", "bytes": data.count]), nil)
        case "terminal.send", "terminal.keys":
            if method == "terminal.send", sendDelay > 0 { Thread.sleep(forTimeInterval: sendDelay) }
            if method == "terminal.keys", keysDelay > 0 { Thread.sleep(forTimeInterval: keysDelay) }
            if method == "terminal.keys", let cb = onKeys { cb(params["terminal"] as? String ?? "", params["keys"] as? [String] ?? []) }
            return (reply(id: id, result: ["turn": 0, "status": "idle", "started": true]), nil)
        case "terminal.attach":
            guard framed else { return (reply(id: id, result: nil, error: (-32000, "attach needs framed mode", "usage")), nil) }
            if attachDelay > 0 { Thread.sleep(forTimeInterval: attachDelay) }
            if let k = attachError { return (reply(id: id, result: nil, error: (-32000, k, k)), nil) }
            let a = params["terminal"] as? String ?? ""
            guard let t = terminals.first(where: { $0["address"] as? String == a }) else {
                return (reply(id: id, result: nil, error: (-32000, "no terminal \(a)", "not_found")), nil)
            }
            let ch = lock.withLock { () -> UInt32 in
                var ch = nextChannel
                if reuseChannels { ch = 1; while attachWriters[ch] != nil { ch += 1 } } else { nextChannel += 1 }
                attachWriters[ch] = (c, writeLock); connOf[ch] = (c, writeLock); return ch
            }
            let paint = firstPaint, burst = paintBurst
            return (reply(id: id, result: ["channel": ch, "terminal": t]), {
                writeLock.withLock {
                    Self.writeAll(c, HCFrame(channel: ch, kind: .data, payload: paint).encoded())
                    for i in 0..<burst { Self.writeAll(c, HCFrame(channel: ch, kind: .data, payload: Data("#\(i + 1);".utf8)).encoded()) }
                }
            })
        default:
            return (reply(id: id, result: nil, error: (-32601, "no method \(method)", "unknown_method")), nil)
        }
    }

    private func serve(_ c: Int32) {
        var buf = [UInt8]()
        var chunk = [UInt8](repeating: 0, count: 65536)
        var framed = false
        let writeLock = NSLock()
        var decoder = HCFrameDecoder()
        while true {
            let n = read(c, &chunk, chunk.count)
            if n <= 0 {
                lock.withLock {
                    open.removeAll { $0 == c }; subscribers.removeAll { $0 == c }
                    attachWriters = attachWriters.filter { $0.value.0 != c }; connOf = connOf.filter { $0.value.0 != c }
                }
                close(c)
                return
            }
            if framed {
                guard let frames = try? decoder.feed(chunk[..<n]) else { return }
                for f in frames { onFrame(f, c, writeLock) }
                continue
            }
            buf += chunk[..<n]
            while !framed, let nl = buf.firstIndex(of: 0x0A) {
                let line = Data(buf[..<nl]); buf.removeSubrange(...nl)
                guard let o = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { continue }
                let (out, after) = handle(o, conn: c, framed: false, writeLock: writeLock)
                writeLock.withLock { Self.writeAll(c, out + Data([0x0A])) }
                after?()
                if o["method"] as? String == "hello", (o["params"] as? [String: Any])?["framed"] as? Bool == true {
                    framed = true
                    if !buf.isEmpty, let frames = try? decoder.feed(buf[...]) { for f in frames { onFrame(f, c, writeLock) } }
                    buf = []
                }
            }
        }
    }

    private func onFrame(_ f: HCFrame, _ c: Int32, _ writeLock: NSLock) {
        switch (f.channel, f.kind) {
        case (0, .data):
            guard let o = try? JSONSerialization.jsonObject(with: f.payload) as? [String: Any] else { return }
            let (out, after) = handle(o, conn: c, framed: true, writeLock: writeLock)
            writeLock.withLock { Self.writeAll(c, HCFrame(channel: 0, kind: .data, payload: out).encoded()) }
            after?()
        case (let ch, .data):
            lock.withLock { _viewerBytes[ch, default: Data()].append(f.payload) }
        case (let ch, .credit):
            let v = f.payload.count >= 4 ? Int(f.payload.withUnsafeBytes { $0.loadUnaligned(as: UInt32.self) }.littleEndian) : 0
            lock.withLock { _credits[ch, default: 0] += v }
        case (let ch, .resize):
            let p = [UInt8](f.payload)
            if p.count >= 4 { lock.withLock { _resizes[ch, default: []].append((Int(p[0]) | Int(p[1]) << 8, Int(p[2]) | Int(p[3]) << 8)) } }
        case (let ch, .close):
            lock.withLock { _closedByViewer.insert(ch); attachWriters[ch] = nil }
            writeLock.withLock { Self.writeAll(c, HCFrame(channel: ch, kind: .close, payload: Data()).encoded()) }
        default: break
        }
    }
}
