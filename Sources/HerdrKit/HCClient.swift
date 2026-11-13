import Foundation

/// herdr-core (`hc`) client. Plain-mode JSON-RPC calls each on a short-lived connection; attach on a shared framed
/// connection (see `HCFramedConnection`). Layout writes (`HCLayout.swift`) change only what the person asked for.
public actor HCClient {
    public let session: HCSession
    private var seq = 0
    private var framed: HCFramedConnection?
    /// The open in progress: concurrent first attaches wait for it instead of opening their own.
    private var opening: Task<HCFramedConnection, Error>?
    /// Cached `herdr-app` document value. Refetched after a `doc` event, a `missed` snapshot or a reconnect.
    private var appDoc: HCJSON?
    private var docStale = true
    /// Each other connected machine's `herdr-app` document (`doc.get {on}`), as `machine.list` orders them. A remote
    /// document change brings no local `doc` event, so these are also re-read when older than `machineDocsTTL`.
    private var machineDocs: [(String, HCJSON)] = []
    private var machineDocsAt = Date.distantPast
    static let machineDocsTTL: TimeInterval = 2
    /// The background read of those documents, while it runs (a slow machine must not hold up a snapshot).
    private var machineDocsTask: Task<Void, Never>?
    /// Counts reads started; only the newest one's result is kept (a stale mark starts a new read while an
    /// old one, maybe on a dropped connection, still runs).
    private var machineDocsGen = 0
    /// Seconds one machine's `doc.get` may take; seconds a snapshot waits for a running read before it uses the last one.
    static let machineDocTimeout: TimeInterval = 3
    static let machineDocsWait: TimeInterval = 0.25
    /// The official app's command line to use instead of `HerdrAppCLI.find()` (tests). Without it, `herdr-app do`
    /// is used only on the default session, where the official app runs.
    var appCLI: HerdrAppCLI?
    public func setAppCLI(_ cli: HerdrAppCLI?) { appCLI = cli }
    /// A folder's git checkout top, nil outside one (`gitTopLevel`; checks swap it).
    var gitTop: @Sendable (String) -> String? = HCClient.gitTopLevel
    public func setGitTop(_ f: @escaping @Sendable (String) -> String?) { gitTop = f }

    /// Remote agent logs (`HCRemoteLogs.swift`): per pane, and the mirror pollers by machine + remote path.
    var remoteLogs: [String: RemoteLog] = [:]
    var mirrors: [String: Task<Void, Never>] = [:]
    var mirrorRoot = (NSSearchPathForDirectoriesInDomains(.cachesDirectory, .userDomainMask, true).first ?? NSTemporaryDirectory())
        + "/Flockbook/remote-logs"
    var mirrorPoll: TimeInterval = 1
    /// Seconds before a found remote log is looked for again.
    var remoteRecheck: TimeInterval = 30
    /// The mirror root was emptied (earlier runs' copies) and made owner-only in this client.
    var mirrorRootReady = false
    /// Per mirror path: what went wrong copying it (lines skipped, reads failing). `mirrorProblems()`.
    var mirrorIssues: [String: String] = [:]
    /// Other machines' homes (`$HOME` there), asked once.
    var machineHomes: [String: String] = [:]

    /// Seconds a plain call waits for its reply.
    public let callTimeout: TimeInterval
    /// Seconds `terminal.send` waits: the core answers only after nobody typed for ~3 s and the program went `working`.
    public let sendTimeout: TimeInterval

    /// Finds a wrapper-started agent's process for the key identity (issue #15); checks swap it.
    var wrapperCheck: HCAgentCheck = HCTreeCheck()
    public func setWrapperCheck(_ c: HCAgentCheck) { wrapperCheck = c }

    public init(session: HCSession, callTimeout: TimeInterval = 10, sendTimeout: TimeInterval = 60) {
        self.session = session; self.callTimeout = callTimeout; self.sendTimeout = sendTimeout
    }

    /// One plain-mode call on its own connection. Skips notifications until the reply with our id.
    public func call(_ method: String, _ params: [String: any Sendable] = [:]) async throws -> HCJSON {
        try await call(method, params, timeout: callTimeout)
    }

    /// `HerdrError.timeout` once the request was written: it may still take effect.
    func call(_ method: String, _ params: [String: any Sendable], timeout: TimeInterval) async throws -> HCJSON {
        try await call(method, json: HCJSON(params as [String: Any]), timeout: timeout)
    }

    /// The same, with any JSON params (a document value is not `Sendable`).
    func call(_ method: String, json params: HCJSON, timeout: TimeInterval? = nil) async throws -> HCJSON {
        let timeout = timeout ?? callTimeout
        seq += 1
        let id = seq
        let path = session.socketPath
        let body = try HCRPC.request(id: id, method: method, params: params.object)
        let line = try await Task.detached(priority: .userInitiated) { () throws -> Data in
            let sock = try LineSocket(path: path, timeout: timeout)
            defer { sock.close() }
            try sock.writeLine(body)
            while true {
                let line = try sock.readLine()
                if let o = try? JSONSerialization.jsonObject(with: line) as? [String: Any], HCRPC.id(of: o) == id { return line }
            }
        }.value
        guard let o = try JSONSerialization.jsonObject(with: line) as? [String: Any] else { throw HerdrError.protocolError("hc reply is not an object") }
        return try HCRPC.result(o)
    }

    /// The shared framed connection, opened on first use and reopened after it drops.
    public func framedConnection() async throws -> HCFramedConnection {
        if let f = framed, !f.isClosed { return f }
        if let t = opening { return try await t.value }
        let path = session.socketPath
        let t = Task { try await HCFramedConnection.open(socketPath: path) }
        opening = t
        defer { opening = nil }
        let f = try await t.value
        framed = f
        return f
    }

    public func attach(terminal: String, cols: Int, rows: Int, history: Int = 0, observe: Bool = false,
                       resumed: Bool = false) async throws -> HCAttachment {
        try await framedConnection().attach(terminal: terminal, cols: cols, rows: rows, history: history, observe: observe, resumed: resumed)
    }

    // MARK: - Snapshot

    public static let appDocName = "herdr-app"

    func markDocStale() { docStale = true }

    /// `terminal.list` + the `herdr-app` document (read only here), mapped by `HCMapper`.
    public func snapshot() async throws -> [Workspace] {
        let list = try await call("terminal.list")
        let terms = list["terminals"] as? [[String: Any]] ?? []
        let stale = docStale
        if docStale {
            docStale = false
            do { appDoc = HCJSON(["value": try await call("doc.get", ["name": Self.appDocName])["value"] ?? NSNull()]) }
            catch { appDoc = nil; docStale = true }   // names fall back to ids/folders until the next try
        }
        if stale || (machineDocsTask == nil && Date().timeIntervalSince(machineDocsAt) > Self.machineDocsTTL) {
            machineDocsAt = Date()
            machineDocsGen += 1
            let gen = machineDocsGen, previous = machineDocs
            machineDocsTask = Task { [weak self] in
                guard let self else { return }
                let docs = await self.readMachineDocs(previous: previous)
                await self.setMachineDocs(docs, gen: gen)
            }
        }
        // A quick read shows in this snapshot; a slow one in a later one (the store polls every 2 s).
        let deadline = Date().addingTimeInterval(Self.machineDocsWait)
        while machineDocsTask != nil, Date() < deadline { try? await Task.sleep(nanoseconds: 25_000_000) }
        let value = appDoc?["value"]
        let remote: [(String, Any?)] = machineDocs.map { m, d in let v = d["value"]; return (m, v is NSNull ? nil : v) }
        return HCMapper.workspaces(terminals: terms, appDoc: value is NSNull ? nil : value, machineDocs: remote, check: wrapperCheck)
    }

    /// The layout documents of the machines the official app shows: connected and not temporary (window.rs
    /// documents_updated). One that cannot be read keeps its `previous` document (a timeout must not
    /// regroup its tabs); a machine never read groups by labels. A failed `machine.list` keeps all of `previous`.
    /// All machines at once, each with a short timeout; the result keeps `machine.list` order.
    func readMachineDocs(previous: [(String, HCJSON)] = []) async -> [(String, HCJSON)] {
        guard let list = try? await call("machine.list", [:], timeout: Self.machineDocTimeout),
              let ms = list["machines"] as? [[String: Any]] else { return previous }
        let last = Dictionary(previous, uniquingKeysWith: { a, _ in a })
        let names = ms.compactMap { m -> String? in
            guard let name = m["name"] as? String, !name.isEmpty, m["state"] as? String == "connected",
                  m["temporary"] as? Bool != true else { return nil }
            return name
        }
        let got = await withTaskGroup(of: (Int, HCJSON?).self) { g in
            for (i, name) in names.enumerated() {
                g.addTask {
                    guard let r = try? await self.call("doc.get", ["name": Self.appDocName, "on": name], timeout: Self.machineDocTimeout)
                    else { return (i, nil) }
                    return (i, HCJSON(["value": r["value"] ?? NSNull()]))
                }
            }
            var out: [Int: HCJSON] = [:]
            for await (i, d) in g { if let d { out[i] = d } }
            return out
        }
        return names.indices.compactMap { i in (got[i] ?? last[names[i]]).map { (names[i], $0) } }
    }

    private func setMachineDocs(_ docs: [(String, HCJSON)], gen: Int) {
        guard gen == machineDocsGen else { return }   // an older read: a newer one runs
        machineDocs = docs
        machineDocsTask = nil
    }

    // MARK: - Events

    /// Plain-mode `events.subscribe`: yields once when subscribed, then once per event or `missed` snapshot
    /// (heartbeats are skipped). Doc events, `missed` and every (re)subscribe mark the document stale.
    nonisolated public func events() -> AsyncThrowingStream<Void, Error> {
        let path = session.socketPath
        return AsyncThrowingStream { cont in
            let task = Task.detached { [self] in
                do {
                    let sock = try LineSocket(path: path)
                    cont.onTermination = { _ in sock.close() }
                    try sock.writeLine(try HCRPC.request(id: 1, method: "events.subscribe", params: [:]))
                    while true {
                        let line = try sock.readLine()
                        guard let o = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { continue }
                        if HCRPC.id(of: o) == 1 { _ = try HCRPC.result(o); break }
                    }
                    await self.markDocStale()
                    cont.yield(())
                    while !Task.isCancelled {
                        let line = try sock.readLine()
                        guard let o = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { continue }
                        switch o["method"] as? String {
                        case "event":
                            if (o["params"] as? [String: Any])?["type"] as? String == "doc" { await self.markDocStale() }
                            cont.yield(())
                        case "missed":
                            await self.markDocStale()
                            cont.yield(())
                        default: break
                        }
                    }
                    cont.finish()
                } catch {
                    cont.finish(throwing: error)
                }
            }
            let prev = cont.onTermination
            cont.onTermination = { reason in prev?(reason); task.cancel() }
        }
    }

    // MARK: - Input

    func address(_ paneId: String) throws -> String {
        guard let a = HCMapper.address(fromPaneId: paneId), !a.isEmpty else { throw HerdrError.protocolError("not an hc pane: \(paneId)") }
        return a
    }

    /// `terminal.send`: pastes `text` and presses Enter once nobody has typed for ~3 s.
    /// Throws `HerdrError.timeout` when no answer came within `sendTimeout` (the text may still arrive).
    public func send(text: String, to paneId: String) async throws {
        _ = try await call("terminal.send", ["terminal": try address(paneId), "text": text], timeout: sendTimeout)
    }

    /// `send`, as the composer needs it: a timeout is `unconfirmed` (do not give the text back), anything else that
    /// throws is `failed`.
    public func deliver(text: String, to paneId: String) async -> DeliveryOutcome {
        do { try await send(text: text, to: paneId); return .sent }
        catch HerdrError.timeout { return .unconfirmed }
        catch { return .failed }
    }

    /// `deliver` with pasted files named in `text` (local paths). A terminal on another machine cannot read them:
    /// each is first put beside it (`terminal.put`, on its machine) and the text names that copy. A file that cannot
    /// be read or put fails before anything is sent.
    public func deliver(text: String, to paneId: String, files: [String]) async -> DeliveryOutcome {
        guard let typed = await placed(text: text, to: paneId, files: files) else { return .failed }
        return await deliver(text: typed, to: paneId)
    }

    /// A paired device's send: the files first (on another machine each is a call there), then the key gate on a
    /// fresh read, then the text in one `terminal.send`. Unconfirmed counts as sent: the core may still type it.
    public func sendFromDevice(text: String, to paneId: String, files: [String], gate: KeyGate) async -> HubSendResult {
        guard let typed = await placed(text: text, to: paneId, files: files) else { return .failed }
        guard await passes(gate, paneId: paneId) else { return .changed }
        return await deliver(text: typed, to: paneId) == .failed ? .failed : .sent
    }

    /// `text` with each pasted file put beside a terminal on another machine and named by that copy; as it is for
    /// a terminal here. Nil when a file cannot be read or put (nothing was sent).
    func placed(text: String, to paneId: String, files: [String]) async -> String? {
        guard let a = try? address(paneId) else { return nil }
        var text = text
        if !HCMapper.machinePrefix(a).isEmpty {
            for f in files where text.contains(f) {
                guard let data = FileManager.default.contents(atPath: f),
                      let there = try? await put(paneId: paneId, name: (f as NSString).lastPathComponent, data: data) else { return nil }
                // The copy's path as the composer writes paths; a local path already in quotes (or in
                // the composer's quoting) is swapped whole, so the quotes do not double.
                let q = ShellQuote.quotedIfNeeded(there)
                for form in ["'" + f + "'", ShellQuote.quotedIfNeeded(f)] where form != f { text = text.replacingOccurrences(of: form, with: q) }
                text = text.replacingOccurrences(of: f, with: q)
            }
        }
        return text
    }

    /// `terminal.put`: a file beside the terminal, on its machine (removed with the terminal). The path there.
    public func put(paneId: String, name: String, data: Data) async throws -> String {
        let r = try await call("terminal.put", ["terminal": try address(paneId), "name": name, "data": data.base64EncodedString()])
        guard let p = r["path"] as? String, !p.isEmpty else { throw HerdrError.protocolError("terminal.put gave no path") }
        return p
    }

    /// Paths in `text` of files under `dir` (pasted images), in order, each once. A path ends at whitespace or a quote.
    public static func localFiles(in text: String, under dir: String) -> [String] {
        let base = dir.hasSuffix("/") ? dir : dir + "/"
        var out: [String] = []
        var rest = text[...]
        while let r = rest.range(of: base) {
            let tail = rest[r.upperBound...]
            let end = tail.firstIndex { $0.isWhitespace || $0 == "'" || $0 == "\"" } ?? tail.endIndex
            let p = String(rest[r.lowerBound..<end])
            if end > tail.startIndex, !out.contains(p) { out.append(p) }
            rest = rest[end...]
        }
        return out
    }

    /// Raw bytes go to the terminal as named keys where they are control keys, else as pasted text without Enter.
    /// hc has no raw-text verb outside attach; `HCKeys.names(for:)` turns the bytes into key names.
    public func sendKeys(_ bytes: [UInt8], to paneId: String) async throws {
        let names = HCKeys.names(for: bytes)
        guard !names.isEmpty else { return }
        _ = try await call("terminal.keys", ["terminal": try address(paneId), "keys": names])
    }

    /// `terminal.read` with no history: the visible screen as text. Read-only.
    public func visibleText(paneId: String) async throws -> String {
        let r = try await call("terminal.read", ["terminal": try address(paneId)])
        guard let text = r["text"] as? String else { throw HerdrError.protocolError("terminal.read has no text") }
        return text
    }

    /// Claude's permission prompt: `1` allows once, `esc` denies (same keys as the TUI path, `ApprovalKeys`).
    public func respond(to paneId: String, allow: Bool) async throws {
        _ = try await call("terminal.keys", ["terminal": try address(paneId), "keys": [allow ? ApprovalKeys.allowOnce : HCKeys.translate(ApprovalKeys.deny)]])
    }

    /// herdr key names ("Enter", "Down", "C-c") translated to hc's ("enter", "down", "ctrl+c").
    public func sendNamedKeys(_ keys: [String], to paneId: String) async throws {
        _ = try await call("terminal.keys", ["terminal": try address(paneId), "keys": keys.map(HCKeys.translate)])
    }
}

