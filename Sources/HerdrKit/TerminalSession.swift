import Foundation

public enum TerminalEvent: Sendable {
    /// First frame after (re)connect is `full`; later ones are increments (or a full redraw after a resize).
    case frame(TerminalFrame)
    /// Final: the stream ends after this. Reasons seen: `detached`, `terminal attach taken over`,
    /// `terminal attach failed: … already has an attached client …`, `… not found`.
    case closed(reason: String)
    /// The process ended without a closed record; a new one starts after a short delay.
    case reconnecting(attempt: Int)
    /// A line from stderr, e.g. an ignored input.
    case notice(String)
    /// Final: the terminal could not start (no herdr program, or herdr exited 127 before any frame). Why, in words.
    case failed(reason: String)
}

/// One long-lived `herdr terminal session control|observe <terminal_id>` process. Input, resize and frames share its
/// pipes, so a key press is one small pipe write (no socket per key). Control mode owns the terminal's input and size
/// (herdr allows one controller; `takeover` steals it). Observe mode is read-only.
/// For a session on a host (`HerdrSession.remote`) the process is the host's herdr through `RemoteHost.process`: the same
/// JSON lines on stdio, so frames and input do not change.
public final class TerminalSession: @unchecked Sendable {
    public enum Mode: Sendable, Equatable { case observe, control(takeover: Bool) }
    /// The host command line for herdr session `session` (nil: default) and herdr's words `args`. Checks replace it.
    public typealias RemoteCommand = @Sendable (_ session: String?, _ args: [String]) -> String

    public let terminalId: String
    public let mode: Mode
    /// Frames and state changes. Finishes after `closed`, `release()` or `stop()`.
    public let events: AsyncStream<TerminalEvent>

    private let cont: AsyncStream<TerminalEvent>.Continuation
    /// The program and its first arguments; nil: herdr's, found when each process starts (`find`).
    private let launcher: [String]?
    private let cliPrefix: [String]
    private let find: @Sendable () -> String?
    private let stripHerdrEnv: Bool
    private let sessionName: String?
    private let host: RemoteHost?
    private let remoteCommand: RemoteCommand
    private let lock = NSLock()
    private var proc: Process?
    private var stdin: FileHandle?
    private var started = false
    private var stopped = false
    private var wantCols: Int?
    private var wantRows: Int?
    private var lastSize: (cols: Int, rows: Int)?
    private var showed = false        // a frame came in some run: from then on a missing herdr is a moment (an upgrade), not final
    /// herdr-core mode: the stream is a `terminal.attach` channel instead of the herdr CLI process.
    private var hcClient: HCClient?
    private var attachment: HCAttachment?
    /// hc: a control session watches (observe attach, never resizes the user's terminal) until the user types. The
    /// first keys switch it to a controller: the watching channel closes, a controller attach opens with the view's
    /// size, then the keys go out. `controlling` stays on for the session (reconnects come back `resumed`).
    private var controlling = false
    private var attachIsController = false
    private var upgrading = false
    private var pendingInput: [(data: Data, at: Date)] = []
    private var overflowNoted = false
    /// The queue overflowed during this upgrade: it was dropped, and keys typed until the upgrade ends are dropped too
    /// (a line must go whole or not at all).
    private var pendingDropped = false
    /// Keys queued while a controller attach opens: dropped (and reported) once older than this, and beyond `pendingCap`.
    var pendingWindow: TimeInterval = 5
    var pendingCap = 256
    private var bracketed = true      // herdr frames do not carry DEC private modes (see README); assume yes until a frame says ?2004l

    /// `launcher` replaces `<herdr> --session <name>` (tests use a fake script). `find` gives herdr's full path.
    /// A remote session (`session.remote`) uses neither: it runs `remoteCommand` on the host (checks pass a fake herdr).
    public init(terminalId: String, session: HerdrSession, mode: Mode = .control(takeover: false),
                cols: Int? = nil, rows: Int? = nil, launcher: [String]? = nil, find: @escaping @Sendable () -> String? = { ConnectionCheck.findHerdr() },
                remoteCommand: RemoteCommand? = nil) {
        self.terminalId = terminalId
        self.mode = mode
        self.launcher = launcher
        self.cliPrefix = session.cliPrefix
        self.find = find
        self.stripHerdrEnv = session.name != nil
        self.sessionName = session.name
        self.host = session.remote
        self.remoteCommand = remoteCommand ?? { s, a in TerminalSession.remoteCommand(session: s, a) }
        self.wantCols = cols.flatMap { $0 > 0 ? $0 : nil }
        self.wantRows = rows.flatMap { $0 > 0 ? $0 : nil }
        (events, cont) = AsyncStream.makeStream(of: TerminalEvent.self, bufferingPolicy: .unbounded)
        signal(SIGPIPE, SIG_IGN)
        cont.onTermination = { [weak self] _ in self?.stop() }
    }

