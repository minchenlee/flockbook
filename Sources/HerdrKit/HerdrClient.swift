import Foundation

public actor HerdrClient {
    public let session: HerdrSession
    private var seq = 0

    public init(session: HerdrSession) { self.session = session }

    // MARK: - Raw requests

    /// One request on its own short-lived connection. Returns the full response line.
    func rawRequest(method: String, params: [String: Any] = [:]) async throws -> Data {
        try Self.refuseHCIds(params)
        seq += 1
        let id = "desk:\(seq)"
        let path = session.socketPath
        let body = try JSONSerialization.data(withJSONObject: ["id": id, "method": method, "params": params])
        return try await Task.detached(priority: .userInitiated) {
            let sock = try LineSocket(path: path, timeout: 10)   // a stuck server must not hold this thread forever
            defer { sock.close() }
            try sock.writeLine(body)
            let line = try sock.readLine()
            try Self.throwIfError(line)
            return line
        }.value
    }

    /// An hc pane/tab/workspace id (`hc:` prefix) sent to the TUI server is a routing bug; refuse it before it leaves.
    static func refuseHCIds(_ params: [String: Any]) throws {
        for k in ["pane_id", "target_pane_id", "tab_id", "workspace_id"] {
            if let v = params[k] as? String, v.hasPrefix(HCMapper.panePrefix) {
                throw HerdrError.protocolError("\(k) \(v) belongs to herdr-core, not the TUI server")
            }
        }
    }

    static func throwIfError(_ line: Data) throws {
        guard let obj = try JSONSerialization.jsonObject(with: line) as? [String: Any] else {
            throw HerdrError.protocolError("response is not an object")
        }
        if let err = obj["error"] as? [String: Any] {
            throw HerdrError.server(code: err["code"] as? String ?? "error", message: err["message"] as? String ?? "")
        }
    }

    // MARK: - Read

    public func snapshot() async throws -> [Workspace] {
        try SnapshotParser.parse(try await rawRequest(method: "session.snapshot"))
    }

    // MARK: - Input

    /// Types `text` into the pane and presses Enter.
    public func send(text: String, to paneId: String) async throws {
        _ = try await rawRequest(method: "pane.send_input", params: ["pane_id": paneId, "text": text, "keys": ["Enter"]])
    }

    /// Text through the same call as `send(text:to:)`, without Enter: a newline stays inside the agent's input.
    public func type(text: String, to paneId: String) async throws {
        _ = try await rawRequest(method: "pane.send_input", params: ["pane_id": paneId, "text": text, "keys": [String]()])
    }

    /// Raw bytes (control keys, escape sequences) via the terminal text channel.
    public func sendKeys(_ bytes: [UInt8], to paneId: String) async throws {
        let text = String(decoding: bytes, as: UTF8.self)
        _ = try await rawRequest(method: "pane.send_text", params: ["pane_id": paneId, "text": text])
    }

    /// Named keys such as "Enter", "Down", "Escape", "C-c".
    public func sendNamedKeys(_ keys: [String], to paneId: String) async throws {
        _ = try await rawRequest(method: "pane.send_keys", params: ["pane_id": paneId, "keys": keys])
    }

    // MARK: - Layout (mutating)

    // MARK: - Workspaces

    /// Creates a workspace with one tab and one shell pane (herdr always makes both). Returns the new workspace id.
    /// `label` nil: herdr names it (number or cwd). `cwd` nil: herdr's default directory. `focus` selects it in the herdr TUI.
    @discardableResult
    public func createWorkspace(label: String? = nil, cwd: String? = nil, focus: Bool = false) async throws -> String {
        var p: [String: Any] = ["focus": focus]
        if let label, !label.isEmpty { p["label"] = label }
        if let cwd, !cwd.isEmpty { p["cwd"] = cwd }
        let data = try await rawRequest(method: "workspace.create", params: p)
        guard let o = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let r = o["result"] as? [String: Any], let w = r["workspace"] as? [String: Any],
              let id = w["workspace_id"] as? String else { throw HerdrError.protocolError("workspace.create: no workspace_id") }
        return id
    }

    public func renameWorkspace(id: String, to label: String) async throws {
        _ = try await rawRequest(method: "workspace.rename", params: ["workspace_id": id, "label": label])
    }

    /// Closes the workspace with all its tabs, panes and their processes (agents are terminated). Fails when it does not exist.
    public func closeWorkspace(id: String) async throws {
        _ = try await rawRequest(method: "workspace.close", params: ["workspace_id": id])
    }

    /// Selects the workspace in herdr (the herdr TUI follows; the GUI keeps its own selection).
    public func focusWorkspace(id: String) async throws {
        _ = try await rawRequest(method: "workspace.focus", params: ["workspace_id": id])
    }

    public func createTab(workspaceId: String, label: String? = nil, cwd: String? = nil) async throws {
        var p: [String: Any] = ["workspace_id": workspaceId, "focus": true]
        if let label { p["label"] = label }
        if let cwd { p["cwd"] = cwd }
        _ = try await rawRequest(method: "tab.create", params: p)
    }

    public func closeTab(_ tabId: String) async throws {
        _ = try await rawRequest(method: "tab.close", params: ["tab_id": tabId])
    }

    /// Moves the tab so that it ends at `toIndex` (0-based, clamped) among the tabs of its workspace.
    /// herdr's `tab.move` takes `tab_id` and `insert_index`, where `insert_index` is a slot in the list **before** the tab is
    /// removed (moving tab 0 to slot 2 gives [1, 0, 2]). This method turns a final index into that slot.
    public func moveTab(_ tabId: String, toIndex: Int) async throws {
        guard let ws = tabId.split(separator: ":").first.map(String.init) else { throw HerdrError.protocolError("bad tab id \(tabId)") }
        let data = try await rawRequest(method: "tab.list", params: ["workspace_id": ws])
        guard let o = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let r = o["result"] as? [String: Any], let tabs = r["tabs"] as? [[String: Any]] else { throw HerdrError.protocolError("tab.list: bad response") }
        let ids = tabs.compactMap { $0["tab_id"] as? String }
        guard let from = ids.firstIndex(of: tabId) else { throw HerdrError.server(code: "tab_not_found", message: "tab \(tabId) not found") }
        let to = min(max(toIndex, 0), ids.count - 1)
        if to == from { return }
        _ = try await rawRequest(method: "tab.move", params: ["tab_id": tabId, "insert_index": to > from ? to + 1 : to])
    }

    public func renameTab(_ tabId: String, to label: String) async throws {
        _ = try await rawRequest(method: "tab.rename", params: ["tab_id": tabId, "label": label])
    }

    public func split(paneId: String, direction: SplitDirection) async throws {
        _ = try await rawRequest(method: "pane.split", params: ["target_pane_id": paneId, "direction": direction.rawValue, "focus": true])
    }

    // MARK: - Pane menu

    /// Names a pane. `nil` or an empty string clears the name (the label shows up as `Pane.label` in the next snapshot).
    public func renamePane(paneId: String, to label: String?) async throws {
        let l = label.flatMap { $0.isEmpty ? nil : $0 }
        _ = try await rawRequest(method: "pane.rename", params: ["pane_id": paneId, "label": l as Any? ?? NSNull()])
    }

    /// Closes the pane (and its process). Fails when the pane does not exist.
    public func closePane(paneId: String) async throws {
        _ = try await rawRequest(method: "pane.close", params: ["pane_id": paneId])
    }

    public enum RightClickTarget: String, Sendable { case herdr, pane }

    /// herdr's "Send right-clicks to pane" (`pane.input.set`). `.pane`: a right click in the pane goes to the program;
    /// `.herdr`: herdr shows its own menu. This is **write-only**: no API reports the current value (not in `pane.get`, the
    /// snapshot or the layout), so the app keeps the state itself. It is a per-pane setting inside herdr that the herdr TUI
    /// obeys; it does not change what `TerminalSession.rightClick` sends. A GUI that forwards right clicks itself only needs
    /// its own flag, and may also set this so the TUI behaves the same.
    public func setRightClickRouting(paneId: String, to target: RightClickTarget) async throws {
        _ = try await rawRequest(method: "pane.input.set", params: ["pane_id": paneId, "right_click": target.rawValue])
    }

    // MARK: - Events

    /// `pane.agent_status_changed` is left out on purpose: herdr requires a `pane_id` for it and rejects the whole
    /// subscription otherwise. Status changes are caught by `pane.updated` and the store's periodic refresh.
    static let eventTypes = [
        "workspace.created", "workspace.updated", "workspace.renamed", "workspace.moved", "workspace.reordered",
        "tab.created", "tab.renamed", "tab.moved", "tab.focused",
        "pane.created", "pane.moved", "pane.updated", "pane.closed", "pane.agent_detected",
        "workspace.closed", "tab.closed", "layout.updated",
    ]

    /// Yields once per event. Finishes (with an error on failure) when the connection drops.
    nonisolated public func events() -> AsyncThrowingStream<Void, Error> {
        let path = session.socketPath
        return AsyncThrowingStream { cont in
            let task = Task.detached {
                do {
                    let sock = try LineSocket(path: path)
                    cont.onTermination = { _ in sock.close() }
                    let subs = Self.eventTypes.map { ["type": $0] }
                    let body = try JSONSerialization.data(withJSONObject: [
                        "id": "desk:sub", "method": "events.subscribe", "params": ["subscriptions": subs],
                    ])
                    try sock.writeLine(body)
                    let ack = try sock.readLine()
                    try Self.throwIfError(ack)
                    cont.yield(())
                    while !Task.isCancelled {
                        let line = try sock.readLine()
                        if let o = try? JSONSerialization.jsonObject(with: line) as? [String: Any],
                           (o["type"] as? String) == "events_lost" || (o["event"] != nil) || (o["type"] != nil) {
                            cont.yield(())
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

    // MARK: - Terminal observe (read-only)

    /// A long-lived control (or observe) channel for one terminal: input, resize and frames over one process.
    /// Not started yet; call `start()` on the result. See `TerminalSession`.
    nonisolated public func terminalSession(terminalId: String, mode: TerminalSession.Mode = .control(takeover: false),
                                            cols: Int? = nil, rows: Int? = nil, launcher: [String]? = nil) -> TerminalSession {
        TerminalSession(terminalId: terminalId, session: session, mode: mode, cols: cols, rows: rows, launcher: launcher)
    }

    /// One-way stream without reconnect. `TerminalSession(mode: .observe)` is the same with reconnect and size.
    /// A remote session runs the host's herdr (`remoteCommand`, checks pass a fake); no herdr there ends it with an error.
    nonisolated public func observe(terminalId: String, remoteCommand: TerminalSession.RemoteCommand? = nil) -> AsyncThrowingStream<TerminalFrame, Error> {
        let prefix = session.cliPrefix
        let isNamed = session.name != nil
        let name = session.name, host = session.remote
        return AsyncThrowingStream { cont in
            if let why = TerminalSession.idProblem(terminalId, host: host) { cont.finish(throwing: HerdrError.protocolError(why)); return }
            let words = TerminalSession.arguments(terminalId: terminalId, mode: .observe, cols: nil, rows: nil)
            let proc: Process
            let errText = Locked("")
            if let host {
                let command = remoteCommand.map { $0(name, words) } ?? RemoteHost.herdrCommand(session: name, words)
                do { proc = try host.process(command) } catch {
                    cont.finish(throwing: error); return
                }
                // stdin stays open (never the app's own stdin): ssh would send EOF, or read what is not its own
                proc.standardInput = Pipe()
                let err = Pipe()
                proc.standardError = err
                err.fileHandleForReading.readabilityHandler = { h in
                    let d = h.availableData
                    if d.isEmpty { h.readabilityHandler = nil; return }
                    errText.update { if $0.utf8.count < 8192 { $0 += String(decoding: d, as: UTF8.self) } }
                }
            } else {
                proc = Process()
                guard let herdr = ConnectionCheck.findHerdr() else { cont.finish(throwing: HerdrError.socket(ConnectionCheck.notFound)); return }
                proc.executableURL = URL(fileURLWithPath: herdr)
                proc.arguments = prefix + words
                var env = ProcessInfo.processInfo.environment
                if isNamed { for k in env.keys where k.hasPrefix("HERDR_") { env[k] = nil } }
                env["PATH"] = TerminalSession.childPath(environment: env, home: NSHomeDirectory())
                proc.environment = env
                proc.standardError = FileHandle.nullDevice
            }
            let out = Pipe()
            proc.standardOutput = out
            let framed = Locked(false)
            let reader = LineAccumulator { line in
                guard let o = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { return }
                switch o["type"] as? String {
                case "terminal.frame":
                    let b64 = o["bytes"] as? String ?? ""
                    framed.set(true)
                    cont.yield(TerminalFrame(
                        seq: o["seq"] as? Int ?? 0, full: o["full"] as? Bool ?? false,
                        width: o["width"] as? Int ?? 0, height: o["height"] as? Int ?? 0,
                        bytes: Data(base64Encoded: b64) ?? Data()))
                case "terminal.closed":
                    cont.finish()
                default: break
                }
            }
            out.fileHandleForReading.readabilityHandler = { h in
                let d = h.availableData
                // a host run that ends before any frame finishes in the termination handler, with the reason
                if d.isEmpty { h.readabilityHandler = nil; if host == nil || framed.get() { cont.finish() }; return }
                reader.feed(d)
            }
            proc.terminationHandler = { p in
                // a host run that ended before any frame: say why (stderr gets a moment to arrive)
                guard let host, !framed.get() else { cont.finish(); return }
                let status = p.terminationStatus
                DispatchQueue.global().asyncAfter(deadline: .now() + 0.3) {
                    // frames still in the pipe at exit: the stdout end finishes the stream
                    if framed.get() { cont.finish(); return }
                    if let problem = TerminalSession.remoteProblem(status: status, stderr: errText.get(), host: host.label) {
                        cont.finish(throwing: HerdrError.socket(problem.reason))
                    } else { cont.finish() }
                }
            }
            cont.onTermination = { _ in
                out.fileHandleForReading.readabilityHandler = nil
                if proc.isRunning { proc.terminate() }
            }
            do { try proc.run() } catch { cont.finish(throwing: error) }
        }
    }
}

/// Splits a byte stream into lines. Feed from one thread at a time (readabilityHandler is serial).
final class LineAccumulator: @unchecked Sendable {
    private var buf = Data()
    private let onLine: (Data) -> Void
    init(onLine: @escaping (Data) -> Void) { self.onLine = onLine }
    func feed(_ d: Data) {
        buf.append(d)
        while let i = buf.firstIndex(of: 0x0A) {
            onLine(buf.subdata(in: buf.startIndex..<i))
            buf.removeSubrange(buf.startIndex...i)
        }
    }
}