extension HCClient: HerdrBackend {
    nonisolated public var backend: BackendKind { .hc }
    nonisolated public var supportsLayout: Bool { true }
}

/// Key names for hc's `terminal.keys` ("enter, esc, tab, up, ctrl+c, alt+x, f5, a, ...").
public enum HCKeys {
    /// herdr / tmux spelling → hc spelling. Unknown names pass through lowercased.
    public static func translate(_ k: String) -> String {
        if k.count == 1 { return k }                    // a literal character keeps its case
        let l = k.lowercased()
        let map: [String: String] = ["escape": "esc", "return": "enter", "bspace": "backspace", "btab": "shift+tab",
                                     "pageup": "pageup", "pagedown": "pagedown", "ppage": "pageup", "npage": "pagedown", "dc": "delete"]
        if let m = map[l] { return m }
        for (p, r) in [("c-", "ctrl+"), ("m-", "alt+"), ("s-", "shift+")] where l.hasPrefix(p) {
            return r + translate(String(k.dropFirst(2)))
        }
        return l
    }

    /// Bytes → key names: CR/LF enter, ESC esc, tab, DEL backspace, 0x01–0x1A ctrl+letter, arrow escape sequences,
    /// every other character as itself.
    public static func names(for bytes: [UInt8]) -> [String] {
        var out: [String] = []
        let s = String(decoding: bytes, as: UTF8.self)
        var chars = Array(s.unicodeScalars)[...]
        while let c = chars.first {
            if c == "\u{1b}", chars.count >= 3, chars[chars.startIndex + 1] == "[" {
                let code = chars[chars.startIndex + 2]
                let arrow: [Unicode.Scalar: String] = ["A": "up", "B": "down", "C": "right", "D": "left", "H": "home", "F": "end", "Z": "shift+tab"]
                if let a = arrow[code] { out.append(a); chars = chars.dropFirst(3); continue }
            }
            chars = chars.dropFirst()
            switch c.value {
            case 0x0D, 0x0A: out.append("enter")
            case 0x1B: out.append("esc")
            case 0x09: out.append("tab")
            case 0x7F, 0x08: out.append("backspace")
            case 0x01...0x1A: out.append("ctrl+" + String(Character(Unicode.Scalar(c.value + 0x60)!)))
            case 0x20: out.append("space")
            default: out.append(String(Character(c)))
            }
        }
        return out
    }
}