    /// herdr-core terminal (`t7`, `machine/t7`): the same events over an attach stream. The first chunk is a full frame
    /// (the core's first paint) with the pty size; later chunks are increments. `observe` attaches watch-only.
    public convenience init(hcTerminal address: String, client: HCClient, mode: Mode = .control(takeover: false),
                            cols: Int? = nil, rows: Int? = nil) {
        self.init(terminalId: address, session: .named("hc"), mode: mode, cols: cols, rows: rows, launcher: [])
        self.hcClient = client
    }

    deinit { stop() }

    /// Current pty size as last reported by herdr (nil before the first frame).
    public var size: (cols: Int, rows: Int)? { lock.withLock { lastSize } }
    /// Whether `paste` wraps the text in bracketed-paste markers. herdr's frames normally do not show the program's mode, so
    /// this stays true unless a frame carries `ESC[?2004l` (then false until `ESC[?2004h`). Override with `paste(_:bracketed:)`.
    public var bracketedPaste: Bool { lock.withLock { bracketed } }

    /// herdr's full path and the session's arguments; nil when there is no herdr program.
    static func launcher(session: HerdrSession, environment: [String: String], home: String) -> [String]? {
        ConnectionCheck.findHerdr(environment: environment, home: home).map { [$0] + session.cliPrefix }
    }
    /// herdr's words after the program (and `--session`): mode, terminal id, `--takeover`, size.
    static func arguments(terminalId: String, mode: Mode, cols: Int?, rows: Int?) -> [String] {
        var args = ["terminal", "session", mode == .observe ? "observe" : "control", terminalId]
        if case .control(let takeover) = mode, takeover { args.append("--takeover") }
        if let cols { args += ["--cols", String(cols)] }
        if let rows { args += ["--rows", String(rows)] }
        return args
    }

    /// The host command line: `RemoteHost.herdrCommand` finds herdr there and quotes every word.
    public static func remoteCommand(session: String?, _ args: [String]) -> String { RemoteHost.herdrCommand(session: session, args) }

    /// Why `id` is not a terminal id to pass on; nil: fine. herdr's ids are short words such as `term_a` or `w1:p2`;
    /// a leading "-" (an option), white space and shell characters are refused, even though every word is quoted.
    public static func idProblem(_ id: String, host: RemoteHost?) -> String? {
        guard host != nil else { return nil }   // on this Mac the id is one argv word: no shell reads it
        let ok = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_:.-")
        if id.isEmpty || id.utf8.count > 128 || id.hasPrefix("-") || !id.unicodeScalars.allSatisfy(ok.contains) {
            return "not a herdr terminal id: \(id.debugDescription.prefix(40))"
        }
        return nil
    }

    /// A failed host run before any frame, in plain words: nil when it is not an ssh or missing-herdr problem.
    /// `final`: retrying will not help (no herdr, login refused, host key unknown or changed); otherwise the host may come back.
    /// ssh exits 255 for its own failures. (Small on purpose; the API relay has the full classifier.)
    static func remoteProblem(status: Int32, stderr: String, host: String) -> (reason: String, final: Bool)? {
        if status == 127 && stderr.contains(RemoteHost.herdrMissingMarker) { return ("herdr is not installed on \(host).", true) }
        guard status == 255 else { return nil }
        let e = stderr.lowercased()
        if e.contains("host key verification failed") || e.contains("remote host identification has changed") || e.contains("host key is known") {
            return ("The host key of \(host) is unknown or has changed. Run `ssh \(host)` once in Terminal to check it.", true)
        }
        if e.contains("permission denied") || e.contains("too many authentication failures") {
            return ("ssh could not log in to \(host). Add your key with `ssh-add`, or run `ssh \(host)` once in Terminal.", true)
        }
        if e.contains("could not resolve hostname") { return ("Can't find the host \(host).", false) }
        for s in ["connection refused", "timed out", "no route to host", "network is unreachable", "connection closed", "connection reset"] where e.contains(s) {
            return ("Can't reach \(host) (\(s)).", false)
        }
        let last = stderr.split(whereSeparator: \.isNewline).last.map(String.init) ?? "exit 255"
        return ("ssh to \(host) failed: \(last)", false)
    }

    /// `PATH` for the herdr process: the app's, then the usual install folders it lacks.
    static func childPath(environment: [String: String], home: String) -> String {
        let have = (environment["PATH"] ?? "").split(separator: ":").map(String.init)
        return (have + ConnectionCheck.usualFolders(home: home).filter { !have.contains($0) }).joined(separator: ":")
    }

    public func start() {
        let go = lock.withLock { () -> Bool in defer { started = true }; return !started && !stopped }
        guard go else { return }
        if let hc = hcClient {
            Task.detached { [self] in await runHC(hc) }
            return
        }
        Task.detached { [self] in
            let attempt = Locked(0)
            while !lock.withLock({ stopped }) {
                let outcome = await runOnce(onFrame: { attempt.set(0) })
                if outcome == .final || lock.withLock({ stopped }) { break }
                let n = attempt.update { $0 += 1; return $0 }
                cont.yield(.reconnecting(attempt: n))
                try? await Task.sleep(for: .milliseconds(min(2000, 250 << min(n - 1, 3))))
            }
            cont.finish()
        }
    }

    // MARK: input (control mode)

    /// Raw bytes to the program: text, control keys, escape sequences, any UTF-8. False when not connected or observing.
    @discardableResult
    public func send(bytes: [UInt8]) -> Bool {
        guard !bytes.isEmpty else { return true }
        if hcClient != nil {
            guard mode != .observe else { return false }
            let d = Data(bytes)
            enum Step { case send(HCAttachment), upgrade(HCAttachment?), queued, overflow(first: Bool), refuse }
            let step = lock.withLock { () -> Step in
                if stopped { return .refuse }
                // While the controller attach opens (and until its queue is flushed), keys queue: order is kept.
                if upgrading {
                    guard !pendingDropped, pendingInput.count < pendingCap else {
                        pendingInput = []; pendingDropped = true
                        defer { overflowNoted = true }; return .overflow(first: !overflowNoted)
                    }
                    pendingInput.append((d, Date())); return .queued
                }
                if attachIsController, let a = attachment { return .send(a) }
                if controlling { return .refuse }   // a controller between reconnects: keys would land late
                // Focus and mouse reports come from looking, not typing: they never take the terminal over.
                if Self.isReportOnly(bytes) { return .refuse }
                controlling = true; upgrading = true; pendingInput = [(d, Date())]; overflowNoted = false; pendingDropped = false
                defer { attachment = nil }
                return .upgrade(attachment)
            }
            switch step {
            case .send(let a): a.send(d); return true
            case .upgrade(let old): old?.close(); return true   // runHC reattaches as a controller and flushes
            case .queued: return true
            case .overflow(let first):
                if first { cont.yield(.notice("Some keys were not sent: the terminal was still connecting.")) }
                return false
            case .refuse: return false
            }
        }
        return writeJSON(["type": "terminal.input", "bytes": Data(bytes).base64EncodedString()])
    }

    @discardableResult
    public func send(text: String) -> Bool { send(bytes: Array(text.utf8)) }

    /// Bytes that are only terminal reports the view sends by itself: focus in/out (`CSI I`, `CSI O`), X10 mouse
    /// (`CSI M` + 3 bytes), SGR mouse (`CSI < n;n;n M|m`). Keys, text and paste are not.
    public static func isReportOnly(_ bytes: [UInt8]) -> Bool {
        var i = 0
        func at(_ k: Int) -> UInt8? { i + k < bytes.count ? bytes[i + k] : nil }
        while i < bytes.count {
            guard at(0) == 0x1B, at(1) == 0x5B, let c = at(2) else { return false }
            switch c {
            case 0x49, 0x4F: i += 3                                   // I, O
            case 0x4D: guard i + 6 <= bytes.count else { return false }; i += 6   // M b x y
            case 0x3C:                                                // < digits ; digits ; digits M|m
                var j = i + 3
                while j < bytes.count, (0x30...0x39).contains(bytes[j]) || bytes[j] == 0x3B { j += 1 }
                guard j < bytes.count, j > i + 3, bytes[j] == 0x4D || bytes[j] == 0x6D else { return false }
                i = j + 1
            default: return false
            }
        }
        return !bytes.isEmpty
    }

    /// Pastes text. Wrapped in bracketed paste (default: `bracketedPaste`); otherwise newlines become CR.
    @discardableResult
    public func paste(_ text: String, bracketed: Bool? = nil) -> Bool {
        guard mode != .observe else { return false }
        if bracketed ?? bracketedPaste { return send(text: "\u{1b}[200~" + text + "\u{1b}[201~") }
        return send(text: text.replacingOccurrences(of: "\r\n", with: "\r").replacingOccurrences(of: "\n", with: "\r"))
    }

    /// Resizes the pty (control mode). Kept for reconnects. Ignores values below 1.
    public func resize(cols: Int, rows: Int) {
        guard cols > 0, rows > 0 else { return }
        lock.withLock { wantCols = cols; wantRows = rows }
        if hcClient != nil {
            // Only a controller sets the size; a watcher just remembers it for the controller attach.
            if mode != .observe, let a = lock.withLock({ attachIsController ? attachment : nil }) { a.resize(cols: cols, rows: rows) }
            return
        }
        writeJSON(["type": "terminal.resize", "cols": cols, "rows": rows])
    }

    public func scroll(lines: Int, up: Bool) { wheel(lines: lines, up: up) }

    /// One wheel step for the terminal, control mode only (observe returns false and sends nothing). herdr decides where it goes
    /// from the program's current mode, so the app must not turn it into keys itself:
    /// - program tracks the mouse (`?1000`/`?1002`/`?1003`, SGR or legacy encoding): one wheel event (button 64 up / 65 down,
    ///   position 1;1) reaches the program;
    /// - alternate screen with alternate scroll (`?1007`): one Up/Down arrow;
    /// - alternate screen with neither: nothing;
    /// - main screen: herdr scrolls its own history by `lines` (shared pane position, see `HerdrClient.setScrollOffset`).
    /// A forwarded step is **one event per call**, whatever `lines` is. For a program, pass `eventPerLine: true` to send
    /// `lines` separate steps. The position cannot be chosen (herdr always uses 1;1).
    @discardableResult
    public func wheel(lines: Int, up: Bool, eventPerLine: Bool = false) -> Bool {
        guard mode != .observe, lines > 0 else { return false }
        // hc streams raw bytes and never encodes input; the app cannot see the program's mouse mode, so send nothing.
        if hcClient != nil { return false }
        let dir = up ? "up" : "down"
        if eventPerLine {
            var ok = true
            for _ in 0..<lines { ok = writeJSON(["type": "terminal.scroll", "lines": 1, "direction": dir]) && ok }
            return ok
        }
        return writeJSON(["type": "terminal.scroll", "lines": lines, "direction": dir])
    }

    public enum MouseEncoding: Sendable { case sgr, legacy }

    /// Right click (press and release) at a cell, 1-based `col`/`row` of the terminal grid. Control mode only. herdr 0.9.1 has no
    /// `terminal.mouse`, so this sends the xterm mouse bytes as plain input: `ESC[<2;col;rowM` + `ESC[<2;col;rowm` (SGR, `?1006`)
    /// or `ESC[M` + 3 bytes (legacy, `?1000` only; col and row must be <= 223). The app cannot read the program's mouse mode, so the
    /// caller chooses: a program that did not enable mouse tracking will print these bytes as text. Use it only when the user
    /// turned "Send right-clicks to pane" on. Returns false in observe mode or for an invalid cell.
    @discardableResult
    public func rightClick(col: Int, row: Int, encoding: MouseEncoding = .sgr) -> Bool {
        guard mode != .observe, col >= 1, row >= 1 else { return false }
        switch encoding {
        case .sgr:
            return send(text: "\u{1b}[<2;\(col);\(row)M\u{1b}[<2;\(col);\(row)m")
        case .legacy:
            guard col <= 223, row <= 223 else { return false }
            let c = UInt8(32 + col), r = UInt8(32 + row)
            return send(bytes: [0x1b, 0x5b, 0x4d, 34, c, r, 0x1b, 0x5b, 0x4d, 35, c, r])
        }
    }

    /// Gives the input back (the pty returns to the other client's size) and ends the stream.
    public func release() {
        if hcClient != nil { stop(); return }   // CLOSE detaches; the core keeps the program running
        lock.withLock { stopped = true }
        writeJSON(["type": "terminal.release"])
        // the process answers closed(detached) and exits; stop() is the fallback if it does not
        Task.detached { [self] in try? await Task.sleep(for: .seconds(2)); stop() }
    }

    /// Ends the session without telling herdr (the process is terminated).
    public func stop() {
        let (p, att) = lock.withLock { () -> (Process?, HCAttachment?) in
            stopped = true; defer { proc = nil; stdin = nil; attachment = nil }; return (proc, attachment)
        }
        att?.close()
        if let p, p.isRunning { p.terminate() }
        cont.finish()
    }

    var processIdentifier: Int32? { lock.withLock { proc?.processIdentifier } }

    func sendRaw(_ line: String) { _ = write(Data((line + "\n").utf8)) }

    @discardableResult
    private func writeJSON(_ o: [String: Any]) -> Bool {
        guard mode != .observe, let d = try? JSONSerialization.data(withJSONObject: o, options: [.withoutEscapingSlashes]) else { return false }
        return write(d + Data([0x0A]))
    }

    private func write(_ d: Data) -> Bool {
        guard let h = lock.withLock({ stdin }) else { return false }
        return (try? h.write(contentsOf: d)) != nil
    }

    // MARK: process

    private enum Outcome { case final, retry }

    /// CLOSE reasons after which the terminal is gone for good. Anything else (a kept terminal lost, waiting for its
    /// new shell; a dropped connection) is retried with backoff, like the herdr CLI channel.
    /// attach error kinds that a retry cannot fix (the terminal is gone or ended, the machine is unknown, the connection
    /// is not framed). `unavailable` and dropped connections are retried.
    static let hcFinalErrors: Set<String> = ["not_found", "usage", "finished", "machine_unknown", "needs_framed"]
    static let hcFinalReasons: Set<String> = ["exited", "killed", "server_stop", "host_shutdown", "holder_lost", "stopped_by_restart"]

    /// Sends the keys queued during the upgrade, oldest first, and only then lets new keys go straight to the channel
    /// (`upgrading` stays on until the queue is empty, so a key typed meanwhile queues behind them). Keys older than
    /// `pendingWindow` are dropped (a long failed upgrade must not replay old Enter presses) and reported.
    /// Dropping is all or nothing: once one queued key expired (or the queue overflowed), the whole queue goes, so a
    /// slowly typed line never loses its start and still sends the rest plus Enter.
    private func flushPending(_ att: HCAttachment) {
        var dropping = false, expired = false
        while true {
            let (batch, overflowed) = lock.withLock { () -> ([(data: Data, at: Date)], Bool) in
                defer { pendingInput = [] }
                let o = pendingDropped
                if pendingInput.isEmpty { upgrading = false; pendingDropped = false }
                return (pendingInput, o)
            }
            if overflowed { dropping = true }   // already reported when it overflowed
            if batch.isEmpty { break }
            let now = Date()
            if !dropping, batch.contains(where: { now.timeIntervalSince($0.at) > pendingWindow }) { dropping = true; expired = true }
            if !dropping { for k in batch { att.send(k.data) } }
        }
        if expired { cont.yield(.notice("Keys typed while the terminal was connecting were not sent (waited too long).")) }
    }

    private func runHC(_ hc: HCClient) async {
        var attempt = 0
        var seq = 0
        var resumed = false   // the last controller attach dropped: come back resumed
        while !lock.withLock({ stopped }) {
            var final: String?
            var wasUpgrade = false
            do {
                let (c, r, control) = lock.withLock { (wantCols ?? 80, wantRows ?? 24, controlling) }
                let att = try await hc.attach(terminal: terminalId, cols: c, rows: r, observe: !control,
                                              resumed: control && resumed)
                let stop = lock.withLock { () -> Bool in
                    if stopped { return true }
                    // Keys typed while this (watching) attach was in flight: take control right away.
                    if !control && upgrading { return false }
                    attachment = att; attachIsController = control; lastSize = (att.cols, att.rows)
                    return false
                }
                if stop { att.close(); break }
                if !control && lock.withLock({ upgrading }) { att.close(); continue }
                if control { flushPending(att) }
                var first = true
                for try await chunk in att.bytes {
                    attempt = 0
                    seq += 1
                    lock.withLock {
                        if let on = chunk.range(of: Data("\u{1b}[?2004h".utf8), options: .backwards),
                           chunk.range(of: Data("\u{1b}[?2004l".utf8), options: .backwards).map({ $0.lowerBound < on.lowerBound }) ?? true {
                            bracketed = true
                        } else if chunk.range(of: Data("\u{1b}[?2004l".utf8)) != nil { bracketed = false }
                    }
                    let n = chunk.count
                    cont.yield(.frame(TerminalFrame(seq: seq, full: first, width: first ? att.cols : 0, height: first ? att.rows : 0,
                                                    bytes: chunk, ack: FrameAck { att.consumed(n) })))
                    first = false   // CREDIT goes out when the view calls frame.consumed()
                }
                wasUpgrade = lock.withLock { () -> Bool in
                    if attachment === att { attachment = nil; attachIsController = false }
                    return upgrading && !control
                }
                if control { resumed = true }   // a controller that comes back is a reconnect
                if let reason = att.closeReason?["reason"] as? String, Self.hcFinalReasons.contains(reason) { final = reason }
            } catch HerdrError.server(let code, let message) where Self.hcFinalErrors.contains(code) {
                final = code == "not_found" || code == "usage" ? "not found: \(message)" : "\(code): \(message)"
            } catch {}
            if let final { cont.yield(.closed(reason: final)); break }
            if lock.withLock({ stopped }) { break }
            if wasUpgrade { continue }   // the watcher closed to make room for the controller: no backoff
            attempt += 1
            cont.yield(.reconnecting(attempt: attempt))
            try? await Task.sleep(for: .milliseconds(min(2000, 250 << min(attempt - 1, 3))))
        }
        cont.finish()
    }

    private func runOnce(onFrame: @escaping @Sendable () -> Void) async -> Outcome {
        let (cols, rows) = lock.withLock { () -> (Int?, Int?) in (wantCols, wantRows) }
        if let why = Self.idProblem(terminalId, host: host) { cont.yield(.failed(reason: why)); return .final }
        let words = Self.arguments(terminalId: terminalId, mode: mode, cols: cols, rows: rows)
        let p: Process
        if let host {
            // the host's herdr; ssh (or sh for checks) and the env come from RemoteHost
            do { p = try host.process(self.remoteCommand(sessionName, words)) } catch {
                cont.yield(.failed(reason: "cannot start the terminal on \(host.label): \(error)"))
                return .final
            }
        } else {
            guard let argvPrefix = launcher ?? find().map({ [$0] + cliPrefix }) else {
                if lock.withLock({ showed }) { cont.yield(.notice(ConnectionCheck.notFound)); return .retry }
                cont.yield(.failed(reason: ConnectionCheck.notFound))
                return .final
            }
            p = Process()
            p.executableURL = URL(fileURLWithPath: argvPrefix[0])
            p.arguments = Array(argvPrefix.dropFirst()) + words
            var env = ProcessInfo.processInfo.environment
            if stripHerdrEnv { for k in env.keys where k.hasPrefix("HERDR_") { env[k] = nil } }
            env["PATH"] = Self.childPath(environment: env, home: NSHomeDirectory())
            p.environment = env
        }
        let onHost = host
        let out = Pipe(), err = Pipe(), inp = Pipe()
        p.standardOutput = out; p.standardError = err; p.standardInput = inp

        let finalReason = Locked<String?>(nil)
        // Exit 127 before any frame: the program is not there (`env: herdr: …`) or cannot run. Final, with its last words.
        let framed = Locked(false), lastErr = Locked<String?>(nil), status = Locked<Int32>(0)
        // a host run keeps all of stderr (bounded): ssh's reason is not always its last line
        let allErr = Locked("")
        let outReader = LineAccumulator { [self] line in
            guard let o = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { return }
            switch o["type"] as? String {
            case "terminal.frame":
                let bytes = Data(base64Encoded: o["bytes"] as? String ?? "") ?? Data()
                let f = TerminalFrame(seq: o["seq"] as? Int ?? 0, full: o["full"] as? Bool ?? false,
                                      width: o["width"] as? Int ?? 0, height: o["height"] as? Int ?? 0, bytes: bytes)
                lock.withLock {
                    if f.width > 0, f.height > 0 { lastSize = (f.width, f.height) }
                    if let on = bytes.range(of: Data("\u{1b}[?2004h".utf8), options: .backwards),
                       bytes.range(of: Data("\u{1b}[?2004l".utf8), options: .backwards).map({ $0.lowerBound < on.lowerBound }) ?? true {
                        bracketed = true
                    } else if bytes.range(of: Data("\u{1b}[?2004l".utf8)) != nil { bracketed = false }
                }
                framed.set(true)
                lock.withLock { showed = true }
                onFrame()
                cont.yield(.frame(f))
            case "terminal.closed":
                let r = o["reason"] as? String ?? ""
                finalReason.set(r)
                cont.yield(.closed(reason: r))
            default: break
            }
        }
        let errReader = LineAccumulator { [cont] line in
            if let s = String(data: line, encoding: .utf8), !s.isEmpty {
                lastErr.set(s)
                if onHost != nil { allErr.update { if $0.utf8.count < 8192 { $0 += s + "\n" } } }
                cont.yield(.notice(s))
            }
        }
        // three things must finish: stdout EOF, stderr EOF (herdr's last words are all read) and process exit
        return await withCheckedContinuation { (k: CheckedContinuation<Outcome, Never>) in
            let pending = Locked(3)
            let done: @Sendable () -> Void = { [self] in
                guard pending.update({ $0 -= 1; return $0 }) == 0 else { return }
                lock.withLock { proc = nil; stdin = nil }
                // a host run that ended before any frame: ssh or the host side in plain words (a missing herdr after a
                // frame is a moment, as on this Mac)
                if let onHost, finalReason.get() == nil, !framed.get(),
                   let problem = Self.remoteProblem(status: status.get(), stderr: allErr.get(), host: onHost.label) {
                    if problem.final && !lock.withLock({ showed }) {
                        finalReason.set(problem.reason)
                        cont.yield(.failed(reason: problem.reason))
                    } else {
                        cont.yield(.notice(problem.reason))
                    }
                }
                if finalReason.get() == nil, status.get() == 127, !framed.get(), !lock.withLock({ showed }) {
                    let why = lastErr.get() ?? "herdr could not start (exit 127)."
                    finalReason.set(why)
                    cont.yield(.failed(reason: why))
                }
                k.resume(returning: finalReason.get() != nil ? .final : .retry)
            }
            out.fileHandleForReading.readabilityHandler = { h in
                let d = h.availableData
                if d.isEmpty { h.readabilityHandler = nil; done(); return }
                outReader.feed(d)
            }
            // stderr ends once: at EOF, or a second after exit when a child of herdr still holds it open
            let errEnded = Locked(false)
            let endErr: @Sendable () -> Void = {
                guard errEnded.update({ let was = $0; $0 = true; return !was }) else { return }
                errReader.feed(Data([0x0A]))   // a last line without newline too
                done()
            }
            err.fileHandleForReading.readabilityHandler = { h in
                withExtendedLifetime(err) {}   // the pipe stays open until EOF, after the run too
                let d = h.availableData
                if d.isEmpty { h.readabilityHandler = nil; endErr(); return }
                errEnded.update { ended in if !ended { errReader.feed(d) } }
            }
            p.terminationHandler = { p in
                status.set(p.terminationStatus)
                DispatchQueue.global().asyncAfter(deadline: .now() + 1) {
                    // End the run with what came. The handler keeps reading, and drops, until EOF: closing the pipe
                    // would kill a child that still writes (SIGPIPE), and a queued handler would read a closed handle.
                    guard !errEnded.get() else { return }
                    endErr()
                }
                done()
            }
            do {
                lock.withLock { proc = p; stdin = inp.fileHandleForWriting }
                try p.run()
                if lock.withLock({ stopped }) && mode != .observe { /* release() may race the start; the process sees EOF */ }
            } catch {
                lock.withLock { proc = nil; stdin = nil }
                out.fileHandleForReading.readabilityHandler = nil
                err.fileHandleForReading.readabilityHandler = nil
                cont.yield(.failed(reason: "cannot start herdr: \(error.localizedDescription)"))
                finalReason.set("start failed")
                k.resume(returning: .final)
            }
        }
    }
}

final class Locked<T>: @unchecked Sendable {
    private let l = NSLock()
    private var v: T
    init(_ v: T) { self.v = v }
    func get() -> T { l.withLock { v } }
    func set(_ x: T) { l.withLock { v = x } }
    func update<R>(_ f: (inout T) -> R) -> R { l.withLock { f(&v) } }
}
